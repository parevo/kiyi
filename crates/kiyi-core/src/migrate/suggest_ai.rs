//! Asking the AI to improve a plan: it sees both databases' structure (and, only when the person
//! allows it, a few example values) and answers with a plan in the same fixed vocabulary of steps.
//! Whatever it returns is checked against the real tables and columns; anything that doesn't
//! exist is dropped and the earlier mapping kept.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{json, Value};

use super::automap::norm;
use super::{table_key, ColumnMapping, IdMode, MapPair, MigrationPlan, Otherwise, Step, TableMapping, ValueSource, WriteMode};
use crate::ai::AiProvider;
use crate::design::TableDetails;
use crate::drivers::DbDriver;
use crate::error::{Error, Result};

const MAX_TABLES: usize = 80;
const MAX_COLUMNS: usize = 60;
const EXAMPLE_LEN: usize = 40;

struct Table {
    schema: Option<String>,
    name: String,
    details: TableDetails,
    examples: HashMap<String, Vec<String>>,
}

async fn tables(driver: &dyn DbDriver, include_views: bool, with_examples: bool) -> Result<Vec<Table>> {
    let snap = driver.schema().await?;
    let d = driver.dialect();
    let sqlite = d.is_sqlite();
    let mut out = Vec::new();
    for s in &snap.schemas {
        for t in &s.tables {
            if out.len() >= MAX_TABLES {
                break;
            }
            if !include_views && t.kind == crate::types::TableKind::View {
                continue;
            }
            let schema = (!sqlite).then(|| s.name.clone());
            let details = driver.table_details(schema.as_deref(), &t.name).await?;
            let mut examples: HashMap<String, Vec<String>> = HashMap::new();
            if with_examples {
                let sql = if d.is_sqlserver() { format!("SELECT TOP 3 * FROM {}", d.table(schema.as_deref(), &t.name)) } else { format!("SELECT * FROM {} LIMIT 3", d.table(schema.as_deref(), &t.name)) };
                if let Ok((cols, rows)) = driver.fetch(&sql).await {
                    for (i, c) in cols.iter().enumerate() {
                        let vals: Vec<String> = rows.iter().filter_map(|r| r.get(i).cloned().flatten()).map(|v| v.chars().take(EXAMPLE_LEN).collect()).collect();
                        examples.insert(c.name.clone(), vals);
                    }
                }
            }
            out.push(Table { schema, name: t.name.clone(), details, examples });
        }
    }
    Ok(out)
}

fn describe(tables: &[Table], target: bool) -> String {
    let mut out = String::new();
    for t in tables {
        out.push_str(&format!("- {}\n", table_key(t.schema.as_deref(), &t.name)));
        for c in t.details.design.columns.iter().take(MAX_COLUMNS) {
            let mut notes = vec![c.data_type.clone()];
            if c.primary_key {
                notes.push(if c.auto_increment { "key, auto-numbered".into() } else { "key".into() });
            }
            if target && !c.nullable && c.default.is_none() && !c.auto_increment && !c.generated {
                notes.push("required".into());
            }
            if c.generated {
                notes.push("computed, never written".into());
            }
            if !c.enum_values.is_empty() {
                notes.push(format!("one of: {}", c.enum_values.join(", ")));
            }
            if let Some(f) = t.details.design.foreign_keys.iter().find(|f| f.columns.len() == 1 && f.columns[0] == c.name) {
                notes.push(format!("links to {}.{}", f.ref_table, f.ref_columns.join(",")));
            }
            if let Some(ex) = t.examples.get(&c.name).filter(|e| !e.is_empty()) {
                notes.push(format!("e.g. {}", ex.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>().join(", ")));
            }
            out.push_str(&format!("    {} ({})\n", c.name, notes.join("; ")));
        }
    }
    out
}

