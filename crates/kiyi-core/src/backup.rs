//! Backing up and restoring a database. With the database's own tools installed (pg_dump/psql,
//! mysqldump/mysql) Kiyi uses them for a complete copy. Without them it writes its own SQL file
//! with the tables, their data, keys and indexes. SQLite backups are a consistent copy of the file.
//! Both report progress as they go and stop when asked; a stopped or failed backup leaves no file.

use std::future::Future;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::{ConnectionConfig, DbKind, SslMode};
use crate::design::{self, TableDesign};
use crate::drivers::DbDriver;
use crate::error::{is_missing_table, Error, Result};
use crate::types::{QueryEvent, Sink, ValueKind};

const HEADER: &str = "-- Kiyi backup";
/// Ends every statement in Kiyi's own files, so a restore can read them line by line.
const END: &str = "-- kiyi:end";
const INSERT_BATCH: usize = 200;
/// How often a running backup checks for a stop and reports how far it got.
const POLL: Duration = Duration::from_millis(200);
/// After asking the server to stop a read, give it this long before dropping the connection.
const CANCEL_GRACE: Duration = Duration::from_secs(3);

#[cfg(test)]
mod tests {
    #[test]
    fn removes_definers() {
        assert_eq!(super::remove_definer("/*!50013 DEFINER=`root`@`%` SQL SECURITY DEFINER */"), "/*!50013 SQL SECURITY DEFINER */");
        assert_eq!(super::remove_definer("CREATE DEFINER=`app`@`localhost` PROCEDURE p()"), "CREATE PROCEDURE p()");
        assert_eq!(super::remove_definer("CREATE DEFINER=CURRENT_USER TRIGGER t"), "CREATE TRIGGER t");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupMethod {
    /// pg_dump / mysqldump, or a copy of the SQLite file.
    Native,
    /// Kiyi's own SQL file.
    Kiyi,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReport {
    pub method: BackupMethod,
    /// The tool that did it, e.g. "pg_dump" or "Kiyi".
    pub tool: String,
    pub tables: usize,
    pub rows: Option<u64>,
    pub bytes: u64,
    /// What this kind of backup leaves out, if anything.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    /// Reading the table list and each table's structure.
    #[default]
    Preparing,
    /// Copying data, or an outside tool is at work.
    Copying,
}

/// How far a backup or restore has got, for a progress bar.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: Stage,
    /// The table being read; empty during a restore or when an outside tool does the work.
    pub table: String,
    pub tables_done: usize,
    pub tables: usize,
    /// Rows backed up so far (statements run, for a restore), and roughly how many to expect.
    pub rows: u64,
    pub expected_rows: Option<u64>,
    /// Bytes of the backup written so far, or of the backup file read during a restore.
    pub bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Where a running backup or restore reports to, and the flag that stops it.
pub struct Job<'a> {
    pub progress: &'a (dyn Fn(Progress) + Send + Sync),
    pub cancel: &'a AtomicBool,
}

impl Job<'_> {
    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn check(&self) -> Result<()> {
        if self.stopped() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

/// Progress sent a few times a second rather than for every batch of rows.
struct Reporter<'a> {
    job: &'a Job<'a>,
    state: Mutex<(Progress, Option<Instant>)>,
}

impl<'a> Reporter<'a> {
    fn new(job: &'a Job<'a>) -> Self {
        Self { job, state: Mutex::new((Progress::default(), None)) }
    }

    fn update(&self, force: bool, change: impl FnOnce(&mut Progress)) {
        let mut state = self.state.lock().unwrap();
        change(&mut state.0);
        if force || state.1.is_none_or(|t| t.elapsed() >= POLL) {
            state.1 = Some(Instant::now());
            (self.job.progress)(state.0.clone());
        }
    }
}

/// A file written beside its final path and moved into place only when complete, so a stopped
/// or failed backup neither leaves half a file behind nor spoils one that was already there.
struct Partial {
    tmp: PathBuf,
    path: PathBuf,
}

impl Partial {
    fn new(path: &Path) -> Self {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".partial");
        let tmp = PathBuf::from(tmp);
        let _ = std::fs::remove_file(&tmp);
        Self { tmp, path: path.to_path_buf() }
    }

    fn size(&self) -> u64 {
        std::fs::metadata(&self.tmp).map(|m| m.len()).unwrap_or(0)
    }

    /// Moves the finished file into place and returns its size.
    fn finish(self) -> Result<u64> {
        std::fs::rename(&self.tmp, &self.path)?;
        Ok(std::fs::metadata(&self.path)?.len())
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.tmp);
    }
}

