//! Microsoft SQL Server and Azure SQL, over TDS with tiberius.
//!
//! tiberius has no pool, so this keeps a few idle connections itself. A connection goes back to
//! the pool only after a call finished cleanly; one that failed mid-stream or was killed is dropped.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::TryStreamExt;
use tiberius::{AuthMethod, ColumnData, ColumnType, EncryptionLevel, FromSql, QueryItem};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::DbDriver;
use crate::config::{ConnectionConfig, SslMode};
use crate::design::{ColumnDesign, FkAction, ForeignKeyDesign, IndexDesign, TableDesign, TableDetails};
use crate::dialect::Dialect;
use crate::drivers::engine::{FLUSH_INTERVAL, MAX_BATCH};
use crate::error::{Error, Result};
use crate::types::{Cell, ColumnMeta, QueryEvent, SchemaSnapshot, Sink, ValueKind};

type Client = tiberius::Client<Compat<TcpStream>>;

const MAX_CONNECTIONS: usize = 4;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct SqlServerDriver {
    config: Mutex<tiberius::Config>,
    /// tiberius keeps the login inside its auth method; kept here to swap in fresh IAM-style passwords.
    user: String,
    idle: Mutex<Vec<Client>>,
    permits: Semaphore,
    /// Sessions this driver killed on request, so their errors read as a cancel.
    killed: Mutex<HashSet<u64>>,
}

fn mssql_error(e: tiberius::error::Error) -> Error {
    match e {
        tiberius::error::Error::Server(t) => Error::Mssql { code: t.code(), message: t.message().to_string() },
        tiberius::error::Error::Io { kind, message } => Error::Io(std::io::Error::new(kind, message)),
        other => Error::Invalid(other.to_string()),
    }
}

/// A pooled connection; returned to the pool only when `done()` was called.
struct Lease<'a> {
    driver: &'a SqlServerDriver,
    client: Option<Client>,
    _permit: tokio::sync::SemaphorePermit<'a>,
}

impl Lease<'_> {
    fn client(&mut self) -> &mut Client {
        self.client.as_mut().expect("leased client")
    }

    fn done(mut self) {
        if let Some(c) = self.client.take() {
            self.driver.idle.lock().unwrap().push(c);
        }
    }
}

impl SqlServerDriver {
    pub async fn connect(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        let mut c = tiberius::Config::new();
        c.host(&config.host);
        c.port(config.port);
        c.authentication(AuthMethod::sql_server(&config.user, password.unwrap_or_default()));
        if let Some(db) = config.database.as_deref().filter(|d| !d.is_empty()) {
            c.database(db);
        }
        c.application_name("Kiyi");
        let cert = config.ssl_root_cert.as_deref().filter(|c| !c.trim().is_empty());
        match config.ssl_mode {
            // Only the login is encrypted (the server may still insist on more).
            SslMode::Disable => {
                c.encryption(EncryptionLevel::Off);
                c.trust_cert();
            }
            SslMode::Prefer => {
                c.encryption(EncryptionLevel::On);
                c.trust_cert();
            }
            SslMode::Require => {
                c.encryption(EncryptionLevel::Required);
                c.trust_cert();
            }
            SslMode::VerifyCa | SslMode::VerifyFull => {
                c.encryption(EncryptionLevel::Required);
                if let Some(path) = cert {
                    c.trust_cert_ca(path.trim());
                }
            }
        }
        let driver = Self { config: Mutex::new(c), user: config.user.clone(), idle: Mutex::new(Vec::new()), permits: Semaphore::new(MAX_CONNECTIONS), killed: Mutex::new(HashSet::new()) };
        // Prove the settings work now, like the other drivers do.
        driver.lease().await?.done();
        Ok(driver)
    }

    async fn open(&self) -> Result<Client> {
        let mut config = self.config.lock().unwrap().clone();
        let mut redirects = 0;
        loop {
            let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(config.get_addr())).await.map_err(|_| Error::Timeout("connecting"))??;
            tcp.set_nodelay(true)?;
            match tiberius::Client::connect(config.clone(), tcp.compat_write()).await {
                Ok(client) => return Ok(client),
                // Azure SQL sends clients on to the node that holds the database.
                Err(tiberius::error::Error::Routing { host, port }) if redirects < 3 => {
                    redirects += 1;
                    config.host(&host);
                    config.port(port);
                }
                Err(e) => return Err(mssql_error(e)),
            }
        }
    }

    async fn lease(&self) -> Result<Lease<'_>> {
        let permit = self.permits.acquire().await.map_err(|_| Error::NotConnected)?;
        let pooled = self.idle.lock().unwrap().pop();
        let client = match pooled {
            Some(c) => c,
            None => self.open().await?,
        };
        Ok(Lease { driver: self, client: Some(client), _permit: permit })
    }

    async fn rows(&self, sql: &str) -> Result<Vec<Vec<Cell>>> {
        Ok(self.fetch(sql).await?.1)
    }

    async fn scalar(&self, sql: &str) -> Result<Option<String>> {
        Ok(self.rows(sql).await?.into_iter().next().and_then(|r| r.into_iter().next().flatten()))
    }
}