fn schema() -> Value {
    let s = |t: &str| json!({ "type": t });
    let step = json!({
        "type": "object", "additionalProperties": false,
        "required": ["op", "find", "with", "separator", "part", "rest", "pairs", "otherwise", "value"],
        "properties": {
            "op": { "type": "string", "enum": ["trim", "lower", "upper", "replace", "split", "map", "ifEmpty"] },
            "find": s("string"), "with": s("string"), "separator": s("string"), "part": s("integer"), "rest": s("boolean"),
            "pairs": { "type": "array", "items": { "type": "object", "additionalProperties": false, "required": ["from", "to"], "properties": { "from": s("string"), "to": { "type": ["string", "null"] } } } },
            "otherwise": { "type": "string", "enum": ["keep", "null", "value"] },
            "value": { "type": ["string", "null"] }
        }
    });
    let column = json!({
        "type": "object", "additionalProperties": false,
        "required": ["target", "from", "columns", "separator", "value", "table", "steps"],
        "properties": {
            "target": s("string"),
            "from": { "type": "string", "enum": ["column", "combine", "fixed", "reference", "default"] },
            "columns": { "type": "array", "items": s("string") },
            "separator": s("string"),
            "value": { "type": ["string", "null"] },
            "table": s("string"),
            "steps": { "type": "array", "items": step }
        }
    });
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["tables", "explanation"],
        "properties": {
            "explanation": s("string"),
            "tables": { "type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["sourceTable", "targetTable", "ids", "columns"],
                "properties": {
                    "sourceTable": s("string"), "targetTable": s("string"),
                    "ids": { "type": "string", "enum": ["keep", "renumber"] },
                    "columns": { "type": "array", "items": column }
                }
            }}
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPlan {
    tables: Vec<RawTable>,
    explanation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTable {
    source_table: String,
    target_table: String,
    ids: String,
    columns: Vec<RawColumn>,
}

#[derive(Deserialize)]
struct RawColumn {
    target: String,
    from: String,
    #[serde(default)]
    columns: Vec<String>,
    #[serde(default)]
    separator: String,
    value: Option<String>,
    #[serde(default)]
    table: String,
    #[serde(default)]
    steps: Vec<RawStep>,
}

#[derive(Deserialize)]
struct RawStep {
    op: String,
    #[serde(default)]
    find: String,
    #[serde(default)]
    with: String,
    #[serde(default)]
    separator: String,
    #[serde(default)]
    part: usize,
    #[serde(default)]
    rest: bool,
    #[serde(default)]
    pairs: Vec<RawPair>,
    #[serde(default)]
    otherwise: String,
    value: Option<String>,
}

#[derive(Deserialize)]
struct RawPair {
    from: String,
    to: Option<String>,
}

fn find_table<'a>(tables: &'a [Table], name: &str) -> Option<&'a Table> {
    tables.iter().find(|t| table_key(t.schema.as_deref(), &t.name) == name).or_else(|| tables.iter().find(|t| t.name.eq_ignore_ascii_case(name) || norm(&table_key(t.schema.as_deref(), &t.name)) == norm(name)))
}

fn step(r: &RawStep) -> Option<Step> {
    Some(match r.op.as_str() {
        "trim" => Step::Trim,
        "lower" => Step::Lower,
        "upper" => Step::Upper,
        "replace" => Step::Replace { find: r.find.clone(), with: r.with.clone() },
        "split" => Step::Split { separator: r.separator.clone(), part: r.part.max(1), rest: r.rest },
        "map" => Step::Map {
            pairs: r.pairs.iter().map(|p| MapPair { from: p.from.clone(), to: p.to.clone() }).collect(),
            otherwise: match r.otherwise.as_str() {
                "null" => Otherwise::Null,
                "value" => Otherwise::Value { value: r.value.clone().unwrap_or_default() },
                _ => Otherwise::Keep,
            },
        },
        "ifEmpty" => Step::IfEmpty { value: r.value.clone() },
        _ => return None,
    })
}

