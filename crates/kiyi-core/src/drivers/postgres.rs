use std::time::Duration;

use async_trait::async_trait;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgQueryResult, PgRow, PgSslMode, PgTypeInfo};
use sqlx::{Column, Executor, Row, TypeInfo, ValueRef};

use super::engine;
use super::DbDriver;
use crate::config::{ConnectionConfig, SslMode};
use crate::design::{ColumnDesign, FkAction, ForeignKeyDesign, IndexDesign, TableDesign, TableDetails};
use crate::dialect::Dialect;
use crate::error::{Error, Result};
use crate::types::{Cell, ColumnMeta, SchemaSnapshot, Sink, ValueKind};

pub struct PgDriver {
    pool: PgPool,
}

engine::define_engine!(sqlx::PgConnection);

fn rows_affected(r: &PgQueryResult) -> u64 {
    r.rows_affected()
}

fn kind_of(t: &PgTypeInfo) -> ValueKind {
    match t.name() {
        "BOOL" => ValueKind::Bool,
        "INT2" | "INT4" | "INT8" | "FLOAT4" | "FLOAT8" | "NUMERIC" | "OID" | "MONEY" => ValueKind::Number,
        "JSON" | "JSONB" => ValueKind::Json,
        "UUID" => ValueKind::Uuid,
        "DATE" | "TIME" | "TIMETZ" | "TIMESTAMP" | "TIMESTAMPTZ" | "INTERVAL" => ValueKind::Temporal,
        "BYTEA" => ValueKind::Binary,
        "?" => ValueKind::Other,
        name if name.ends_with("[]") => ValueKind::Array,
        _ => ValueKind::Text,
    }
}

fn column(c: &sqlx::postgres::PgColumn) -> ColumnMeta {
    let t = c.type_info();
    let type_name = match t.name() {
        // Enums, domains and extension types aren't resolved in the simple protocol.
        "?" => t.oid().map(|o| format!("oid {}", o.0)).unwrap_or_else(|| "unknown".into()),
        name => name.to_ascii_lowercase(),
    };
    ColumnMeta { name: c.name().to_string(), type_name, kind: kind_of(t) }
}

fn cell(row: &PgRow, i: usize, kind: ValueKind) -> Cell {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    // Simple-query results are always in text format.
    let text = raw.as_str().unwrap_or_default();
    Some(match (kind, text) {
        (ValueKind::Bool, "t") => "true".into(),
        (ValueKind::Bool, "f") => "false".into(),
        _ => text.to_string(),
    })
}

impl PgDriver {
    pub async fn connect(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        let mut options = PgConnectOptions::new()
            .host(&config.host)
            .port(config.port)
            .username(&config.user)
            .application_name("Kıyı")
            .ssl_mode(match config.ssl_mode {
                SslMode::Disable => PgSslMode::Disable,
                SslMode::Prefer => PgSslMode::Prefer,
                SslMode::Require => PgSslMode::Require,
                SslMode::VerifyFull => PgSslMode::VerifyFull,
            });
        if let Some(p) = password {
            options = options.password(p);
        }
        if let Some(db) = &config.database {
            options = options.database(db);
        }

        let read_only = config.read_only;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(15))
            .idle_timeout(Duration::from_secs(600))
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    // Generated SQL escapes literals assuming standard strings; never rely on the server default.
                    conn.execute(sqlx::raw_sql("SET standard_conforming_strings = on")).await?;
                    if read_only {
                        conn.execute(sqlx::raw_sql("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")).await?;
                    }
                    Ok(())
                })
            })
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    async fn scalar(&self, sql: &str) -> Result<String> {
        let mut conn = self.pool.acquire().await?;
        Ok(first_cell(text_rows(&mut conn, sql).await?).unwrap_or_default())
    }
}

const SCHEMA_SQL: &str = r#"
SELECT n.nspname, c.relname,
       CASE WHEN c.relkind IN ('v', 'm') THEN 'VIEW' ELSE 'TABLE' END,
       a.attname, format_type(a.atttypid, a.atttypmod),
       CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
  AND NOT c.relispartition
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND n.nspname NOT LIKE 'pg\_toast%' AND n.nspname NOT LIKE 'pg\_temp%'
ORDER BY n.nspname, c.relname, a.attnum
"#;

#[async_trait]
impl DbDriver for PgDriver {
    async fn server_version(&self) -> Result<String> {
        self.scalar("SHOW server_version").await.map(|v| format!("PostgreSQL {v}"))
    }

    async fn schema(&self) -> Result<SchemaSnapshot> {
        let default_schema = Some(self.scalar("SELECT current_schema()").await?).filter(|s| !s.is_empty());
        let mut conn = self.pool.acquire().await?;
        let rows = text_rows(&mut conn, SCHEMA_SQL).await?;
        Ok(SchemaSnapshot::from_rows(default_schema, rows))
    }

    async fn execute(&self, sql: &str, on_session: &(dyn Fn(u64) + Send + Sync), sink: &Sink<'_>) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        run(&mut conn, "SELECT pg_backend_pid()", sql, on_session, sink).await
    }

    async fn cancel(&self, session_id: u64) -> Result<()> {
        sqlx::query("SELECT pg_cancel_backend($1)").bind(session_id as i32).execute(&self.pool).await?;
        Ok(())
    }

    async fn close(&self) {
        self.pool.close().await;
    }

