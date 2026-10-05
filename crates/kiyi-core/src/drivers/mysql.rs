use std::time::Duration;

use async_trait::async_trait;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlQueryResult, MySqlRow, MySqlSslMode, MySqlTypeInfo};
use sqlx::{Column, Decode, Executor, MySql, Row, TypeInfo, ValueRef};

use super::engine;
use super::postgres::split_list;
use super::DbDriver;
use crate::config::{ConnectionConfig, SslMode};
use crate::design::{ColumnDesign, FkAction, ForeignKeyDesign, IndexDesign, TableDesign, TableDetails};
use crate::dialect::Dialect;
use crate::error::{Error, Result};
use crate::types::{Cell, ColumnMeta, SchemaSnapshot, Sink, ValueKind};

pub struct MySqlDriver {
    pool: MySqlPool,
    dialect: Dialect,
}

engine::define_engine!(sqlx::MySqlConnection);

fn rows_affected(r: &MySqlQueryResult) -> u64 {
    r.rows_affected()
}

fn kind_of(t: &MySqlTypeInfo) -> ValueKind {
    let name = t.name();
    let base = name.split_whitespace().next().unwrap_or(name);
    match base {
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "FLOAT" | "DOUBLE" | "DECIMAL" | "YEAR" | "BOOLEAN" => {
            ValueKind::Number
        }
        "JSON" => ValueKind::Json,
        "DATE" | "TIME" | "DATETIME" | "TIMESTAMP" => ValueKind::Temporal,
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BIT" | "GEOMETRY" => ValueKind::Binary,
        _ => ValueKind::Text,
    }
}

fn column(c: &sqlx::mysql::MySqlColumn) -> ColumnMeta {
    let t = c.type_info();
    ColumnMeta { name: c.name().to_string(), type_name: t.name().to_ascii_lowercase(), kind: kind_of(t) }
}

fn cell(row: &MySqlRow, i: usize, kind: ValueKind) -> Cell {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    let bytes = <&[u8] as Decode<MySql>>::decode(raw).unwrap_or_default();
    Some(match (kind, std::str::from_utf8(bytes)) {
        (ValueKind::Binary, _) | (_, Err(_)) => engine::hex(bytes),
        (_, Ok(text)) => text.to_string(),
    })
}

impl MySqlDriver {
    pub async fn connect(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        let mut options = MySqlConnectOptions::new()
            .host(&config.host)
            .port(config.port)
            .username(&config.user)
            .charset("utf8mb4")
            .ssl_mode(match config.ssl_mode {
                SslMode::Disable => MySqlSslMode::Disabled,
                SslMode::Prefer => MySqlSslMode::Preferred,
                SslMode::Require => MySqlSslMode::Required,
                SslMode::VerifyFull => MySqlSslMode::VerifyIdentity,
            });
        if let Some(p) = password {
            options = options.password(p);
        }
        if let Some(db) = &config.database {
            options = options.database(db);
        }

        let read_only = config.read_only;
        let pool = MySqlPoolOptions::new()
            .max_connections(4)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(15))
            .idle_timeout(Duration::from_secs(600))
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    if read_only {
                        conn.execute(sqlx::raw_sql("SET SESSION TRANSACTION READ ONLY")).await?;
                    }
                    Ok(())
                })
            })
            .connect_with(options)
            .await?;
        let mut conn = pool.acquire().await?;
        let sql_mode = first_cell(text_rows(&mut conn, "SELECT @@SESSION.sql_mode").await?).unwrap_or_default();
        drop(conn);
        let dialect = Dialect { backslash_escapes: !sql_mode.contains("NO_BACKSLASH_ESCAPES"), ..Dialect::MYSQL };
        Ok(Self { pool, dialect })
    }

    async fn scalar(&self, sql: &str) -> Result<String> {
        let mut conn = self.pool.acquire().await?;
        Ok(first_cell(text_rows(&mut conn, sql).await?).unwrap_or_default())
    }
}

const SCHEMA_SQL: &str = r#"
SELECT c.TABLE_SCHEMA, c.TABLE_NAME, t.TABLE_TYPE, c.COLUMN_NAME, c.COLUMN_TYPE, c.IS_NULLABLE,
       CASE WHEN t.TABLE_TYPE = 'BASE TABLE' THEN t.TABLE_ROWS END