fn kind_of(t: ColumnType) -> ValueKind {
    use ColumnType::*;
    match t {
        Bit | Bitn => ValueKind::Bool,
        Int1 | Int2 | Int4 | Int8 | Intn | Float4 | Float8 | Floatn | Money | Money4 | Decimaln | Numericn => ValueKind::Number,
        Datetime | Datetime4 | Datetimen | Daten | Timen | Datetime2 | DatetimeOffsetn => ValueKind::Temporal,
        Guid => ValueKind::Uuid,
        BigVarBin | BigBinary | Image => ValueKind::Binary,
        BigVarChar | BigChar | NVarchar | NChar | Text | NText => ValueKind::Text,
        Xml | Udt | SSVariant | Null => ValueKind::Other,
    }
}

fn type_name(t: ColumnType) -> &'static str {
    use ColumnType::*;
    match t {
        Bit | Bitn => "bit",
        Int1 => "tinyint",
        Int2 => "smallint",
        Int4 => "int",
        Int8 => "bigint",
        Intn => "int",
        Float4 => "real",
        Float8 | Floatn => "float",
        Money | Money4 => "money",
        Decimaln | Numericn => "decimal",
        Datetime | Datetime4 | Datetimen => "datetime",
        Daten => "date",
        Timen => "time",
        Datetime2 => "datetime2",
        DatetimeOffsetn => "datetimeoffset",
        Guid => "uniqueidentifier",
        BigVarBin | BigBinary | Image => "varbinary",
        BigVarChar | BigChar | Text => "varchar",
        NVarchar | NChar | NText => "nvarchar",
        Xml => "xml",
        Udt => "udt",
        SSVariant => "sql_variant",
        Null => "null",
    }
}

/// A value as the text the other drivers would show (ISO dates, `true`/`false`, `0x…` bytes).
fn cell(data: &ColumnData<'static>) -> Cell {
    use tiberius::time::chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
    match data {
        ColumnData::U8(v) => v.map(|x| x.to_string()),
        ColumnData::I16(v) => v.map(|x| x.to_string()),
        ColumnData::I32(v) => v.map(|x| x.to_string()),
        ColumnData::I64(v) => v.map(|x| x.to_string()),
        ColumnData::F32(v) => v.map(|x| x.to_string()),
        ColumnData::F64(v) => v.map(|x| x.to_string()),
        ColumnData::Bit(v) => v.map(|x| x.to_string()),
        ColumnData::String(v) => v.as_ref().map(|s| s.to_string()),
        ColumnData::Guid(v) => v.map(|g| g.to_string().to_uppercase()),
        ColumnData::Binary(v) => v.as_ref().map(|b| format!("0x{}", b.iter().map(|x| format!("{x:02X}")).collect::<String>())),
        ColumnData::Numeric(v) => v.map(|n| n.to_string()),
        ColumnData::Xml(v) => v.as_ref().map(|x| x.to_string()),
        ColumnData::Date(_) => NaiveDate::from_sql(data).ok().flatten().map(|d| d.format("%Y-%m-%d").to_string()),
        ColumnData::Time(_) => NaiveTime::from_sql(data).ok().flatten().map(|t| t.format("%H:%M:%S%.f").to_string()),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            NaiveDateTime::from_sql(data).ok().flatten().map(|t| t.format("%Y-%m-%d %H:%M:%S%.f").to_string())
        }
        ColumnData::DateTimeOffset(_) => DateTime::<FixedOffset>::from_sql(data).ok().flatten().map(|t| {
            // %.f prints only the fraction digits the value has (none, 3, 6 or 9).
            t.format("%Y-%m-%d %H:%M:%S%.f%:z").to_string()
        }),
    }
}