/// Awaits `work`, calling `tick` a few times a second. Once the job is stopped `work` is dropped,
/// which ends an outside tool (they're started with `kill_on_drop`).
async fn watched<T>(work: impl Future<Output = Result<T>>, job: &Job<'_>, tick: impl Fn()) -> Result<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return out,
            _ = tokio::time::sleep(POLL) => {
                job.check()?;
                tick();
            }
        }
    }
}

/// Runs `sql`, ending it on the server as soon as the job is stopped.
async fn execute(driver: &dyn DbDriver, sql: &str, sink: &Sink<'_>, job: &Job<'_>) -> Result<()> {
    let session = Mutex::new(None);
    let on_session = |id: u64| *session.lock().unwrap() = Some(id);
    let run = driver.execute(sql, &on_session, sink);
    tokio::pin!(run);
    loop {
        tokio::select! {
            out = &mut run => return out,
            _ = tokio::time::sleep(POLL) => if job.stopped() { break },
        }
    }
    let id = *session.lock().unwrap();
    if let Some(id) = id {
        let _ = driver.cancel(id).await;
    }
    // Dropping the call closes its connection if the server is slow to stop.
    let _ = tokio::time::timeout(CANCEL_GRACE, &mut run).await;
    Err(Error::Cancelled)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub tool: String,
    pub statements: Option<u64>,
}

/// Where and how to reach the database right now (through its tunnel, if any).
pub struct Target {
    pub config: ConnectionConfig,
    pub password: Option<String>,
}

fn ssl_mode(m: SslMode) -> &'static str {
    match m {
        SslMode::Disable => "disable",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    }
}

fn database(t: &Target) -> String {
    t.config.database.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| if t.config.kind == DbKind::Postgres { "postgres".into() } else { String::new() })
}

fn pg_args(t: &Target) -> Vec<String> {
    vec!["--host".into(), t.config.host.clone(), "--port".into(), t.config.port.to_string(), "--username".into(), t.config.user.clone(), "--dbname".into(), database(t), "--no-password".into()]
}

fn pg_env(t: &Target) -> Vec<(String, String)> {
    let mut env = vec![("PGSSLMODE".into(), ssl_mode(t.config.ssl_mode).into()), ("PGCONNECT_TIMEOUT".into(), "15".into())];
    if let Some(p) = &t.password {
        env.push(("PGPASSWORD".into(), p.clone()));
    }
    if let Some(cert) = t.config.ssl_root_cert.as_deref().filter(|c| !c.is_empty()) {
        env.push(("PGSSLROOTCERT".into(), cert.into()));
    }
    env
}

fn mysql_args(t: &Target, tool: &Path) -> Vec<String> {
    let mut args = match t.config.socket_path() {
        Some(sock) => vec![format!("--socket={sock}")],
        None => vec![format!("--host={}", t.config.host), format!("--port={}", t.config.port)],
    };
    args.push(format!("--user={}", t.config.user));
    // MariaDB's clients spell SSL options differently; leave them at their defaults there.
    let mariadb = std::process::Command::new(tool).arg("--version").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("MariaDB"));
    if !mariadb {
        let mode = match t.config.ssl_mode {
            SslMode::Disable => "DISABLED",
            SslMode::Prefer => "PREFERRED",
            SslMode::Require => "REQUIRED",
            SslMode::VerifyCa => "VERIFY_CA",
            SslMode::VerifyFull => "VERIFY_IDENTITY",
        };
        args.push(format!("--ssl-mode={mode}"));
        if let Some(cert) = t.config.ssl_root_cert.as_deref().filter(|c| !c.is_empty()) {
            args.push(format!("--ssl-ca={cert}"));
        }
    }
    args
}