    fn dialect(&self) -> Dialect {
        Dialect::POSTGRES
    }

    async fn table_details(&self, schema: Option<&str>, table: &str) -> Result<TableDetails> {
        let d = Dialect::POSTGRES;
        let mut conn = self.pool.acquire().await?;
        let oid = format!("{}::regclass", d.string(&d.table(schema, table)));

        let info = text_rows(&mut conn, &format!("SELECT c.reltuples::bigint, c.relkind, n.nspname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = {oid}")).await?;
        let info = info.into_iter().next().ok_or_else(|| Error::Invalid(format!("{table} bulunamadı")))?;
        let get = |row: &Vec<Cell>, i: usize| row.get(i).cloned().flatten();
        let row_estimate = get(&info, 0).and_then(|v| v.parse::<i64>().ok()).filter(|n| *n >= 0);
        let is_view = matches!(get(&info, 1).as_deref(), Some("v") | Some("m"));
        let schema_name = get(&info, 2);

        let pk = text_rows(&mut conn, &format!(
            "SELECT c.conname, array_to_string(ARRAY(SELECT a.attname FROM unnest(c.conkey) WITH ORDINALITY k(n, o) \
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n ORDER BY k.o), chr(31)) \
             FROM pg_constraint c WHERE c.conrelid = {oid} AND c.contype = 'p'"
        )).await?;
        let primary_key_name = pk.first().and_then(|r| get(r, 0));
        let pk_cols: Vec<String> = pk.first().and_then(|r| get(r, 1)).map(|s| split_list(&s)).unwrap_or_default();

        let columns = text_rows(&mut conn, &format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(ad.adbin, ad.adrelid), \
             a.attidentity <> '', col_description(a.attrelid, a.attnum), a.attgenerated <> '' \
             FROM pg_attribute a LEFT JOIN pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum \
             WHERE a.attrelid = {oid} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
        )).await?;
        let columns = columns
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                let generated = get(r, 6).as_deref() == Some("t");
                ColumnDesign {
                    original: Some(name.clone()),
                    primary_key: pk_cols.contains(&name),
                    name,
                    data_type: get(r, 1).unwrap_or_default(),
                    nullable: get(r, 2).as_deref() != Some("t"),
                    // A generated column's expression isn't a default.
                    default: if generated { None } else { get(r, 3) },
                    auto_increment: get(r, 4).as_deref() == Some("t"),
                    comment: get(r, 5),
                    generated,
                    extra: None,
                }
            })
            .collect();

        let indexes = text_rows(&mut conn, &format!(
            "SELECT i.relname, ix.indisunique, con.oid IS NOT NULL, \
             array_to_string(ARRAY(SELECT pg_get_indexdef(ix.indexrelid, k, true) FROM generate_series(1, ix.indnkeyatts) k), chr(31)) \
             FROM pg_index ix JOIN pg_class i ON i.oid = ix.indexrelid \
             LEFT JOIN pg_constraint con ON con.conindid = ix.indexrelid AND con.contype IN ('u', 'x') \
             WHERE ix.indrelid = {oid} AND NOT ix.indisprimary ORDER BY i.relname"
        )).await?;
        let indexes = indexes
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                IndexDesign {
                    original: Some(name.clone()),
                    name,
                    unique: get(r, 1).as_deref() == Some("t"),
                    is_constraint: get(r, 2).as_deref() == Some("t"),
                    columns: get(r, 3).map(|s| split_list(&s)).unwrap_or_default(),
                }
            })
            .collect();

        let fks = text_rows(&mut conn, &format!(
            "SELECT c.conname, \
             array_to_string(ARRAY(SELECT a.attname FROM unnest(c.conkey) WITH ORDINALITY k(n, o) JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.n ORDER BY k.o), chr(31)), \
             rn.nspname, rc.relname, \
             array_to_string(ARRAY(SELECT a.attname FROM unnest(c.confkey) WITH ORDINALITY k(n, o) JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.n ORDER BY k.o), chr(31)), \
             c.confdeltype, c.confupdtype \
             FROM pg_constraint c JOIN pg_class rc ON rc.oid = c.confrelid JOIN pg_namespace rn ON rn.oid = rc.relnamespace \
             WHERE c.conrelid = {oid} AND c.contype = 'f' ORDER BY c.conname"
        )).await?;
        let foreign_keys = fks
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                ForeignKeyDesign {
                    original: Some(name.clone()),
                    name,
                    columns: get(r, 1).map(|s| split_list(&s)).unwrap_or_default(),
                    ref_schema: get(r, 2),
                    ref_table: get(r, 3).unwrap_or_default(),
                    ref_columns: get(r, 4).map(|s| split_list(&s)).unwrap_or_default(),
                    on_delete: FkAction::parse(&get(r, 5).unwrap_or_default()),
                    on_update: FkAction::parse(&get(r, 6).unwrap_or_default()),
                }
            })
            .collect();

        Ok(TableDetails {
            schema: schema_name,
            design: TableDesign { name: table.to_string(), columns, indexes, foreign_keys, primary_key_name },
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
        script(&mut conn, "BEGIN", statements, transactional, expect_single_row).await
    }
}

/// Splits the `chr(31)`-joined lists the catalog queries return.
pub(super) fn split_list(s: &str) -> Vec<String> {
    s.split('\u{1f}').filter(|p| !p.is_empty()).map(str::to_string).collect()
}