/// Runs one statement as a plain SQL batch and returns the rows it changed. `execute` would wrap
/// it in sp_executesql, whose scope undoes session settings such as SET IDENTITY_INSERT.
async fn batch(client: &mut Client, sql: &str) -> std::result::Result<u64, tiberius::error::Error> {
    let results = client.simple_query(format!("{sql};\nSELECT CAST(@@ROWCOUNT AS bigint)")).await?.into_results().await?;
    Ok(results.last().and_then(|rows| rows.first()).and_then(|r| r.get::<i64, _>(0)).unwrap_or(0) as u64)
}

/// Scripts with no statement that can return rows run through `execute`, which reports rows affected.
fn only_changes(sql: &str) -> bool {
    let statements = crate::dml::split_sql(sql);
    !statements.is_empty()
        && statements.iter().all(|s| {
            let first = s.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
            matches!(first.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "GRANT" | "REVOKE")
        })
}

const SCHEMA_SQL: &str = r#"
SELECT s.name, o.name, CASE WHEN o.type = 'V' THEN 'VIEW' ELSE 'TABLE' END, c.name,
  TYPE_NAME(c.user_type_id) + CASE
    WHEN TYPE_NAME(c.user_type_id) IN ('varchar', 'char', 'varbinary', 'binary') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')'
    WHEN TYPE_NAME(c.user_type_id) IN ('nvarchar', 'nchar') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')'
    WHEN TYPE_NAME(c.user_type_id) IN ('decimal', 'numeric') THEN '(' + CAST(c.precision AS varchar(5)) + ',' + CAST(c.scale AS varchar(5)) + ')'
    WHEN TYPE_NAME(c.user_type_id) IN ('datetime2', 'time', 'datetimeoffset') THEN '(' + CAST(c.scale AS varchar(5)) + ')'
    ELSE '' END,
  CASE WHEN c.is_nullable = 1 THEN 'YES' ELSE 'NO' END,
  CASE WHEN o.type = 'U' THEN (SELECT SUM(p.rows) FROM sys.partitions p WHERE p.object_id = o.object_id AND p.index_id IN (0, 1)) END
FROM sys.objects o
JOIN sys.schemas s ON s.schema_id = o.schema_id
JOIN sys.columns c ON c.object_id = o.object_id
WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0
ORDER BY s.name, o.name, c.column_id
"#;

/// The full type of a column as SQL Server spells it, e.g. `nvarchar(120)`.
const TYPE_EXPR: &str = "TYPE_NAME(c.user_type_id) + CASE \
    WHEN TYPE_NAME(c.user_type_id) IN ('varchar', 'char', 'varbinary', 'binary') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')' \
    WHEN TYPE_NAME(c.user_type_id) IN ('nvarchar', 'nchar') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')' \
    WHEN TYPE_NAME(c.user_type_id) IN ('decimal', 'numeric') THEN '(' + CAST(c.precision AS varchar(5)) + ',' + CAST(c.scale AS varchar(5)) + ')' \
    WHEN TYPE_NAME(c.user_type_id) IN ('datetime2', 'time', 'datetimeoffset') THEN '(' + CAST(c.scale AS varchar(5)) + ')' \
    ELSE '' END";

#[async_trait]
impl DbDriver for SqlServerDriver {
    async fn server_version(&self) -> Result<String> {
        let v = self.scalar("SELECT @@VERSION").await?.unwrap_or_default();
        Ok(v.lines().next().unwrap_or("SQL Server").trim().to_string())
    }

    async fn schema(&self) -> Result<SchemaSnapshot> {
        let default = self.scalar("SELECT SCHEMA_NAME()").await?;
        Ok(SchemaSnapshot::from_rows(default, self.rows(SCHEMA_SQL).await?))
    }

    async fn execute(&self, sql: &str, on_session: &(dyn Fn(u64) + Send + Sync), sink: &Sink<'_>) -> Result<()> {
        let mut lease = self.lease().await?;
        let spid: u64 = {
            let row = lease.client().simple_query("SELECT CAST(@@SPID AS bigint)").await.map_err(mssql_error)?.into_row().await.map_err(mssql_error)?;
            row.and_then(|r| r.get::<i64, _>(0)).unwrap_or(0) as u64
        };
        on_session(spid);
        let outcome = run(lease.client(), sql, sink).await;
        let killed = self.killed.lock().unwrap().remove(&spid);
        match outcome {
            Ok(()) => {
                lease.done();
                Ok(())
            }
            Err(_) if killed => Err(Error::Cancelled),
            Err(e) => Err(e),
        }
    }