FROM information_schema.COLUMNS c
JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = c.TABLE_SCHEMA AND t.TABLE_NAME = c.TABLE_NAME
WHERE c.TABLE_SCHEMA NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys')
ORDER BY c.TABLE_SCHEMA, c.TABLE_NAME, c.ORDINAL_POSITION
"#;

#[async_trait]
impl DbDriver for MySqlDriver {
    async fn server_version(&self) -> Result<String> {
        let v = self.scalar("SELECT VERSION()").await?;
        Ok(if v.to_ascii_lowercase().contains("mariadb") { format!("MariaDB {v}") } else { format!("MySQL {v}") })
    }

    async fn schema(&self) -> Result<SchemaSnapshot> {
        let default_schema = Some(self.scalar("SELECT DATABASE()").await?).filter(|s| !s.is_empty());
        let mut conn = self.pool.acquire().await?;
        let rows = text_rows(&mut conn, SCHEMA_SQL).await?;
        Ok(SchemaSnapshot::from_rows(default_schema, rows))
    }

    async fn execute(&self, sql: &str, on_session: &(dyn Fn(u64) + Send + Sync), sink: &Sink<'_>) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        run(&mut conn, "SELECT CONNECTION_ID()", sql, on_session, sink).await
    }

    async fn cancel(&self, session_id: u64) -> Result<()> {
        // KILL doesn't accept placeholders; session_id is a number, so formatting is safe.
        sqlx::raw_sql(&format!("KILL QUERY {session_id}")).execute(&self.pool).await?;
        Ok(())
    }

    async fn close(&self) {
        self.pool.close().await;
    }

    fn dialect(&self) -> Dialect {
        self.dialect
    }