fn mysql_env(t: &Target) -> Vec<(String, String)> {
    t.password.iter().map(|p| ("MYSQL_PWD".to_string(), p.clone())).collect()
}

async fn run_tool(tool: &Path, args: &[String], env: &[(String, String)], stdin: Option<&Path>, job: &Job<'_>, tick: impl Fn()) -> Result<()> {
    let name = tool.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut cmd = tokio::process::Command::new(tool);
    cmd.args(args).envs(env.iter().cloned()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    cmd.stdin(match stdin {
        Some(path) => std::process::Stdio::from(std::fs::File::open(path)?),
        None => std::process::Stdio::null(),
    });
    let child = cmd.spawn().map_err(|e| Error::Invalid(format!("Couldn't start {name}: {e}")))?;
    let out = watched(async { Ok(child.wait_with_output().await?) }, job, tick).await?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    Err(Error::Invalid(format!("{name} failed: {}", if line.is_empty() { out.status.to_string() } else { line.to_string() })))
}

/// Backs up with the database's own tool when it's installed, else with Kiyi's SQL writer.
pub async fn backup(driver: &dyn DbDriver, target: &Target, path: &Path, prefer_kiyi: bool, job: &Job<'_>) -> Result<BackupReport> {
    match backup_to(driver, target, path, prefer_kiyi, job).await {
        Err(Error::Cancelled) => Err(Error::Invalid("Backup stopped. No file was written.".into())),
        other => other,
    }
}

async fn backup_to(driver: &dyn DbDriver, target: &Target, path: &Path, prefer_kiyi: bool, job: &Job<'_>) -> Result<BackupReport> {
    let kind = driver.dialect().kind;
    let report = Reporter::new(job);
    report.update(true, |_| {});
    let file = Partial::new(path);
    let written = || report.update(false, |p| p.bytes = file.size());
    if kind == DbKind::Sqlite {
        // A consistent copy of the whole file, even while it's in use. SQLite can't stop it
        // halfway, so a stop takes effect once the copy is done.
        let tables = driver.schema().await?.schemas.iter().map(|s| s.tables.len()).sum();
        report.update(true, |p| {
            p.stage = Stage::Copying;
            p.tables = tables;
        });
        let sql = [format!("VACUUM INTO {}", driver.dialect().string(&file.tmp.to_string_lossy()))];
        watched(driver.execute_script(&sql, false, false), &Job { progress: job.progress, cancel: &AtomicBool::new(false) }, written).await?;
        job.check()?;
        return Ok(BackupReport { method: BackupMethod::Native, tool: "SQLite".into(), tables, rows: None, bytes: file.finish()?, note: None });
    }
    // SQL Server has no dump tool that writes a runnable script; Kiyi's own format is used there.
    let tool = if prefer_kiyi || kind == DbKind::Sqlserver { None } else { crate::tunnel::find_tool(if kind == DbKind::Postgres { "pg_dump" } else { "mysqldump" }) };
    let Some(tool) = tool else {
        return kiyi_backup(driver, file, &report, job).await;
    };
    let snapshot = driver.schema().await?;
    // MySQL lists every database the user can see; count the connected one.
    let tables = snapshot.schemas.iter().filter(|s| kind != DbKind::Mysql || Some(&s.name) == snapshot.default_schema.as_ref()).map(|s| s.tables.len()).sum();
    report.update(true, |p| {
        p.stage = Stage::Copying;
        p.tables = tables;
    });
    if kind == DbKind::Postgres {
        let mut args = vec!["--no-owner".into(), "--no-privileges".into(), "--format=plain".into(), format!("--file={}", file.tmp.display())];
        args.extend(pg_args(target));
        run_tool(&tool, &args, &pg_env(target), None, job, written).await?;
    } else {
        let mut args = mysql_args(target, &tool);
        args.extend(["--single-transaction".into(), "--routines".into(), "--triggers".into(), "--no-tablespaces".into(), format!("--result-file={}", file.tmp.display()), database(target)]);
        run_tool(&tool, &args, &mysql_env(target), None, job, written).await?;
        // Portable dumps: without DEFINER clauses, anyone with rights on the target can restore.
        strip_definers(&file.tmp)?;
    }
    job.check()?;
    let name = tool.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(BackupReport { method: BackupMethod::Native, tool: name, tables, rows: None, bytes: file.finish()?, note: None })
}

/// Rows waiting to be written as one INSERT, and which result columns go into it.
#[derive(Default)]
struct Pending {
    values: Vec<String>,
    columns: Vec<(usize, ValueKind)>,
    names: String,
}

/// Tables in an order where every table comes after the ones it references (cycles keep their order).
fn creation_order(designs: &[(Option<String>, TableDesign)]) -> Vec<usize> {
    let mut placed: Vec<usize> = Vec::new();
    let mut remaining: Vec<usize> = (0..designs.len()).collect();
    while !remaining.is_empty() {
        let ready: Vec<usize> = remaining
            .iter()
            .copied()
            .filter(|&i| {
                designs[i].1.foreign_keys.iter().all(|f| f.ref_table == designs[i].1.name || placed.iter().any(|&p| designs[p].1.name == f.ref_table) || !designs.iter().any(|(_, d)| d.name == f.ref_table))
            })
            .collect();
        let next = if ready.is_empty() { vec![remaining[0]] } else { ready };
        remaining.retain(|i| !next.contains(i));
        placed.extend(next);
    }
    placed
}

/// Kiyi's own backup: types, tables (without foreign keys), data, sequences, then foreign keys.
async fn kiyi_backup(driver: &dyn DbDriver, file: Partial, report: &Reporter<'_>, job: &Job<'_>) -> Result<BackupReport> {
    let d = driver.dialect();
    let snapshot = driver.schema().await?;
    let default_schema = snapshot.default_schema.clone();
    let mut designs: Vec<(Option<String>, TableDesign)> = Vec::new();
    let mut vanished: Vec<String> = Vec::new();
    // MySQL lists every database the user can see; back up the connected one.
    let tables: Vec<_> = snapshot
        .schemas
        .iter()
        .filter(|sc| !d.is_mysql() || Some(&sc.name) == default_schema.as_ref())
        .flat_map(|sc| sc.tables.iter().filter(|t| matches!(t.kind, crate::types::TableKind::Table)).map(move |t| (sc, t)))
        .collect();
    // The database's own row counts, roughly; tables it has no figure for count as empty.
    let estimates: Vec<u64> = tables.iter().filter_map(|(_, t)| t.row_estimate).map(|n| n.max(0) as u64).collect();
    let expected_rows = (!estimates.is_empty()).then(|| estimates.iter().sum());
    report.update(true, |p| {
        p.tables = tables.len();
        p.expected_rows = expected_rows;
    });
    for (i, (sc, t)) in tables.iter().enumerate() {
        job.check()?;
        report.update(false, |p| {
            p.table = t.name.clone();
            p.tables_done = i;
        });
        let schema = if d.is_sqlite() { None } else { Some(sc.name.clone()) };
        let mut des = match driver.table_details(schema.as_deref(), &t.name).await {
            Ok(details) => details.design,
            // Dropped by someone else since the table list was read: nothing to back up.
            Err(e) if is_missing_table(&e) => {
                vanished.push(t.name.clone());
                continue;
            }
            Err(e) => return Err(e),
        };
        if d.is_mysql() {
            // In MySQL the "schema" is the database itself: name nothing after it, so the
            // backup restores into whichever database it's loaded into.
            for f in des.foreign_keys.iter_mut().filter(|f| f.ref_schema.as_deref() == Some(sc.name.as_str())) {
                f.ref_schema = None;
            }
        }
        design::portable_serials(d, &mut des);
        if d.is_sqlserver() {
            // rowversion/timestamp values are set by the server and can't be inserted.
            for c in des.columns.iter_mut().filter(|c| matches!(c.data_type.to_ascii_lowercase().as_str(), "timestamp" | "rowversion")) {
                c.generated = true;
            }
        }
        designs.push((if d.is_mysql() { None } else { schema.clone() }, des));
    }
    let order = creation_order(&designs);
    // The writer, and how many bytes went through it.
    let out = Mutex::new((BufWriter::new(std::fs::File::create(&file.tmp)?), 0u64));
    let write = |text: &str| -> std::io::Result<()> {
        let mut out = out.lock().unwrap();
        out.0.write_all(text.as_bytes())?;
        out.1 += text.len() as u64;
        Ok(())
    };
    let written = || out.lock().unwrap().1;
    let statement = |sql: &str| -> std::io::Result<()> { write(&format!("{sql};\n{END}\n")) };
    let engine = match d.kind {
        DbKind::Postgres => "PostgreSQL",
        DbKind::Mysql => "MySQL",
        DbKind::Sqlite => "SQLite",
        DbKind::Sqlserver => "SQL Server",
    };
    write(&format!("{HEADER} of {} ({engine}), {} tables.\n-- Restore it into an empty database with Kiyi, or run it in any SQL client.\n\n", default_schema.as_deref().unwrap_or("the database"), designs.len()))?;
    if d.is_mysql() {
        // Data goes in before the foreign keys that check it.
        statement("SET FOREIGN_KEY_CHECKS = 0")?;
    }

    // SQL Server schemas other than dbo have to exist before their tables.
    if d.is_sqlserver() {
        let mut made: Vec<&str> = Vec::new();
        for (schema, _) in &designs {
            if let Some(sc) = schema.as_deref().filter(|s| *s != "dbo" && !made.contains(s)) {
                made.push(sc);
                statement(&format!("IF SCHEMA_ID({}) IS NULL EXEC({})", d.string(sc), d.string(&format!("CREATE SCHEMA {}", d.ident(sc)))))?;
            }
        }
    }

    // Postgres enum types the tables use.
    if d.kind == DbKind::Postgres {
        let mut made: Vec<String> = Vec::new();
        for (_, des) in &designs {
            for c in des.columns.iter().filter(|c| !c.enum_values.is_empty()) {
                if made.contains(&c.data_type) {
                    continue;
                }
                made.push(c.data_type.clone());
                let values: Vec<String> = c.enum_values.iter().map(|v| d.string(v)).collect();
                // The type name as Postgres prints it is already valid SQL (quoted if needed).
                statement(&format!("CREATE TYPE {} AS ENUM ({})", c.data_type, values.join(", ")))?;
            }
        }
    }
    for &i in &order {
        let (schema, des) = &designs[i];
        // Foreign keys are added after all data, except in SQLite which can't add them later.
        let mut bare = des.clone();
        if !d.is_sqlite() {
            bare.foreign_keys.clear();
        }
        for sql in design::plan_create(d, schema.as_deref(), &bare).map_err(Error::Invalid)? {
            statement(&sql)?;
        }
    }

    let mut total_rows = 0u64;
    report.update(true, |p| {
        p.stage = Stage::Copying;
        p.tables = designs.len();
        p.tables_done = 0;
    });
    for (n, &i) in order.iter().enumerate() {
        job.check()?;
        let (schema, des) = &designs[i];
        let table = d.table(schema.as_deref(), &des.name);
        report.update(true, |p| {
            p.table = des.name.clone();
            p.tables_done = n;
            p.bytes = written();
        });
        let generated: Vec<&str> = des.columns.iter().filter(|c| c.generated).map(|c| c.name.as_str()).collect();
        let identity = d.kind == DbKind::Postgres && des.columns.iter().any(|c| c.auto_increment);
        // SQL Server only takes explicit values for an identity column with IDENTITY_INSERT on.
        let identity_insert = d.is_sqlserver() && des.columns.iter().any(|c| c.auto_increment);
        if identity_insert {
            statement(&format!("SET IDENTITY_INSERT {table} ON"))?;
        }
        let pending = Mutex::new(Pending::default());
        let failure: Mutex<Option<std::io::Error>> = Mutex::new(None);
        let flush = |p: &mut Pending| {
            if p.values.is_empty() {
                return;
            }
            let sql = format!("INSERT INTO {table} ({}){} VALUES\n{}", p.names, if identity { " OVERRIDING SYSTEM VALUE" } else { "" }, p.values.join(",\n"));
            p.values.clear();
            if let Err(e) = statement(&sql) {
                *failure.lock().unwrap() = Some(e);
            }
        };
        let rows = Mutex::new(0u64);
        let sink = |event: QueryEvent| match event {
            // Stopping: the server is being asked to end the read, and the rest isn't needed.
            _ if job.stopped() => {}
            QueryEvent::Columns { columns } => {
                let mut p = pending.lock().unwrap();
                p.columns = columns.iter().enumerate().filter(|(_, c)| !generated.contains(&c.name.as_str())).map(|(i, c)| (i, c.kind)).collect();
                p.names = p.columns.iter().map(|(i, _)| d.ident(&columns[*i].name)).collect::<Vec<_>>().join(", ");
            }
            QueryEvent::Rows { rows: batch } => {
                let mut p = pending.lock().unwrap();
                for r in &batch {
                    let values: Vec<String> = p.columns.iter().map(|(i, k)| d.value(r[*i].as_deref(), *k == ValueKind::Binary)).collect();
                    p.values.push(format!("({})", values.join(", ")));
                    if p.values.len() >= INSERT_BATCH {
                        flush(&mut p);
                    }
                }
                drop(p);
                *rows.lock().unwrap() += batch.len() as u64;
                report.update(false, |p| {
                    p.rows += batch.len() as u64;
                    p.bytes = written();
                });
            }
            _ => {}
        };
        if let Err(e) = execute(driver, &format!("SELECT * FROM {table}"), &sink, job).await {
            if !is_missing_table(&e) {
                return Err(e);
            }
            vanished.push(des.name.clone());
        }
        flush(&mut pending.lock().unwrap());
        if let Some(e) = failure.into_inner().unwrap() {
            return Err(e.into());
        }
        total_rows += rows.into_inner().unwrap();
        if identity_insert {
            statement(&format!("SET IDENTITY_INSERT {table} OFF"))?;
        }
        if identity {
            for c in des.columns.iter().filter(|c| c.auto_increment) {
                let col = d.ident(&c.name);
                // Continue numbering after the restored rows.
                statement(&format!("SELECT setval(pg_get_serial_sequence({}, {}), COALESCE(MAX({col}), 1), MAX({col}) IS NOT NULL) FROM {table}", d.string(&table), d.string(&c.name)))?;
            }
        }
    }
    if !d.is_sqlite() {
        for &i in &order {
            let (schema, des) = &designs[i];
            for f in &des.foreign_keys {
                statement(&format!("ALTER TABLE {} ADD {}", d.table(schema.as_deref(), &des.name), design::fk_def(d, schema.as_deref(), f)))?;
            }
        }
    }
    if d.is_mysql() {
        statement("SET FOREIGN_KEY_CHECKS = 1")?;
    }
    job.check()?;
    out.into_inner().unwrap().0.flush()?;
    report.update(true, |p| p.tables_done = designs.len());
    Ok(BackupReport {
        method: BackupMethod::Kiyi,
        tool: "Kiyi".into(),
        tables: designs.len(),
        rows: Some(total_rows),
        bytes: file.finish()?,
        note: Some(format!(
            "Tables, data, keys and indexes. Views, functions, triggers and permissions need the database's own tools (pg_dump / mysqldump).{}",
            if vanished.is_empty() { String::new() } else { format!(" Skipped because they were deleted during the backup: {}.", vanished.join(", ")) }
        )),
    })
}

fn is_kiyi_file(path: &Path) -> Result<bool> {
    let mut first = String::new();
    BufReader::new(std::fs::File::open(path)?).read_line(&mut first)?;
    Ok(first.starts_with(HEADER))
}

/// Restores a backup into this database: Kiyi's files statement by statement in one transaction,
/// others with psql / mysql.
pub async fn restore(driver: &dyn DbDriver, target: &Target, path: &Path, job: &Job<'_>) -> Result<RestoreReport> {
    let kind = driver.dialect().kind;
    let kiyi = is_kiyi_file(path)?;
    match restore_from(driver, target, path, kiyi, job).await {
        Err(Error::Cancelled) => Err(Error::Invalid(
            match (kiyi, kind) {
                (true, DbKind::Mysql) => "Restore stopped. Nothing was kept, except structure changes, which MySQL can't roll back.",
                (_, DbKind::Mysql | DbKind::Sqlserver) => "Restore stopped. Statements that had already run were kept.",
                _ => "Restore stopped. Nothing was kept.",
            }
            .into(),
        )),
        other => other,
    }
}

async fn restore_from(driver: &dyn DbDriver, target: &Target, path: &Path, kiyi: bool, job: &Job<'_>) -> Result<RestoreReport> {
    let kind = driver.dialect().kind;
    let report = Reporter::new(job);
    report.update(true, |p| {
        p.stage = Stage::Copying;
        p.total_bytes = std::fs::metadata(path).ok().map(|m| m.len());
    });
    if kiyi {
        let mut lines = BufReader::new(std::fs::File::open(path)?).lines();
        let mut count = 0u64;
        let mut read = 0u64;
        let mut next = || -> Option<Result<String>> {
            if job.stopped() {
                return Some(Err(Error::Cancelled));
            }
            let mut sql = String::new();
            loop {
                let line = lines.next()?;
                if let Ok(line) = &line {
                    read += line.len() as u64 + 1;
                }
                match line {
                    Err(e) => return Some(Err(e.into())),
                    Ok(line) if line == END => break,
                    Ok(line) if sql.is_empty() && (line.is_empty() || line.starts_with("-- ")) => continue,
                    Ok(line) => {
                        sql.push_str(&line);
                        sql.push('\n');
                    }
                }
            }
            count += 1;
            report.update(false, |p| {
                p.rows = count;
                p.bytes = read;
            });
            Some(Ok(sql.trim_end().trim_end_matches(';').to_string()))
        };
        return match driver.execute_stream(&mut next).await {
            Ok(_) => Ok(RestoreReport { tool: "Kiyi".into(), statements: Some(count) }),
            Err(Error::Script { index, source }) => {
                Err(Error::Invalid(format!("Restore stopped at statement {} and nothing was kept{}: {}", index + 1, if kind == DbKind::Mysql { " (except structure changes, which MySQL can't roll back)" } else { "" }, crate::error::ErrorInfo::from(&*source).message)))
            }
            Err(e) => Err(e),
        };
    }
    match kind {
        DbKind::Postgres => {
            let psql = crate::tunnel::find_tool("psql").ok_or_else(|| Error::Invalid("Restoring this file needs psql, from the PostgreSQL client tools. Install them (e.g. `brew install libpq`), or restore a backup made by Kiyi.".into()))?;
            let mut args = vec!["--no-psqlrc".into(), "--set".into(), "ON_ERROR_STOP=1".into(), "--single-transaction".into(), format!("--file={}", path.display())];
            args.extend(pg_args(target));
            run_tool(&psql, &args, &pg_env(target), None, job, || {}).await?;
            Ok(RestoreReport { tool: "psql".into(), statements: None })
        }
        DbKind::Mysql => {
            // Dumps made elsewhere often name a DEFINER the restoring user may not impersonate.
            let cleaned = path.with_extension("kiyi-restore.sql");
            std::fs::copy(path, &cleaned)?;
            strip_definers(&cleaned)?;
            let result = restore_mysql(target, &cleaned, job).await;
            let _ = std::fs::remove_file(&cleaned);
            result
        }
        DbKind::Sqlserver => {
            let sqlcmd = crate::tunnel::find_tool("sqlcmd").ok_or_else(|| Error::Invalid("Restoring this file needs sqlcmd (Microsoft's command-line tools). Install it, or restore a backup made by Kiyi.".into()))?;
            let mut args = vec!["-S".into(), format!("{},{}", target.config.host, target.config.port), "-U".into(), target.config.user.clone(), "-d".into(), database(target), "-i".into(), path.display().to_string(), "-b".into()];
            if !matches!(target.config.ssl_mode, SslMode::VerifyCa | SslMode::VerifyFull) {
                // Same trust as the connection itself: encrypted, certificate not checked.
                args.push("-C".into());
            }
            let env: Vec<(String, String)> = target.password.iter().map(|p| ("SQLCMDPASSWORD".to_string(), p.clone())).collect();
            run_tool(&sqlcmd, &args, &env, None, job, || {}).await?;
            Ok(RestoreReport { tool: "sqlcmd".into(), statements: None })
        }
        DbKind::Sqlite => {
            // A plain .sql file (e.g. from `sqlite3 .dump`): split and run it in one transaction.
            let text = std::fs::read_to_string(path).map_err(|e| Error::Invalid(format!("Couldn't read the file: {e}")))?;
            if text.starts_with("SQLite format 3") {
                return Err(Error::Invalid("This is a SQLite database file, not a SQL script. Open it as its own connection instead.".into()));
            }
            let statements = crate::dml::split_sql(&text);
            let n = statements.len() as u64;
            let mut it = statements.into_iter().filter(|s| !matches!(s.to_ascii_uppercase().as_str(), "BEGIN TRANSACTION" | "BEGIN" | "COMMIT"));
            let mut count = 0u64;
            driver
                .execute_stream(&mut || {
                    if job.stopped() {
                        return Some(Err(Error::Cancelled));
                    }
                    count += 1;
                    report.update(false, |p| p.rows = count);
                    it.next().map(Ok)
                })
                .await?;
            Ok(RestoreReport { tool: "Kiyi".into(), statements: Some(n) })
        }
    }
}

async fn restore_mysql(target: &Target, path: &Path, job: &Job<'_>) -> Result<RestoreReport> {
    let mysql = crate::tunnel::find_tool("mysql").ok_or_else(|| Error::Invalid("Restoring this file needs the mysql command-line client. Install it (e.g. `brew install mysql-client`), or restore a backup made by Kiyi.".into()))?;
    let mut args = mysql_args(target, &mysql);
    args.push(database(target));
    run_tool(&mysql, &args, &mysql_env(target), Some(path), job, || {}).await?;
    Ok(RestoreReport { tool: "mysql".into(), statements: None })
}

/// Removes `DEFINER=`user`@`host`` from a MySQL dump in place, so objects belong to whoever restores it.
fn strip_definers(path: &Path) -> Result<()> {
    let tmp = path.with_extension("definers.tmp");
    {
        let reader = BufReader::new(std::fs::File::open(path)?);
        let mut out = BufWriter::new(std::fs::File::create(&tmp)?);
        for line in reader.split(b'\n') {
            let line = line?;
            let text = String::from_utf8_lossy(&line);
            if text.contains("DEFINER=") {
                out.write_all(remove_definer(&text).as_bytes())?;
            } else {
                out.write_all(&line)?;
            }
            out.write_all(b"\n")?;
        }
        out.flush()?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn remove_definer(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("DEFINER=") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "DEFINER=".len()..];
        // `user`@`host`, 'user'@'host', user@host or CURRENT_USER: skip to the next space.
        let end = after.find(|c: char| c.is_whitespace() || c == '*').unwrap_or(after.len());
        rest = after[end..].trim_start_matches(' ');
    }
    out.push_str(rest);
    out
}

/// Which tools a backup and a restore would use here, for the UI to say up front.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupTools {
    /// "pg_dump", "mysqldump", "SQLite", or None (Kiyi's own backup).
    pub backup: Option<String>,
    /// "psql" or "mysql" for dumps made by other tools; Kiyi files always restore.
    pub restore: Option<String>,
    pub suggested_name: String,
}

pub fn tools(config: &ConnectionConfig, today: &str) -> BackupTools {
    let found = |name: &str| crate::tunnel::find_tool(name).map(|_| name.to_string());
    let (backup, restore) = match config.kind {
        DbKind::Postgres => (found("pg_dump"), found("psql")),
        DbKind::Mysql => (found("mysqldump"), found("mysql")),
        DbKind::Sqlite => (Some("SQLite".to_string()), Some("SQLite".to_string())),
        DbKind::Sqlserver => (None, found("sqlcmd")),
    };
    BackupTools { backup, restore, suggested_name: suggested_name(config, today) }
}

/// A file name like `shop-2026-10-09.sql` for the save dialog.
pub fn suggested_name(config: &ConnectionConfig, today: &str) -> String {
    let base = match config.kind {
        DbKind::Sqlite => config.database.as_deref().and_then(|p| Path::new(p).file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "database".into()),
        _ => config.database.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| config.name.clone()),
    };
    let safe: String = base.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    format!("{safe}-{today}.{}", if config.kind == DbKind::Sqlite { "db" } else { "sql" })
}