    async fn cancel(&self, session_id: u64) -> Result<()> {
        // TDS cancels need the busy connection itself; ending its session from another one works anywhere.
        self.killed.lock().unwrap().insert(session_id);
        let mut lease = self.lease().await?;
        lease.client().execute(format!("KILL {session_id}"), &[]).await.map_err(mssql_error)?;
        lease.done();
        Ok(())
    }

    async fn close(&self) {
        let clients: Vec<Client> = std::mem::take(&mut *self.idle.lock().unwrap());
        for c in clients {
            let _ = c.close().await;
        }
    }

    fn dialect(&self) -> Dialect {
        Dialect::SQLSERVER
    }

    async fn table_details(&self, schema: Option<&str>, table: &str) -> Result<TableDetails> {
        let d = Dialect::SQLSERVER;
        let schema_name = match schema {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => self.scalar("SELECT SCHEMA_NAME()").await?.unwrap_or_else(|| "dbo".into()),
        };
        let object = d.string(&format!("[{}].[{}]", schema_name.replace(']', "]]"), table.replace(']', "]]")));
        let id = format!("OBJECT_ID({object})");
        let get = |row: &Vec<Cell>, i: usize| row.get(i).cloned().flatten();

        let info = self.rows(&format!("SELECT o.type, (SELECT SUM(p.rows) FROM sys.partitions p WHERE p.object_id = o.object_id AND p.index_id IN (0, 1)) FROM sys.objects o WHERE o.object_id = {id}")).await?;
        let info = info.into_iter().next().ok_or_else(|| Error::Mssql { code: 208, message: format!("Table {table} was not found") })?;
        let is_view = get(&info, 0).is_some_and(|t| t.trim() == "V");
        let row_estimate = get(&info, 1).and_then(|v| v.parse().ok());

        let pk_name = self.scalar(&format!("SELECT name FROM sys.indexes WHERE object_id = {id} AND is_primary_key = 1")).await?;
        let columns = self
            .rows(&format!(
                "SELECT c.name, {TYPE_EXPR}, c.is_nullable, OBJECT_DEFINITION(c.default_object_id), c.is_identity, c.is_computed, \
                 CAST(ep.value AS nvarchar(4000)), \
                 CASE WHEN EXISTS (SELECT 1 FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                   WHERE i.object_id = c.object_id AND i.is_primary_key = 1 AND ic.column_id = c.column_id) THEN 1 ELSE 0 END \
                 FROM sys.columns c \
                 LEFT JOIN sys.extended_properties ep ON ep.major_id = c.object_id AND ep.minor_id = c.column_id AND ep.name = 'MS_Description' \
                 WHERE c.object_id = {id} ORDER BY c.column_id"
            ))
            .await?
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                let flag = |i: usize| matches!(get(r, i).as_deref(), Some("true") | Some("1"));
                ColumnDesign {
                    original: Some(name.clone()),
                    name,
                    data_type: get(r, 1).unwrap_or_default(),
                    nullable: flag(2),
                    default: get(r, 3),
                    primary_key: flag(7),
                    auto_increment: flag(4),
                    comment: get(r, 6).filter(|c| !c.is_empty()),
                    generated: flag(5),
                    extra: None,
                    enum_values: vec![],
                }
            })
            .collect();

        let indexes = self
            .rows(&format!(
                "SELECT i.name, i.is_unique, i.is_unique_constraint, \
                 STRING_AGG(c.name, CHAR(31)) WITHIN GROUP (ORDER BY ic.key_ordinal) \
                 FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                 JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
                 WHERE i.object_id = {id} AND i.is_primary_key = 0 AND i.type > 0 AND ic.is_included_column = 0 \
                 GROUP BY i.name, i.is_unique, i.is_unique_constraint ORDER BY i.name"
            ))
            .await?
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                IndexDesign {
                    original: Some(name.clone()),
                    name,
                    unique: get(r, 1).as_deref() == Some("true"),
                    columns: get(r, 3).map(|s| super::postgres::split_list(&s)).unwrap_or_default(),
                    is_constraint: get(r, 2).as_deref() == Some("true"),
                }
            })
            .collect();

        let action = |v: Option<String>| FkAction::parse(&v.unwrap_or_default().replace('_', " "));
        let foreign_keys = self
            .rows(&format!(
                "SELECT fk.name, \
                 STRING_AGG(pc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id), \
                 SCHEMA_NAME(rt.schema_id), rt.name, \
                 STRING_AGG(rc.name, CHAR(31)) WITHIN GROUP (ORDER BY fkc.constraint_column_id), \
                 fk.delete_referential_action_desc, fk.update_referential_action_desc \
                 FROM sys.foreign_keys fk \
                 JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
                 JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
                 JOIN sys.tables rt ON rt.object_id = fk.referenced_object_id \
                 JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
                 WHERE fk.parent_object_id = {id} \
                 GROUP BY fk.name, rt.schema_id, rt.name, fk.delete_referential_action_desc, fk.update_referential_action_desc ORDER BY fk.name"
            ))
            .await?
            .into_iter()
            .map(|r| {
                let name = get(&r, 0).unwrap_or_default();
                ForeignKeyDesign {
                    original: Some(name.clone()),
                    name,
                    columns: get(&r, 1).map(|s| super::postgres::split_list(&s)).unwrap_or_default(),
                    ref_schema: get(&r, 2),
                    ref_table: get(&r, 3).unwrap_or_default(),
                    ref_columns: get(&r, 4).map(|s| super::postgres::split_list(&s)).unwrap_or_default(),
                    on_delete: action(get(&r, 5)),
                    on_update: action(get(&r, 6)),
                }
            })
            .collect();

        Ok(TableDetails {
            schema: Some(schema_name),
            design: TableDesign { name: table.to_string(), columns, indexes, foreign_keys, primary_key_name: pk_name },
            is_view,
            row_estimate,
        })
    }

    async fn fetch(&self, sql: &str) -> Result<(Vec<ColumnMeta>, Vec<Vec<Cell>>)> {
        let mut lease = self.lease().await?;
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        {
            let mut stream = lease.client().simple_query(sql).await.map_err(mssql_error)?;
            while let Some(item) = stream.try_next().await.map_err(mssql_error)? {
                match item {
                    // Keep the first result set, as the other drivers do.
                    QueryItem::Metadata(meta) if meta.result_index() == 0 => {
                        columns = meta.columns().iter().map(|c| ColumnMeta { name: c.name().to_string(), type_name: type_name(c.column_type()).into(), kind: kind_of(c.column_type()) }).collect();
                    }
                    QueryItem::Row(row) if row.result_index() == 0 => rows.push(row.cells().map(|(_, v)| cell(v)).collect()),
                    _ => {}
                }
            }
        }
        lease.done();
        Ok((columns, rows))
    }

    async fn execute_script(&self, statements: &[String], transactional: bool, expect_single_row: bool) -> Result<Vec<u64>> {
        let mut lease = self.lease().await?;
        let client = lease.client();
        if transactional {
            client.simple_query("BEGIN TRANSACTION").await.map_err(mssql_error)?.into_results().await.map_err(mssql_error)?;
        }
        let mut affected = Vec::new();
        let mut failure = None;
        for (index, sql) in statements.iter().enumerate() {
            match batch(client, sql).await {
                Ok(n) => {
                    if expect_single_row && n != 1 {
                        failure = Some(Error::Script { index, source: Box::new(Error::RowMismatch(n)) });
                        break;
                    }
                    affected.push(n);
                }
                Err(e) => {
                    failure = Some(Error::Script { index, source: Box::new(mssql_error(e)) });
                    break;
                }
            }
        }
        match failure {
            None => {
                if transactional {
                    client.simple_query("COMMIT").await.map_err(mssql_error)?.into_results().await.map_err(mssql_error)?;
                }
                lease.done();
                Ok(affected)
            }
            Some(e) => {
                if transactional {
                    // XACT_STATE guards against a transaction the error already rolled back.
                    let _ = client.simple_query("IF XACT_STATE() <> 0 ROLLBACK").await;
                }
                Err(e)
            }
        }
    }

    async fn execute_stream(&self, next: &mut (dyn FnMut() -> Option<Result<String>> + Send)) -> Result<u64> {
        let mut lease = self.lease().await?;
        let client = lease.client();
        client.simple_query("BEGIN TRANSACTION").await.map_err(mssql_error)?.into_results().await.map_err(mssql_error)?;
        let mut total = 0u64;
        let mut index = 0usize;
        let outcome: Result<u64> = loop {
            let sql = match next() {
                None => break Ok(total),
                Some(Err(e)) => break Err(e),
                Some(Ok(sql)) => sql,
            };
            match batch(client, &sql).await {
                Ok(n) => total += n,
                Err(e) => break Err(Error::Script { index, source: Box::new(mssql_error(e)) }),
            }
            index += 1;
        };
        match outcome {
            Ok(n) => {
                client.simple_query("COMMIT").await.map_err(mssql_error)?.into_results().await.map_err(mssql_error)?;
                lease.done();
                Ok(n)
            }
            Err(e) => {
                let _ = client.simple_query("IF XACT_STATE() <> 0 ROLLBACK").await;
                Err(e)
            }
        }
    }

    fn set_password(&self, password: &str) {
        // Idle connections keep working; new ones sign in with the new password.
        self.config.lock().unwrap().authentication(AuthMethod::sql_server(&self.user, password));
    }

    async fn explain_rows(&self, sql: &str) -> Option<Result<Vec<Vec<Cell>>>> {
        Some(self.showplan(sql).await)
    }
}

