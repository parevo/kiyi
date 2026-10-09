//! The app-facing API: saved connections, live pools and running queries.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::{ConnectionConfig, DbAuth, SslMode};
use crate::design::{self, TableAction, TableDesign, TableDetails};
use crate::dml::{self, BrowseRequest, ChangeSet};
use crate::drivers::{self, DbDriver};
use crate::error::{explain_connect_error, is_cancel_error, Error, ErrorInfo, Result};
use crate::secrets;
use crate::store::ConnectionStore;
use crate::types::{Cell, ColumnMeta, QueryEvent, SchemaSnapshot, Sink};

fn tunnel_account(connection_id: &str) -> String {
    format!("tunnel:{connection_id}")
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
/// Rows a SQL-editor run keeps across all its result sets. Beyond this the window would run
/// out of memory; the query is stopped on the server and the user is told to add a LIMIT or export.
pub const MAX_RESULT_ROWS: usize = 100_000;
/// After asking the server to cancel, give it this long before dropping the task.
const CANCEL_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestStep {
    pub label: String,
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub ok: bool,
    pub steps: Vec<TestStep>,
    pub server_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectInfo {
    pub server_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Cell>>,
    /// The exact query that produced this page, for "open in editor".
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacePlan {
    pub statement: String,
    /// Rows the statement would change.
    pub rows: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptKind {
    /// Grid edits: one transaction, and every statement must touch exactly one row.
    Data,
    /// DDL: transactional on Postgres; MySQL commits each DDL statement implicitly.
    Schema,
    /// Bulk inserts (imports): one transaction, any number of rows per statement.
    Bulk,
}

struct Running {
    conn_id: String,
    session_id: Arc<Mutex<Option<u64>>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    abort: tokio::task::AbortHandle,
    sink: Arc<Sink<'static>>,
    started: Instant,
}

/// An open connection: the pool, and the tunnel it goes through (closed on drop).
struct Live {
    driver: Arc<dyn DbDriver>,
    _tunnel: Option<crate::tunnel::Tunnel>,
    /// Renews the IAM token before it expires; stops with the connection.
    _refresh: Option<AbortOnDrop>,
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// RDS IAM tokens last 15 minutes; new pool connections need a valid one.
const IAM_REFRESH: Duration = Duration::from_secs(10 * 60);
const RDS_CA_URL: &str = "https://truststore.pki.rds.amazonaws.com/global/global-bundle.pem";

pub struct Workspace {
    dir: PathBuf,
    store: Mutex<ConnectionStore>,
    ai: Mutex<crate::ai::AiStore>,
    known_hosts: crate::tunnel::KnownHosts,
    live: RwLock<HashMap<String, Live>>,
    running: Arc<Mutex<HashMap<String, Running>>>,
}

impl Workspace {
    pub fn new(config_dir: &Path) -> Result<Self> {
        Ok(Self {
            dir: config_dir.to_path_buf(),
            store: Mutex::new(ConnectionStore::load(config_dir.join("connections.json"))?),
            ai: Mutex::new(crate::ai::AiStore::load(config_dir.join("ai.json"))?),
            known_hosts: crate::tunnel::KnownHosts::load(config_dir.join("known_hosts.json")),
            live: RwLock::new(HashMap::new()),
            running: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn list(&self) -> Vec<ConnectionConfig> {
        self.store.lock().unwrap().list().to_vec()
    }

    /// `password` / `tunnel_secret`: `None` keeps the stored one, `Some("")` removes it,
    /// anything else replaces it. The tunnel secret is the SSH password or key passphrase.
    pub fn save(&self, mut config: ConnectionConfig, password: Option<String>, tunnel_secret: Option<String>) -> Result<ConnectionConfig> {
        if config.id.is_empty() {
            config.id = uuid::Uuid::new_v4().to_string();
        }
        match password.as_deref() {
            None => {}
            Some("") => secrets::delete_password(&config.id)?,
            Some(p) => secrets::set_password(&config.id, p)?,
        }
        match tunnel_secret.as_deref() {
            None => {}
            Some("") => secrets::set_secret(&tunnel_account(&config.id), None)?,
            Some(p) => secrets::set_secret(&tunnel_account(&config.id), Some(p))?,
        }
        self.store.lock().unwrap().upsert(config.clone())?;
        Ok(config)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.disconnect(id).await;
        secrets::delete_password(id)?;
        secrets::set_secret(&tunnel_account(id), None)?;
        self.store.lock().unwrap().remove(id)
    }

    fn config(&self, id: &str) -> Result<ConnectionConfig> {
        self.store.lock().unwrap().get(id).cloned().ok_or_else(|| Error::UnknownConnection(id.into()))
    }

    fn driver(&self, id: &str) -> Result<Arc<dyn DbDriver>> {
        self.live.read().unwrap().get(id).map(|l| l.driver.clone()).ok_or(Error::NotConnected)
    }

    async fn open_driver(config: &ConnectionConfig, password: Option<&str>) -> Result<Arc<dyn DbDriver>> {
        tokio::time::timeout(CONNECT_TIMEOUT, drivers::open(config, password))
            .await
            .map_err(|_| Error::Timeout("connecting"))?
    }

    /// Opens the tunnel (if any) and returns the config the driver should actually use.
    async fn open_tunnel(&self, config: &ConnectionConfig, secret: Option<&str>) -> Result<(ConnectionConfig, Option<crate::tunnel::Tunnel>)> {
        let Some(tunnel) = &config.tunnel else { return Ok((config.clone(), None)) };
        let t = crate::tunnel::open(tunnel, secret, &config.host, config.port, &self.known_hosts).await?;
        let mut local = config.clone();
        local.host = "127.0.0.1".into();
        local.port = t.local_port;
        // Through a tunnel the client talks to 127.0.0.1, which no certificate names. Still insist
        // the certificate chains to a trusted CA; the tunnel itself pins where it leads.
        if local.ssl_mode == SslMode::VerifyFull {
            local.ssl_mode = SslMode::VerifyCa;
        }
        if matches!(tunnel, crate::config::TunnelConfig::CloudSql { .. }) {
            // The proxy already encrypts and authenticates; the database sees a plain local connection.
            local.ssl_mode = SslMode::Disable;
        }
        Ok((local, Some(t)))
    }

    /// Fills in what a connection needs beyond its saved settings: an IAM token as the password,
    /// and the Amazon RDS certificate bundle when verifying an RDS server. Returns notes for the test report.
    async fn prepare(&self, config: &ConnectionConfig, password: Option<String>) -> Result<(ConnectionConfig, Option<String>, Vec<String>)> {
        let mut config = config.clone();
        let mut notes = Vec::new();
        let password = match &config.auth {
            DbAuth::Password => password,
            DbAuth::AwsIam { region, profile } => {
                // RDS only accepts IAM tokens over SSL.
                if matches!(config.ssl_mode, SslMode::Disable | SslMode::Prefer) {
                    config.ssl_mode = SslMode::Require;
                }
                let token = crate::tunnel::rds_auth_token(&config.host, config.port, &config.user, region.as_deref(), profile.as_deref()).await?;
                notes.push("Got an IAM sign-in token from the AWS CLI".to_string());
                Some(token)
            }
        };
        let verifying = matches!(config.ssl_mode, SslMode::VerifyCa | SslMode::VerifyFull);
        let no_cert = config.ssl_root_cert.as_deref().is_none_or(|c| c.trim().is_empty());
        if verifying && no_cert && crate::tunnel::rds_region(&config.host).is_some() {
            config.ssl_root_cert = Some(self.rds_ca_bundle().await?.to_string_lossy().into_owned());
            notes.push("Using the Amazon RDS certificate bundle".to_string());
        }
        Ok((config, password, notes))
    }

    /// Amazon's RDS CA bundle, downloaded once and kept next to the settings.
    async fn rds_ca_bundle(&self) -> Result<PathBuf> {
        let path = self.dir.join("rds-global-bundle.pem");
        if path.is_file() {
            return Ok(path);
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let fail = |e: &dyn std::fmt::Display| Error::Invalid(format!("Couldn't download the Amazon RDS certificate bundle ({e}). Choose it as the CA certificate instead: {RDS_CA_URL}"));
        let response = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| fail(&e))?.get(RDS_CA_URL).send().await.map_err(|e| fail(&e))?;
        let pem = response.error_for_status().map_err(|e| fail(&e))?.bytes().await.map_err(|e| fail(&e))?;
        if !pem.starts_with(b"-----BEGIN CERTIFICATE-----") {
            return Err(fail(&"unexpected content"));
        }
        std::fs::create_dir_all(&self.dir)?;
        let tmp = path.with_extension("pem.tmp");
        std::fs::write(&tmp, &pem)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// Lets the user accept an SSH server's new identity after it legitimately changed.
    pub fn forget_host_key(&self, host: &str, port: u16) {
        self.known_hosts.forget(host, port);
    }

    /// Tries a connection without saving it. Secrets left as `None` fall back to the keychain.
    pub async fn test(&self, config: ConnectionConfig, password: Option<String>, tunnel_secret: Option<String>) -> TestReport {
        let password = match password {
            Some(p) => Some(p),
            None if !config.id.is_empty() => secrets::get_password(&config.id).ok().flatten(),
            None => None,
        };
        let tunnel_secret = match tunnel_secret {
            Some(p) => Some(p),
            None if !config.id.is_empty() => secrets::get_secret(&tunnel_account(&config.id)).ok().flatten(),
            None => None,
        };
        let target = match config.kind {
            crate::config::DbKind::Sqlite => config.database.clone().unwrap_or_default(),
            _ if config.socket_path().is_some() => format!("the socket in {}", config.host),
            _ => match &config.tunnel {
                Some(crate::config::TunnelConfig::CloudSql { instance }) => instance.clone(),
                Some(crate::config::TunnelConfig::Kubernetes { target, .. }) => format!("{target} port {}", config.port),
                _ => format!("{}:{}", config.host, config.port),
            },
        };
        let mut steps = Vec::new();

        let (config, password) = match self.prepare(&config, password).await {
            Ok((config, password, notes)) => {
                steps.extend(notes.into_iter().map(|label| TestStep { label, ok: true, detail: None }));
                (config, password)
            }
            Err(e) => {
                steps.push(TestStep { label: "Couldn't prepare the sign-in".into(), ok: false, detail: Some(e.to_string()) });
                return TestReport { ok: false, steps, server_version: None };
            }
        };
        let (config, _tunnel) = match self.open_tunnel(&config, tunnel_secret.as_deref()).await {
            Ok((local, tunnel)) => {
                for step in tunnel.as_ref().map(|t| t.steps.clone()).unwrap_or_default() {
                    steps.push(TestStep { label: step, ok: true, detail: None });
                }
                (local, tunnel)
            }
            Err(e) => {
                steps.push(TestStep { label: "Couldn't open the tunnel".into(), ok: false, detail: Some(e.to_string()) });
                return TestReport { ok: false, steps, server_version: None };
            }
        };

        let driver = match Self::open_driver(&config, password.as_deref()).await {
            Ok(d) => {
                steps.push(TestStep { label: format!("Connected to {target}"), ok: true, detail: None });
                d
            }
            Err(e) => {
                steps.push(TestStep { label: format!("Could not connect to {target}"), ok: false, detail: Some(explain_connect_error(&e)) });
                return TestReport { ok: false, steps, server_version: None };
            }
        };
        let report = match driver.server_version().await {
            Ok(v) => {
                steps.push(TestStep { label: "Ran a test query".into(), ok: true, detail: Some(v.clone()) });
                TestReport { ok: true, steps, server_version: Some(v) }
            }
            Err(e) => {
                steps.push(TestStep { label: "Could not run a test query".into(), ok: false, detail: Some(explain_connect_error(&e)) });
                TestReport { ok: false, steps, server_version: None }
            }
        };
        driver.close().await;
        report
    }

    pub async fn connect(&self, id: &str) -> Result<ConnectInfo> {
        if let Ok(driver) = self.driver(id) {
            return Ok(ConnectInfo { server_version: driver.server_version().await? });
        }
        let saved = self.config(id)?;
        let password = secrets::get_password(id)?;
        let tunnel_secret = secrets::get_secret(&tunnel_account(id))?;
        let (config, password, _) = self.prepare(&saved, password).await?;
        let (config, tunnel) = self.open_tunnel(&config, tunnel_secret.as_deref()).await?;
        let driver = Self::open_driver(&config, password.as_deref()).await.map_err(|e| Error::Invalid(explain_connect_error(&e)))?;
        let server_version = driver.server_version().await?;
        let refresh = match &saved.auth {
            DbAuth::Password => None,
            DbAuth::AwsIam { region, profile } => {
                let (driver, region, profile) = (Arc::downgrade(&driver), region.clone(), profile.clone());
                Some(AbortOnDrop(tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(IAM_REFRESH).await;
                        let Some(driver) = driver.upgrade() else { break };
                        match crate::tunnel::rds_auth_token(&saved.host, saved.port, &saved.user, region.as_deref(), profile.as_deref()).await {
                            Ok(token) => driver.set_password(&token),
                            Err(e) => tracing::warn!("couldn't renew the IAM token: {e}"),
                        }
                    }
                })))
            }
        };
        self.live.write().unwrap().insert(id.to_string(), Live { driver, _tunnel: tunnel, _refresh: refresh });
        Ok(ConnectInfo { server_version })
    }

    pub async fn disconnect(&self, id: &str) {
        let running: Vec<String> = {
            let running = self.running.lock().unwrap();
            running.iter().filter(|(_, r)| r.conn_id == id).map(|(q, _)| q.clone()).collect()
        };
        for query_id in running {
            self.cancel(&query_id).await;
        }
        let live = self.live.write().unwrap().remove(id);
        if let Some(live) = live {
            live.driver.close().await;
        }
    }

    pub async fn schema(&self, id: &str) -> Result<SchemaSnapshot> {
        self.driver(id)?.schema().await
    }

    /// Starts `sql` in the background; events stream into `sink` and always end with `Done`.
    /// The estimated plan for one statement; nothing is executed.
    pub async fn explain(&self, id: &str, sql: &str) -> Result<crate::explain::PlanNode> {
        use crate::config::DbKind;
        use crate::explain;
        let driver = self.driver(id)?;
        let kind = driver.dialect().kind;
        let first = |rows: &[Vec<Cell>]| rows.first().and_then(|r| r.first().cloned().flatten()).unwrap_or_default();
        let plan = match kind {
            DbKind::Postgres => explain::from_postgres(&first(&driver.fetch(&explain::explain_sql(kind, sql)).await?.1)),
            DbKind::Sqlite => explain::from_sqlite(&driver.fetch(&explain::explain_sql(kind, sql)).await?.1),
            DbKind::Mysql => match driver.fetch(&explain::explain_sql(kind, sql)).await {
                Ok((_, rows)) => explain::from_mysql_tree(&first(&rows)),
                // MariaDB and old MySQL don't know FORMAT=TREE.
                Err(_) => {
                    let (cols, rows) = driver.fetch(&explain::explain_classic_sql(sql)).await?;
                    explain::from_mysql_classic(&cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), &rows)
                }
            },
        };
        plan.map_err(Error::Invalid)
    }

    /// What `sql` would do on this connection, so the UI can ask before changing production data.
    pub fn check_sql(&self, id: &str, sql: &str) -> Result<dml::ScriptCheck> {
        Ok(dml::check_script(self.driver(id)?.dialect(), sql))
    }

    fn is_read_only(&self, id: &str) -> bool {
        self.config(id).map(|c| c.read_only).unwrap_or(false)
    }

    /// Read-only is enforced here as well as in the database session, which a `SET` could undo.
    fn ensure_writable(&self, id: &str) -> Result<()> {
        if self.is_read_only(id) {
            return Err(Error::Invalid("This connection is read-only, so nothing was changed. Turn off Read-only in the connection settings to make changes.".into()));
        }
        Ok(())
    }

    pub fn run(&self, conn_id: &str, query_id: String, sql: String, sink: Arc<Sink<'static>>) -> Result<()> {
        let driver = self.driver(conn_id)?;
        if self.is_read_only(conn_id) && dml::check_script(driver.dialect(), &sql).writes {
            return Err(Error::Invalid(
                "This connection is read-only, so this SQL didn't run: it changes data, structure or session settings. Turn off Read-only in the connection settings to make changes.".into(),
            ));
        }
        let session_id = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let running = self.running.clone();

        // Hold the registry lock across spawn so the task can't finish (and try to
        // deregister itself) before it's registered.
        let mut registry = self.running.lock().unwrap();
        let task = {
            let (session_id, cancelled, query_id, sink) = (session_id.clone(), cancelled.clone(), query_id.clone(), sink.clone());
            let truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));
            tokio::spawn(async move {
                use std::sync::atomic::Ordering::SeqCst;
                let started = Instant::now();
                let on_session = {
                    let session_id = session_id.clone();
                    move |id: u64| *session_id.lock().unwrap() = Some(id)
                };
                let seen = std::sync::atomic::AtomicUsize::new(0);
                let capped = |event: QueryEvent| match event {
                    QueryEvent::Rows { mut rows } => {
                        if truncated.load(SeqCst) {
                            return;
                        }
                        let before = seen.fetch_add(rows.len(), SeqCst);
                        if before + rows.len() > MAX_RESULT_ROWS {
                            rows.truncate(MAX_RESULT_ROWS.saturating_sub(before));
                            truncated.store(true, SeqCst);
                            // Stop the server from producing rows nobody will see.
                            let (driver, session_id) = (driver.clone(), *session_id.lock().unwrap());
                            tokio::spawn(async move {
                                if let Some(id) = session_id {
                                    let _ = driver.cancel(id).await;
                                }
                            });
                        }
                        if !rows.is_empty() {
                            sink(QueryEvent::Rows { rows });
                        }
                    }
                    other => sink(other),
                };
                let result = driver.execute(&sql, &on_session, &capped).await;
                let was_cancelled = cancelled.load(SeqCst);
                let was_truncated = truncated.load(SeqCst);
                if let Err(e) = result {
                    if !((was_cancelled || was_truncated) && is_cancel_error(&e)) {
                        sink(QueryEvent::Error { error: ErrorInfo::from(&e) });
                    }
                }
                running.lock().unwrap().remove(&query_id);
                sink(QueryEvent::Done { elapsed_ms: started.elapsed().as_millis() as u64, cancelled: was_cancelled, truncated: was_truncated });
            })
        };
        registry.insert(
            query_id,
            Running { conn_id: conn_id.to_string(), session_id, cancelled, abort: task.abort_handle(), sink, started: Instant::now() },
        );
        Ok(())
    }

    /// Asks the server to stop the query; drops it client-side if the server doesn't comply in time.
    pub async fn cancel(&self, query_id: &str) {
        let (conn_id, session_id, abort) = {
            let running = self.running.lock().unwrap();
            let Some(r) = running.get(query_id) else { return };
            r.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            let session_id = *r.session_id.lock().unwrap();
            (r.conn_id.clone(), session_id, r.abort.clone())
        };
        if let (Some(session_id), Ok(driver)) = (session_id, self.driver(&conn_id)) {
            if let Err(e) = driver.cancel(session_id).await {
                tracing::warn!("server-side cancel failed: {e}");
            }
        }
        let running = self.running.clone();
        let query_id = query_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(CANCEL_GRACE).await;
            let stuck = running.lock().unwrap().remove(&query_id);
            if let Some(r) = stuck {
                abort.abort();
                (r.sink)(QueryEvent::Done { elapsed_ms: r.started.elapsed().as_millis() as u64, cancelled: true, truncated: false });
            }
        });
    }

    pub async fn table_details(&self, id: &str, schema: Option<&str>, table: &str) -> Result<TableDetails> {
        self.driver(id)?.table_details(schema, table).await
    }

    fn check_condition(driver: &Arc<dyn DbDriver>, req: &BrowseRequest) -> Result<()> {
        match req.raw_where.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(c) => dml::validate_condition(driver.dialect(), c).map_err(Error::Invalid),
            None => Ok(()),
        }
    }

    pub fn ai(&self) -> std::sync::MutexGuard<'_, crate::ai::AiStore> {
        self.ai.lock().unwrap()
    }

    /// Models offered by `provider`, using `key` from the form or the stored one.
    pub async fn ai_models(&self, provider: &crate::ai::AiProvider, key: Option<&str>) -> Result<Vec<String>> {
        let key = self.ai().key(provider, key)?;
        crate::ai::list_models(provider, key.as_deref()).await
    }

    pub async fn ai_filters(&self, id: &str, schema: Option<&str>, table: &str, prompt: &str, today: &str) -> Result<crate::ai::AiFilterResult> {
        let (provider, key) = {
            let ai = self.ai();
            let p = ai.active().ok_or_else(|| Error::Invalid("Choose an AI provider in Settings to use AI.".into()))?;
            let key = ai.key(&p, None)?;
            (p, key)
        };
        let driver = self.driver(id)?;
        let details = driver.table_details(schema, table).await?;
        crate::ai::filters_from_prompt(&provider, key.as_deref(), driver.dialect(), &details, prompt, today).await
    }

    pub async fn browse(&self, id: &str, req: &BrowseRequest) -> Result<Page> {
        let driver = self.driver(id)?;
        Self::check_condition(&driver, req)?;
        let sql = dml::browse_sql(driver.dialect(), req);
        let (columns, rows) = driver.fetch(&sql).await?;
        Ok(Page { columns, rows, sql })
    }

    pub async fn count(&self, id: &str, req: &BrowseRequest) -> Result<u64> {
        let driver = self.driver(id)?;
        Self::check_condition(&driver, req)?;
        let (_, rows) = driver.fetch(&dml::count_sql(driver.dialect(), req)).await?;
        Ok(rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    /// The UPDATE for a find-and-replace and how many rows it would change.
    pub async fn plan_replace(&self, id: &str, req: &BrowseRequest, column: &str, find: &str, replacement: &str) -> Result<ReplacePlan> {
        let driver = self.driver(id)?;
        Self::check_condition(&driver, req)?;
        let (statement, count_sql) = dml::plan_replace(driver.dialect(), req, column, find, replacement).map_err(Error::Invalid)?;
        let (_, rows) = driver.fetch(&count_sql).await?;
        let rows = rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0);
        Ok(ReplacePlan { statement, rows })
    }

    pub fn plan_changes(&self, id: &str, set: &ChangeSet) -> Result<Vec<String>> {
        Ok(dml::plan_changes(self.driver(id)?.dialect(), set))
    }

    /// `old: None` creates the table.
    pub fn plan_table(&self, id: &str, schema: Option<&str>, old: Option<&TableDesign>, new: &TableDesign) -> Result<Vec<String>> {
        let d = self.driver(id)?.dialect();
        match old {
            Some(old) => design::plan_alter(d, schema, old, new),
            None => design::plan_create(d, schema, new),
        }
        .map_err(Error::Invalid)
    }

    pub fn plan_action(&self, id: &str, schema: Option<&str>, table: &str, is_view: bool, action: &TableAction) -> Result<Vec<String>> {
        Ok(design::plan_action(self.driver(id)?.dialect(), schema, table, is_view, action))
    }

    pub async fn execute_script(&self, id: &str, statements: &[String], kind: ScriptKind) -> Result<Vec<u64>> {
        self.ensure_writable(id)?;
        let driver = self.driver(id)?;
        match kind {
            ScriptKind::Data => driver.execute_script(statements, true, true).await,
            ScriptKind::Schema => driver.execute_script(statements, !driver.dialect().is_mysql(), false).await,
            ScriptKind::Bulk => driver.execute_script(statements, true, false).await,
        }
    }

    pub async fn export(&self, id: &str, req: &BrowseRequest, format: crate::transfer::ExportFormat, path: &Path) -> Result<u64> {
        let driver = self.driver(id)?;
        Self::check_condition(&driver, req)?;
        crate::transfer::export(driver.as_ref(), &dml::export_sql(driver.dialect(), req), format, path).await
    }

    pub async fn import_csv(&self, id: &str, path: &Path, plan: &crate::transfer::ImportPlan) -> Result<u64> {
        self.ensure_writable(id)?;
        let driver = self.driver(id)?;
        crate::transfer::import(&*driver, path, plan).await
    }
}