/// Turns the AI's answer into a plan, keeping only what exists. Returns the plan and what was dropped.
fn adopt(raw: RawPlan, current: &MigrationPlan, sources: &[Table], targets: &[Table]) -> (MigrationPlan, Vec<String>) {
    let mut dropped = Vec::new();
    let mut tables: Vec<TableMapping> = Vec::new();
    for rt in raw.tables {
        let (Some(st), Some(tt)) = (find_table(sources, &rt.source_table), find_table(targets, &rt.target_table)) else {
            dropped.push(format!("{} → {} (no such table)", rt.source_table, rt.target_table));
            continue;
        };
        let existing = current.tables.iter().find(|m| m.target_table == tt.name && m.target_schema == tt.schema);
        let mut columns: Vec<ColumnMapping> = Vec::new();
        for tc in &tt.details.design.columns {
            let has = |c: &str| st.details.design.columns.iter().any(|x| x.name == c);
            let earlier = existing.and_then(|m| m.columns.iter().find(|c| c.target == tc.name)).cloned();
            let Some(rc) = rt.columns.iter().find(|c| c.target == tc.name) else {
                columns.push(earlier.unwrap_or(ColumnMapping { target: tc.name.clone(), source: ValueSource::Default, steps: vec![] }));
                continue;
            };
            let source = match rc.from.as_str() {
                "column" if rc.columns.first().is_some_and(|c| has(c)) => Some(ValueSource::Column { column: rc.columns[0].clone() }),
                "combine" if !rc.columns.is_empty() && rc.columns.iter().all(|c| has(c)) => Some(ValueSource::Combine { columns: rc.columns.clone(), separator: rc.separator.clone() }),
                "fixed" => Some(ValueSource::Fixed { value: rc.value.clone() }),
                "reference" if rc.columns.first().is_some_and(|c| has(c)) => find_table(sources, &rc.table).map(|r| ValueSource::Reference { column: rc.columns[0].clone(), schema: r.schema.clone(), table: r.name.clone() }),
                "default" => Some(ValueSource::Default),
                _ => None,
            };
            match source {
                Some(source) => columns.push(ColumnMapping { target: tc.name.clone(), source, steps: rc.steps.iter().filter_map(step).collect() }),
                None => {
                    dropped.push(format!("{}.{}", tt.name, tc.name));
                    columns.push(earlier.unwrap_or(ColumnMapping { target: tc.name.clone(), source: ValueSource::Default, steps: vec![] }));
                }
            }
        }
        tables.push(TableMapping {
            enabled: existing.map(|m| m.enabled).unwrap_or(true),
            source_schema: st.schema.clone(),
            source_table: st.name.clone(),
            target_schema: tt.schema.clone(),
            target_table: tt.name.clone(),
            columns,
            ids: if rt.ids == "renumber" { IdMode::Renumber } else { IdMode::Keep },
            write: existing.map(|m| m.write).unwrap_or(WriteMode::Insert),
            match_on: existing.map(|m| m.match_on.clone()).unwrap_or_else(|| tt.details.design.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect()),
        });
    }
    // Tables the AI left out stay as they were.
    for m in &current.tables {
        if !tables.iter().any(|t| t.target_table == m.target_table && t.target_schema == m.target_schema) {
            tables.push(m.clone());
        }
    }
    (MigrationPlan { tables, ..current.clone() }, dropped)
}

fn engine(d: &dyn DbDriver) -> &'static str {
    match d.dialect().kind {
        crate::config::DbKind::Postgres => "PostgreSQL",
        crate::config::DbKind::Mysql => "MySQL/MariaDB",
        crate::config::DbKind::Sqlite => "SQLite",
        crate::config::DbKind::Sqlserver => "SQL Server",
    }
}