impl SqlServerDriver {
    /// The estimated plan as SHOWPLAN_TEXT rows (one per step, `|--` marks the nesting). Needs one connection.
    async fn showplan(&self, sql: &str) -> Result<Vec<Vec<Cell>>> {
        let mut lease = self.lease().await?;
        let (result, off) = {
            let client = lease.client();
            client.simple_query("SET SHOWPLAN_TEXT ON").await.map_err(mssql_error)?.into_results().await.map_err(mssql_error)?;
            let result = async {
                let mut rows = Vec::new();
                let mut stream = client.simple_query(sql).await.map_err(mssql_error)?;
                while let Some(item) = stream.try_next().await.map_err(mssql_error)? {
                    // The first result set echoes the statement; the next is the plan.
                    if let QueryItem::Row(row) = item {
                        if row.result_index() >= 1 {
                            rows.push(row.cells().map(|(_, v)| cell(v)).collect());
                        }
                    }
                }
                Ok::<_, Error>(rows)
            }
            .await;
            let off = match client.simple_query("SET SHOWPLAN_TEXT OFF").await {
                Ok(stream) => stream.into_results().await.map(|_| ()).map_err(mssql_error),
                Err(e) => Err(mssql_error(e)),
            };
            (result, off)
        };
        let rows = result?;
        off?;
        lease.done();
        Ok(rows)
    }
}

