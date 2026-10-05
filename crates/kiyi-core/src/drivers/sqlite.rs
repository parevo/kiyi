use std::time::Duration;

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteQueryResult, SqliteRow, SqliteTypeInfo};
use sqlx::{Column, Decode, Row, Sqlite, TypeInfo, ValueRef};

use super::engine;
use super::postgres::split_list;
use super::DbDriver;
use crate::config::ConnectionConfig;
use crate::design::{ColumnDesign, FkAction, ForeignKeyDesign, IndexDesign, TableDesign, TableDetails};
use crate::dialect::Dialect;
use crate::error::{Error, Result};
use crate::types::{Cell, ColumnMeta, SchemaSnapshot, Sink, ValueKind};

pub struct SqliteDriver {
    pool: SqlitePool,
}

engine::define_engine!(sqlx::SqliteConnection);

fn rows_affected(r: &SqliteQueryResult) -> u64 {
    r.rows_affected()
}

/// Declared column types are free text in SQLite; classify them by SQLite's affinity rules.
fn kind_of_name(name: &str) -> ValueKind {
    let t = name.to_ascii_uppercase();
    if t.contains("BOOL") {
        ValueKind::Bool
    } else if t.contains("INT") || t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") || t.contains("NUMERIC") || t.contains("DECIMAL") {
        ValueKind::Number
    } else if t.contains("DATE") || t.contains("TIME") {
        ValueKind::Temporal
    } else if t.contains("JSON") {
        ValueKind::Json
    } else if t.contains("BLOB") {
        ValueKind::Binary
    } else {
        ValueKind::Text
    }
}

fn kind_of(t: &SqliteTypeInfo) -> ValueKind {
    kind_of_name(t.name())
}

fn column(c: &sqlx::sqlite::SqliteColumn) -> ColumnMeta {
    let t = c.type_info();
    ColumnMeta { name: c.name().to_string(), type_name: t.name().to_ascii_lowercase(), kind: kind_of(t) }
}

fn cell(row: &SqliteRow, i: usize, kind: ValueKind) -> Cell {
    let raw = row.try_get_raw(i).ok()?;
    if raw.is_null() {
        return None;
    }
    // A value's storage class can differ from the declared type; blobs are always shown as hex.
    let stored = raw.type_info().name().to_ascii_uppercase();
    if kind == ValueKind::Binary || stored == "BLOB" {
        let bytes = <&[u8] as Decode<Sqlite>>::decode(raw).unwrap_or_default();
        return Some(engine::hex(bytes));
    }
    let text = <&str as Decode<Sqlite>>::decode(raw).unwrap_or_default();
    Some(match (kind, text) {
        (ValueKind::Bool, "1") => "true".into(),
        (ValueKind::Bool, "0") => "false".into(),
        _ => text.to_string(),
    })
}

impl SqliteDriver {
    pub async fn connect(config: &ConnectionConfig) -> Result<Self> {
        let path = config.database.as_deref().filter(|p| !p.is_empty()).ok_or_else(|| Error::Invalid("Choose a database file.".into()))?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(!config.read_only)
            .read_only(config.read_only)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new().max_connections(4).min_connections(0).acquire_timeout(Duration::from_secs(15)).connect_with(options).await?;
        Ok(Self { pool })
    }

    async fn rows(&self, sql: &str) -> Result<Vec<Vec<Cell>>> {
        let mut conn = self.pool.acquire().await?;
        text_rows(&mut conn, sql).await
    }
}

const SCHEMA_SQL: &str = r#"
SELECT 'main', m.name, CASE m.type WHEN 'view' THEN 'VIEW' ELSE 'TABLE' END,
       p.name, p.type, CASE WHEN p."notnull" THEN 'NO' ELSE 'YES' END, NULL
FROM sqlite_master m JOIN pragma_table_info(m.name) p
WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite\_%' ESCAPE '\'
ORDER BY m.name, p.cid
"#;

#[async_trait]
impl DbDriver for SqliteDriver {
    async fn server_version(&self) -> Result<String> {
        Ok(format!("SQLite {}", first_cell(self.rows("SELECT sqlite_version()").await?).unwrap_or_default()))
    }

    async fn schema(&self) -> Result<SchemaSnapshot> {
        Ok(SchemaSnapshot::from_rows(Some("main".into()), self.rows(SCHEMA_SQL).await?))
    }

    async fn execute(&self, sql: &str, on_session: &(dyn Fn(u64) + Send + Sync), sink: &Sink<'_>) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        // No server session to cancel; `SELECT NULL` reports none.
        run(&mut conn, "SELECT NULL", sql, on_session, sink).await
    }

