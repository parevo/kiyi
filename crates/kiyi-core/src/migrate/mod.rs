//! Moving data from one database into another whose tables look different: another company's
//! system into yours, an old schema into a new one, PostgreSQL into MySQL.
//!
//! A [`MigrationPlan`] says, for each target table, which source table feeds it and how each
//! target column gets its value: a source column (cleaned up by a few readable steps), several
//! combined, a fixed value, or the new key of a row moved from another table. Kiyi suggests a
//! plan from names, types and foreign keys (and the AI can improve it); the person reviews it;
//! a check previews real rows; and the move runs in one transaction, so it either all lands or
//! nothing does. Nothing in a plan is code: every step is one of a fixed list.

mod automap;
mod convert;
mod engine;
mod suggest_ai;

use serde::{Deserialize, Serialize};

pub use automap::suggest;
pub use engine::{check, run, MigrationCheck, MigrationReport, Progress, Stage, TableCheck, TableResult};
pub use suggest_ai::improve_with_ai;


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlan {
    pub source: String,
    pub target: String,
    /// Shown when the plan is opened again, possibly on another computer.
    #[serde(default)]
    pub source_name: String,
    #[serde(default)]
    pub target_name: String,
    pub tables: Vec<TableMapping>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableMapping {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub source_schema: Option<String>,
    pub source_table: String,
    pub target_schema: Option<String>,
    pub target_table: String,
    pub columns: Vec<ColumnMapping>,
    #[serde(default)]
    pub ids: IdMode,
    #[serde(default)]
    pub write: WriteMode,
    /// Target columns that identify an existing row, for `Skip` and `Update`.
    #[serde(default)]
    pub match_on: Vec<String>,
}

fn yes() -> bool {
    true
}

impl TableMapping {
    pub fn source_key(&self) -> String {
        table_key(self.source_schema.as_deref(), &self.source_table)
    }
}

/// How a table is named inside a plan: `schema.table`, or the table alone without a schema.
pub fn table_key(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(s) if !s.is_empty() => format!("{s}.{table}"),
        _ => table.to_string(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdMode {
    /// The key gets whatever its mapping says, or the database's own next number when it isn't mapped.
    #[default]
    Keep,
    /// Kiyi numbers the rows after the highest key already in the table, and remembers which
    /// source row got which number so references to it can follow.
    Renumber,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteMode {
    /// Add every row; a row that clashes with an existing one stops the move.
    #[default]
    Insert,
    /// Leave rows that already exist (by `match_on`) as they are.
    Skip,
    /// Overwrite rows that already exist (by `match_on`) with the moved values.
    Update,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMapping {
    pub target: String,
    pub source: ValueSource,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ValueSource {
    Column { column: String },
    /// Several columns joined, empty ones left out: first + " " + last.
    Combine { columns: Vec<String>, separator: String },
    /// The same value (or NULL) in every row.
    Fixed { value: Option<String> },
    /// The key, in the target, of the row moved from `table` whose source key is in `column`.
    #[serde(rename_all = "camelCase")]
    Reference { column: String, schema: Option<String>, table: String },
    /// Nothing written: the column's default, the database's next number, or NULL.
    Default,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Step {
    Trim,
    Lower,
    Upper,
    Replace { find: String, with: String },
    /// One part of the text split at `separator`, counting from 1; with `rest`, that part and
    /// everything after it ("Ada María Lovelace" → part 2 with rest = "María Lovelace").
    Split { separator: String, part: usize, #[serde(default)] rest: bool },
    /// Exact values replaced by others ("A" → "active"); anything else follows `otherwise`.
    Map { pairs: Vec<MapPair>, #[serde(default)] otherwise: Otherwise },
    /// A value for rows where it's missing or blank.
    IfEmpty { value: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPair {
    pub from: String,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Otherwise {
    #[default]
    Keep,
    Null,
    Value { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The move can't start until this is fixed.
    Error,
    /// Worth a look; the move can still run.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub severity: Severity,
    /// The target table, or empty for the plan as a whole.
    pub table: String,
    pub column: Option<String>,
    pub message: String,
}

impl Issue {
    fn error(table: &str, column: Option<&str>, message: impl Into<String>) -> Self {
        Self { severity: Severity::Error, table: table.into(), column: column.map(str::to_string), message: message.into() }
    }
    fn warning(table: &str, column: Option<&str>, message: impl Into<String>) -> Self {
        Self { severity: Severity::Warning, table: table.into(), column: column.map(str::to_string), message: message.into() }
    }
}

pub fn save(plan: &MigrationPlan, path: &std::path::Path) -> crate::error::Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({ "kiyi": "migration", "version": 1, "plan": plan }))?)?;
    Ok(())
}

pub fn open(path: &std::path::Path) -> crate::error::Result<MigrationPlan> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if v["kiyi"] != "migration" {
        return Err(crate::error::Error::Invalid("This file isn't a Kiyi migration plan.".into()));
    }
    Ok(serde_json::from_value(v["plan"].clone())?)
}