    async fn table_details(&self, schema: Option<&str>, table: &str) -> Result<TableDetails> {
        let d = self.dialect;
        let mut conn = self.pool.acquire().await?;
        let schema_name = match schema {
            Some(s) => s.to_string(),
            None => first_cell(text_rows(&mut conn, "SELECT DATABASE()").await?).ok_or_else(|| Error::Invalid("No database is selected".into()))?,
        };
        let (schema_lit, table_lit) = (d.string(&schema_name), d.string(table));
        let filter = format!("TABLE_SCHEMA = {schema_lit} AND TABLE_NAME = {table_lit}");
        let get = |row: &Vec<Cell>, i: usize| row.get(i).cloned().flatten();

        let info = text_rows(&mut conn, &format!("SELECT TABLE_TYPE, TABLE_ROWS FROM information_schema.TABLES WHERE {filter}")).await?;
        let info = info.into_iter().next().ok_or_else(|| Error::Invalid(format!("Table {table} was not found")))?;
        let is_view = get(&info, 0).is_some_and(|t| t.contains("VIEW"));
        let row_estimate = get(&info, 1).and_then(|v| v.parse().ok());

        let columns = text_rows(&mut conn, &format!(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, EXTRA, COLUMN_COMMENT, COLUMN_KEY, DATA_TYPE \
             FROM information_schema.COLUMNS WHERE {filter} ORDER BY ORDINAL_POSITION"
        )).await?;
        let columns = columns
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                let extra = get(r, 4).unwrap_or_default();
                let extra_lower = extra.to_ascii_lowercase();
                let generated = extra_lower.contains("virtual generated") || extra_lower.contains("stored generated");
                let default = get(r, 3).filter(|_| !generated).map(|v| default_expr(d, &v, &extra_lower, &get(r, 7).unwrap_or_default()));
                // Keep clauses a MODIFY would otherwise drop.
                let on_update = extra_lower.find("on update ").map(|i| extra[i..].to_string());
                ColumnDesign {
                    original: Some(name.clone()),
                    name,
                    data_type: get(r, 1).unwrap_or_default(),
                    nullable: get(r, 2).as_deref() == Some("YES"),
                    default,
                    primary_key: get(r, 6).as_deref() == Some("PRI"),
                    auto_increment: extra_lower.contains("auto_increment"),
                    comment: get(r, 5).filter(|c| !c.is_empty()),
                    enum_values: enum_values(&get(r, 1).unwrap_or_default()),
                    generated,
                    extra: on_update,
                }
            })
            .collect();

        let indexes = text_rows(&mut conn, &format!(
            "SELECT INDEX_NAME, MIN(NON_UNIQUE), GROUP_CONCAT(COLUMN_NAME ORDER BY SEQ_IN_INDEX SEPARATOR X'1F') \
             FROM information_schema.STATISTICS WHERE {filter} AND INDEX_NAME <> 'PRIMARY' \
             GROUP BY INDEX_NAME ORDER BY INDEX_NAME"
        )).await?;
        let indexes = indexes
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                IndexDesign {
                    original: Some(name.clone()),
                    name,
                    unique: get(r, 1).as_deref() == Some("0"),
                    columns: get(r, 2).map(|s| split_list(&s)).unwrap_or_default(),
                    is_constraint: false,
                }
            })
            .collect();

        let fks = text_rows(&mut conn, &format!(
            "SELECT k.CONSTRAINT_NAME, GROUP_CONCAT(k.COLUMN_NAME ORDER BY k.ORDINAL_POSITION SEPARATOR X'1F'), \
             k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, \
             GROUP_CONCAT(k.REFERENCED_COLUMN_NAME ORDER BY k.ORDINAL_POSITION SEPARATOR X'1F'), r.DELETE_RULE, r.UPDATE_RULE \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
               ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = {schema_lit} AND k.TABLE_NAME = {table_lit} AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             GROUP BY k.CONSTRAINT_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, r.DELETE_RULE, r.UPDATE_RULE \
             ORDER BY k.CONSTRAINT_NAME"
        )).await?;
        let foreign_keys = fks
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                let ref_schema = get(r, 2);
                ForeignKeyDesign {
                    original: Some(name.clone()),
                    name,
                    columns: get(r, 1).map(|s| split_list(&s)).unwrap_or_default(),
                    // Same-database references are written without a schema prefix.
                    ref_schema: ref_schema.filter(|s| *s != schema_name),
                    ref_table: get(r, 3).unwrap_or_default(),
                    ref_columns: get(r, 4).map(|s| split_list(&s)).unwrap_or_default(),
                    on_delete: FkAction::parse(&get(r, 5).unwrap_or_default()),
                    on_update: FkAction::parse(&get(r, 6).unwrap_or_default()),
                }
            })
            .collect();

        Ok(TableDetails {
            schema: Some(schema_name),
            design: TableDesign { name: table.to_string(), columns, indexes, foreign_keys, primary_key_name: Some("PRIMARY".into()) },
            is_view,
            row_estimate,
        })
    }

    async fn fetch(&self, sql: &str) -> Result<(Vec<ColumnMeta>, Vec<Vec<Cell>>)> {
        let mut conn = self.pool.acquire().await?;
        fetch_all(&mut conn, sql).await
    }

    async fn execute_script(&self, statements: &[String], transactional: bool, expect_single_row: bool) -> Result<Vec<u64>> {
        let mut conn = self.pool.acquire().await?;
        script(&mut conn, "START TRANSACTION", statements, transactional, expect_single_row).await
    }
}

/// information_schema reports defaults as bare text; turn them back into SQL.
/// MySQL 8 flags expression defaults with DEFAULT_GENERATED.
fn default_expr(d: Dialect, raw: &str, extra_lower: &str, data_type: &str) -> String {
    let upper = raw.to_ascii_uppercase();
    if upper.starts_with("CURRENT_TIMESTAMP") || upper == "NOW()" || upper.starts_with("NULL") && raw.len() == 4 {
        raw.to_string()
    } else if extra_lower.contains("default_generated") {
        format!("({raw})")
    } else if matches!(data_type, "bit") && raw.starts_with("b'") {
        raw.to_string()
    } else {
        d.string(raw)
    }
}

/// Values of an `enum('a','b')` column type, unescaping doubled quotes.
fn enum_values(column_type: &str) -> Vec<String> {
    let Some(inner) = column_type.strip_prefix("enum(").and_then(|s| s.strip_suffix(')')) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' {
            continue;
        }
        let mut value = String::new();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    value.push('\'');
                } else {
                    break;
                }
            } else {
                value.push(c);
            }
        }
        out.push(value);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_enum_values() {
        assert_eq!(super::enum_values("enum('a','it''s','c d')"), ["a", "it's", "c d"]);
        assert!(super::enum_values("varchar(10)").is_empty());
    }
}