    async fn cancel(&self, _session_id: u64) -> Result<()> {
        Ok(())
    }

    async fn close(&self) {
        self.pool.close().await;
    }

    fn dialect(&self) -> Dialect {
        Dialect::SQLITE
    }

    async fn table_details(&self, _schema: Option<&str>, table: &str) -> Result<TableDetails> {
        let d = Dialect::SQLITE;
        let lit = d.string(table);
        let get = |row: &Vec<Cell>, i: usize| row.get(i).cloned().flatten();

        let kind = self.rows(&format!("SELECT type FROM sqlite_master WHERE name = {lit} AND type IN ('table', 'view')")).await?;
        let kind = kind.first().and_then(|r| get(r, 0)).ok_or_else(|| Error::Invalid(format!("Table {table} was not found")))?;

        let cols = self.rows(&format!("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info({lit}) ORDER BY cid")).await?;
        let pk_count = cols.iter().filter(|r| get(r, 4).is_some_and(|p| p != "0")).count();
        let columns = cols
            .iter()
            .map(|r| {
                let name = get(r, 0).unwrap_or_default();
                let data_type = get(r, 1).unwrap_or_default();
                let primary_key = get(r, 4).is_some_and(|p| p != "0");
                ColumnDesign {
                    original: Some(name.clone()),
                    name,
                    // A lone INTEGER PRIMARY KEY is the rowid: numbered automatically.
                    auto_increment: primary_key && pk_count == 1 && data_type.eq_ignore_ascii_case("INTEGER"),
                    data_type,
                    nullable: get(r, 2).as_deref() != Some("1") && !primary_key,
                    default: get(r, 3),
                    primary_key,
                    comment: None,
                    generated: false,
                    extra: None,
                    enum_values: vec![],
                }
            })
            .collect();

        let index_rows = self.rows(&format!("SELECT name, \"unique\", origin FROM pragma_index_list({lit}) WHERE origin <> 'pk' ORDER BY name")).await?;
        let mut indexes = Vec::new();
        for r in &index_rows {
            let name = get(r, 0).unwrap_or_default();
            let cols = self
                .rows(&format!("SELECT group_concat(name, char(31)) FROM (SELECT name FROM pragma_index_info({}) ORDER BY seqno)", d.string(&name)))
                .await?;
            indexes.push(IndexDesign {
                original: Some(name.clone()),
                name,
                columns: first_cell(cols).map(|s| split_list(&s)).unwrap_or_default(),
                unique: get(r, 1).as_deref() == Some("1"),
                is_constraint: get(r, 2).as_deref() == Some("u"),
            });
        }

        let fk_rows = self.rows(&format!("SELECT id, \"table\", \"from\", \"to\", on_delete, on_update FROM pragma_foreign_key_list({lit}) ORDER BY id, seq")).await?;
        let mut foreign_keys: Vec<ForeignKeyDesign> = Vec::new();
        for r in &fk_rows {
            let name = format!("fk_{}", get(r, 0).unwrap_or_default());
            match foreign_keys.last_mut().filter(|f| f.name == name) {
                Some(f) => {
                    f.columns.push(get(r, 2).unwrap_or_default());
                    f.ref_columns.push(get(r, 3).unwrap_or_default());
                }
                None => foreign_keys.push(ForeignKeyDesign {
                    original: Some(name.clone()),
                    name,
                    columns: vec![get(r, 2).unwrap_or_default()],
                    ref_schema: None,
                    ref_table: get(r, 1).unwrap_or_default(),
                    ref_columns: vec![get(r, 3).unwrap_or_default()],
                    on_delete: FkAction::parse(&get(r, 4).unwrap_or_default()),
                    on_update: FkAction::parse(&get(r, 5).unwrap_or_default()),
                }),
            }
        }

        Ok(TableDetails {
            schema: Some("main".into()),
            design: TableDesign { name: table.to_string(), columns, indexes, foreign_keys, primary_key_name: None },
            is_view: kind == "view",
            row_estimate: None,
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

#[cfg(test)]
mod tests {
    #[test]
    fn affinity_kinds() {
        use super::kind_of_name;
        use crate::types::ValueKind::*;
        assert_eq!(kind_of_name("INTEGER"), Number);
        assert_eq!(kind_of_name("varchar(20)"), Text);
        assert_eq!(kind_of_name("BOOLEAN"), Bool);
        assert_eq!(kind_of_name("DATETIME"), Temporal);
        assert_eq!(kind_of_name("blob"), Binary);
    }
}
