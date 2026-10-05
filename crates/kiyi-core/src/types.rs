use serde::Serialize;

use crate::error::ErrorInfo;

/// Coarse classification used by the UI to align and render cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueKind {
    Text,
    Number,
    Bool,
    Json,
    Temporal,
    Uuid,
    Binary,
    Array,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    pub kind: ValueKind,
}

/// Every value travels as its canonical text form (what psql / mysql CLI print).
/// This keeps int8 / numeric precision intact in JavaScript. `None` is SQL NULL.
pub type Cell = Option<String>;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum QueryEvent {
    /// A new result set starts.
    #[serde(rename_all = "camelCase")]
    Columns { columns: Vec<ColumnMeta> },
    /// A batch of rows for the current result set.
    Rows { rows: Vec<Vec<Cell>> },
    /// A statement finished.
    #[serde(rename_all = "camelCase")]
    StatementDone { rows_affected: u64 },
    /// The whole run finished.
    #[serde(rename_all = "camelCase")]
    Done { elapsed_ms: u64, cancelled: bool },
    Error { error: ErrorInfo },
}

pub type Sink<'a> = dyn Fn(QueryEvent) + Send + Sync + 'a;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TableKind {
    Table,
    View,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub kind: TableKind,
    pub columns: Vec<ColumnInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaInfo {
    pub name: String,
    pub tables: Vec<TableInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSnapshot {
    /// Schema the session resolves unqualified names against.
    pub default_schema: Option<String>,
    pub schemas: Vec<SchemaInfo>,
}

impl SchemaSnapshot {
    /// Builds the tree from flat `(schema, table, table_type, column, data_type, is_nullable)`
    /// rows that are already ordered by schema, table and column position.
    pub(crate) fn from_rows(default_schema: Option<String>, rows: Vec<Vec<Cell>>) -> Self {
        let mut schemas: Vec<SchemaInfo> = Vec::new();
        for row in rows {
            let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
            let (schema, table) = (get(0), get(1));
            if schemas.last().map(|s| s.name != schema).unwrap_or(true) {
                schemas.push(SchemaInfo { name: schema, tables: Vec::new() });
            }
            let tables = &mut schemas.last_mut().unwrap().tables;
            if tables.last().map(|t| t.name != table).unwrap_or(true) {
                let kind = if get(2).contains("VIEW") { TableKind::View } else { TableKind::Table };
                tables.push(TableInfo { name: table, kind, columns: Vec::new() });
            }
            tables.last_mut().unwrap().columns.push(ColumnInfo {
                name: get(3),
                data_type: get(4),
                nullable: get(5) == "YES",
            });
        }
        SchemaSnapshot { default_schema, schemas }
    }
}