/// A better plan from the AI, and its one-paragraph explanation.
pub async fn improve_with_ai(p: &AiProvider, key: Option<&str>, source: &dyn DbDriver, target: &dyn DbDriver, current: &MigrationPlan, examples: bool) -> Result<(MigrationPlan, String)> {
    let sources = tables(source, true, examples).await?;
    let targets = tables(target, false, false).await?;
    let system = "You plan a data migration between two relational databases whose tables differ. For each target table worth filling, \
        pick the source table it comes from and say how every target column gets its value.\n\n\
        `from`: column (copy `columns[0]`), combine (join `columns` with `separator`, empty values skipped), fixed (`value` in every row, null for NULL), \
        reference (the new key of the row moved from source table `table` whose key is in `columns[0]`; use it for every link/foreign key so links survive renumbering), \
        or default (write nothing: the column's default, an auto-number or NULL).\n\
        `steps` clean the value in order: trim, lower, upper, replace (`find` → `with`), split (`part` counted from 1 of the text split at `separator`; `rest` keeps that part and everything after), \
        map (exact `pairs`; anything else follows `otherwise`: keep, null, or value), ifEmpty (`value` when missing or blank). Unused fields: empty string, 0, false, [] or null.\n\
        `ids`: renumber when the target key is auto-numbered and the source key doesn't fit it (different type, or rows already in the target); keep otherwise.\n\n\
        Rules: use only tables and columns listed. Every required target column needs a value. Never map a computed column. \
        Map values to the target's allowed values with map steps when they differ (\"A\" → \"active\"). Leave out target tables with no sensible source. \
        `explanation`: two or three short sentences, in plain words, on what moves where and anything the person should check.";
    let user = format!(
        "Source database ({}):\n{}\nTarget database ({}):\n{}\nCurrent plan, to improve:\n{}",
        engine(source),
        describe(&sources, false),
        engine(target),
        describe(&targets, true),
        serde_json::to_string(&current.tables)?
    );
    let text = crate::ai::complete_json_pub(p, key, system, &user, &schema()).await?;
    let raw: RawPlan = serde_json::from_str(&text).map_err(|e| Error::Invalid(format!("The AI answer was not understood: {e}")))?;
    let explanation = raw.explanation.clone();
    let (plan, dropped) = adopt(raw, current, &sources, &targets);
    let note = if dropped.is_empty() { String::new() } else { format!(" Kept the earlier mapping for: {} (the AI named things that don't exist).", dropped.join(", ")) };
    Ok((plan, format!("{explanation}{note}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{ColumnDesign, TableDesign};

    fn table(name: &str, cols: &[&str]) -> Table {
        let columns = cols
            .iter()
            .map(|c| ColumnDesign { original: None, name: c.to_string(), data_type: "text".into(), nullable: true, default: None, primary_key: *c == "id", auto_increment: false, comment: None, generated: false, extra: None, enum_values: vec![] })
            .collect();
        Table { schema: None, name: name.into(), details: TableDetails { schema: None, design: TableDesign { name: name.into(), columns, indexes: vec![], foreign_keys: vec![], primary_key_name: None }, is_view: false, row_estimate: None }, examples: HashMap::new() }
    }

    #[test]
    fn the_ai_answer_is_kept_only_where_it_exists() {
        let sources = vec![table("clients", &["id", "full_name", "state"])];
        let targets = vec![table("customers", &["id", "first_name", "last_name", "status", "notes"])];
        let current = MigrationPlan { source: "s".into(), target: "t".into(), source_name: String::new(), target_name: String::new(), tables: vec![] };
        let raw: RawPlan = serde_json::from_value(json!({
            "explanation": "Clients become customers.",
            "tables": [{ "sourceTable": "clients", "targetTable": "Customers", "ids": "renumber", "columns": [
                { "target": "first_name", "from": "column", "columns": ["full_name"], "separator": "", "value": null, "table": "", "steps": [
                    { "op": "split", "find": "", "with": "", "separator": " ", "part": 1, "rest": false, "pairs": [], "otherwise": "keep", "value": null } ] },
                { "target": "status", "from": "column", "columns": ["state"], "separator": "", "value": null, "table": "", "steps": [
                    { "op": "map", "find": "", "with": "", "separator": "", "part": 0, "rest": false, "pairs": [{ "from": "A", "to": "active" }], "otherwise": "null", "value": null } ] },
                { "target": "notes", "from": "column", "columns": ["invented"], "separator": "", "value": null, "table": "", "steps": [] },
                { "target": "nonexistent", "from": "fixed", "columns": [], "separator": "", "value": "x", "table": "", "steps": [] }
            ]}, { "sourceTable": "ghosts", "targetTable": "customers", "ids": "keep", "columns": [] }]
        }))
        .unwrap();
        let (plan, dropped) = adopt(raw, &current, &sources, &targets);
        assert_eq!(plan.tables.len(), 1);
        let t = &plan.tables[0];
        assert_eq!((t.source_table.as_str(), t.target_table.as_str(), t.ids), ("clients", "customers", IdMode::Renumber));
        assert_eq!(t.columns.len(), 5, "one mapping per target column");
        let col = |n: &str| t.columns.iter().find(|c| c.target == n).unwrap();
        assert_eq!(col("first_name").steps, vec![Step::Split { separator: " ".into(), part: 1, rest: false }]);
        assert!(matches!(&col("status").steps[0], Step::Map { otherwise: Otherwise::Null, .. }));
        assert_eq!(col("notes").source, ValueSource::Default, "an invented source column is dropped");
        assert!(dropped.iter().any(|d| d == "customers.notes") && dropped.iter().any(|d| d.contains("ghosts")), "{dropped:?}");
    }
}