/// Streams a script's results into `sink` in batches, like the other drivers.
async fn run(client: &mut Client, sql: &str, sink: &Sink<'_>) -> Result<()> {
    if only_changes(sql) {
        let r = client.execute(sql, &[]).await.map_err(mssql_error)?;
        for n in r.rows_affected() {
            sink(QueryEvent::StatementDone { rows_affected: *n });
        }
        if r.rows_affected().is_empty() {
            sink(QueryEvent::StatementDone { rows_affected: 0 });
        }
        return Ok(());
    }
    let mut stream = client.simple_query(sql).await.map_err(mssql_error)?;
    let mut batch: Vec<Vec<Cell>> = Vec::new();
    let mut last_flush = Instant::now();
    let mut current: Option<usize> = None;
    let mut rows_in_set = 0u64;
    let flush = |batch: &mut Vec<Vec<Cell>>| {
        if !batch.is_empty() {
            sink(QueryEvent::Rows { rows: std::mem::take(batch) });
        }
    };
    while let Some(item) = stream.try_next().await.map_err(mssql_error)? {
        match item {
            QueryItem::Metadata(meta) => {
                flush(&mut batch);
                if current.is_some() {
                    sink(QueryEvent::StatementDone { rows_affected: rows_in_set });
                }
                current = Some(meta.result_index());
                rows_in_set = 0;
                let columns = meta.columns().iter().map(|c| ColumnMeta { name: c.name().to_string(), type_name: type_name(c.column_type()).into(), kind: kind_of(c.column_type()) }).collect();
                sink(QueryEvent::Columns { columns });
            }
            QueryItem::Row(row) => {
                batch.push(row.cells().map(|(_, v)| cell(v)).collect());
                rows_in_set += 1;
                if batch.len() >= MAX_BATCH || last_flush.elapsed() >= FLUSH_INTERVAL {
                    flush(&mut batch);
                    last_flush = Instant::now();
                }
            }
        }
    }
    flush(&mut batch);
    sink(QueryEvent::StatementDone { rows_affected: rows_in_set });
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn scripts_that_only_change_data_report_counts() {
        assert!(super::only_changes("UPDATE t SET a = 1; DELETE FROM u"));
        assert!(!super::only_changes("UPDATE t SET a = 1; SELECT * FROM t"));
        assert!(!super::only_changes("EXEC sp_who"));
    }
}
