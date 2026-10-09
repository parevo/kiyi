//! Backing up and restoring a database. With the database's own tools installed (pg_dump/psql,
//! mysqldump/mysql) Kiyi uses them for a complete copy. Without them it writes its own SQL file
//! with the tables, their data, keys and indexes. SQLite backups are a consistent copy of the file.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::{ConnectionConfig, DbKind, SslMode};
use crate::design::{self, TableDesign};
use crate::drivers::DbDriver;
use crate::error::{is_missing_table, Error, Result};
use crate::types::{QueryEvent, ValueKind};

const HEADER: &str = "-- Kiyi backup";
/// Ends every statement in Kiyi's own files, so a restore can read them line by line.
const END: &str = "-- kiyi:end";
const INSERT_BATCH: usize = 200;

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

async fn run_tool(tool: &Path, args: &[String], env: &[(String, String)], stdin: Option<&Path>) -> Result<()> {
    let name = tool.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut cmd = tokio::process::Command::new(tool);
    cmd.args(args).envs(env.iter().cloned()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    cmd.stdin(match stdin {
        Some(path) => std::process::Stdio::from(std::fs::File::open(path)?),
        None => std::process::Stdio::null(),
    });
    let out = cmd.output().await.map_err(|e| Error::Invalid(format!("Couldn't start {name}: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    Err(Error::Invalid(format!("{name} failed: {}", if line.is_empty() { out.status.to_string() } else { line.to_string() })))
}

/// Backs up with the database's own tool when it's installed, else with Kiyi's SQL writer.
pub async fn backup(driver: &dyn DbDriver, target: &Target, path: &Path, prefer_kiyi: bool) -> Result<BackupReport> {
    let kind = driver.dialect().kind;
    if kind == DbKind::Sqlite {
        // A consistent copy of the whole file, even while it's in use.
        let _ = std::fs::remove_file(path);
        driver.execute_script(&[format!("VACUUM INTO {}", driver.dialect().string(&path.to_string_lossy()))], false, false).await?;
        let tables = driver.schema().await?.schemas.iter().map(|s| s.tables.len()).sum();
        return Ok(BackupReport { method: BackupMethod::Native, tool: "SQLite".into(), tables, rows: None, bytes: std::fs::metadata(path)?.len(), note: None });
    }
    // SQL Server has no dump tool that writes a runnable script; Kiyi's own format is used there.
    let tool = if prefer_kiyi || kind == DbKind::Sqlserver { None } else { crate::tunnel::find_tool(if kind == DbKind::Postgres { "pg_dump" } else { "mysqldump" }) };
    if let Some(tool) = tool {
        let snapshot = driver.schema().await?;
        // MySQL lists every database the user can see; count the connected one.
        let tables = snapshot.schemas.iter().filter(|s| kind != DbKind::Mysql || Some(&s.name) == snapshot.default_schema.as_ref()).map(|s| s.tables.len()).sum();
        if kind == DbKind::Postgres {
            let mut args = vec!["--no-owner".into(), "--no-privileges".into(), "--format=plain".into(), format!("--file={}", path.display())];
            args.extend(pg_args(target));
            run_tool(&tool, &args, &pg_env(target), None).await?;
        } else {
            let mut args = mysql_args(target, &tool);
            args.extend(["--single-transaction".into(), "--routines".into(), "--triggers".into(), "--no-tablespaces".into(), format!("--result-file={}", path.display()), database(target)]);
            run_tool(&tool, &args, &mysql_env(target), None).await?;
            // Portable dumps: without DEFINER clauses, anyone with rights on the target can restore.
            strip_definers(path)?;
        }
        let name = tool.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Ok(BackupReport { method: BackupMethod::Native, tool: name, tables, rows: None, bytes: std::fs::metadata(path)?.len(), note: None });
    }
    kiyi_backup(driver, path).await
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
async fn kiyi_backup(driver: &dyn DbDriver, path: &Path) -> Result<BackupReport> {
    let d = driver.dialect();
    let snapshot = driver.schema().await?;
    let default_schema = snapshot.default_schema.clone();
    let mut designs: Vec<(Option<String>, TableDesign)> = Vec::new();
    let mut vanished: Vec<String> = Vec::new();
    for sc in &snapshot.schemas {
        // MySQL lists every database the user can see; back up the connected one.
        if d.is_mysql() && Some(&sc.name) != default_schema.as_ref() {
            continue;
        }
        for t in sc.tables.iter().filter(|t| matches!(t.kind, crate::types::TableKind::Table)) {
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
    }
    let order = creation_order(&designs);
    let tmp = path.with_extension("partial");
    let out = Mutex::new(BufWriter::new(std::fs::File::create(&tmp)?));
    let write = |text: &str| -> std::io::Result<()> { out.lock().unwrap().write_all(text.as_bytes()) };
    let statement = |sql: &str| -> std::io::Result<()> { write(&format!("{sql};\n{END}\n")) };
    let engine = if d.is_mysql() { "MySQL" } else if d.is_sqlite() { "SQLite" } else { "PostgreSQL" };
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
    for &i in &order {
        let (schema, des) = &designs[i];
        let table = d.table(schema.as_deref(), &des.name);
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
                *rows.lock().unwrap() += batch.len() as u64;
            }
            _ => {}
        };
        if let Err(e) = driver.execute(&format!("SELECT * FROM {table}"), &|_| {}, &sink).await {
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
    out.into_inner().unwrap().flush()?;
    std::fs::rename(&tmp, path)?;
    Ok(BackupReport {
        method: BackupMethod::Kiyi,
        tool: "Kiyi".into(),
        tables: designs.len(),
        rows: Some(total_rows),
        bytes: std::fs::metadata(path)?.len(),
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
pub async fn restore(driver: &dyn DbDriver, target: &Target, path: &Path) -> Result<RestoreReport> {
    let kind = driver.dialect().kind;
    if is_kiyi_file(path)? {
        let mut lines = BufReader::new(std::fs::File::open(path)?).lines();
        let mut count = 0u64;
        let mut next = || -> Option<Result<String>> {
            let mut sql = String::new();
            loop {
                match lines.next()? {
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
            run_tool(&psql, &args, &pg_env(target), None).await?;
            Ok(RestoreReport { tool: "psql".into(), statements: None })
        }
        DbKind::Mysql => {
            // Dumps made elsewhere often name a DEFINER the restoring user may not impersonate.
            let cleaned = path.with_extension("kiyi-restore.sql");
            std::fs::copy(path, &cleaned)?;
            strip_definers(&cleaned)?;
            let result = restore_mysql(target, &cleaned).await;
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
            run_tool(&sqlcmd, &args, &env, None).await?;
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
            let mut it = statements.into_iter().filter(|s| !matches!(s.to_ascii_uppercase().as_str(), "BEGIN TRANSACTION" | "BEGIN" | "COMMIT")).map(Ok);
            driver.execute_stream(&mut || it.next()).await?;
            Ok(RestoreReport { tool: "Kiyi".into(), statements: Some(n) })
        }
    }
}

async fn restore_mysql(target: &Target, path: &Path) -> Result<RestoreReport> {
    let mysql = crate::tunnel::find_tool("mysql").ok_or_else(|| Error::Invalid("Restoring this file needs the mysql command-line client. Install it (e.g. `brew install mysql-client`), or restore a backup made by Kiyi.".into()))?;
    let mut args = mysql_args(target, &mysql);
    args.push(database(target));
    run_tool(&mysql, &args, &mysql_env(target), Some(path)).await?;
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
