//! AI that works with SQL: answering a question with a query (and a chart in mind), writing or
//! fixing SQL in the editor, and explaining a query in plain words. Only the database's structure
//! (table and column names and types) is sent, never row data.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::{complete_json_pub as complete_json, AiProvider};
use crate::dialect::Dialect;
use crate::dml;
use crate::error::{Error, Result};
use crate::types::SchemaSnapshot;

/// Characters of schema description sent at most; large databases send the most relevant tables.
const SCHEMA_BUDGET: usize = 40_000;

fn engine(d: Dialect) -> &'static str {
    if d.is_mysql() {
        "MySQL"
    } else if d.is_sqlite() {
        "SQLite"
    } else if d.is_sqlserver() {
        "SQL Server (T-SQL: TOP instead of LIMIT, [brackets] for names)"
    } else {
        "PostgreSQL"
    }
}

/// How to cap a list in this dialect, for the instructions.
fn limit_hint(d: Dialect) -> &'static str {
    if d.is_sqlserver() { "SELECT TOP 100" } else { "LIMIT 100" }
}

/// `schema.table(column type, …)` lines, tables mentioned in `hint` first, within the budget.
pub fn describe_schema(snapshot: &SchemaSnapshot, hint: &str) -> String {
    let hint = hint.to_lowercase();
    let qualify = snapshot.schemas.len() > 1;
    let mut tables: Vec<(bool, String)> = Vec::new();
    for sc in &snapshot.schemas {
        for t in &sc.tables {
            let name = if qualify && Some(&sc.name) != snapshot.default_schema.as_ref() { format!("{}.{}", sc.name, t.name) } else { t.name.clone() };
            let cols: Vec<String> = t.columns.iter().map(|c| format!("{} {}", c.name, c.data_type)).collect();
            let kind = if matches!(t.kind, crate::types::TableKind::View) { " (view)" } else { "" };
            let mentioned = hint.contains(&t.name.to_lowercase()) || hint.contains(t.name.trim_end_matches('s'));
            tables.push((mentioned, format!("{name}{kind}({})", cols.join(", "))));
        }
    }
    tables.sort_by_key(|(mentioned, _)| !*mentioned);
    let mut out = String::new();
    let mut left_out = 0;
    for (_, line) in tables {
        if out.len() + line.len() > SCHEMA_BUDGET {
            left_out += 1;
            continue;
        }
        out.push_str(&line);
        out.push('\n');
    }
    if left_out > 0 {
        out.push_str(&format!("({left_out} more tables not listed)\n"));
    }
    out
}

fn single_statement(d: Dialect, sql: &str) -> Result<String> {
    let sql = sql.trim().trim_end_matches(';').trim().to_string();
    if sql.is_empty() {
        return Err(Error::Invalid("The AI didn't write any SQL.".into()));
    }
    if dml::statement_count(d, &sql) > 1 {
        return Err(Error::Invalid("The AI wrote more than one statement, so Kiyi didn't run it. Try asking for one thing at a time.".into()));
    }
    Ok(sql)
}

// ---- asking a question

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartHint {
    /// "bar", "line", "pie" or "number".
    pub kind: String,
    pub x: Option<String>,
    pub y: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskResult {
    pub sql: String,
    /// One sentence, in the person's language, saying what the result shows.
    pub explanation: String,
    pub chart: Option<ChartHint>,
}

fn ask_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["sql", "explanation", "chart"],
        "properties": {
            "sql": { "type": "string" },
            "explanation": { "type": "string" },
            "chart": {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "x", "y"],
                "properties": {
                    "kind": { "type": "string", "enum": ["bar", "line", "pie", "number", "none"] },
                    "x": { "type": "string" },
                    "y": { "type": "array", "items": { "type": "string" } }
                }
            }
        }
    })
}

/// Answers a question about the data with one read-only query, plus how to chart its result.
pub async fn ask(p: &AiProvider, key: Option<&str>, d: Dialect, snapshot: &SchemaSnapshot, question: &str, today: &str) -> Result<AskResult> {
    let system = format!(
        "You answer questions about a {engine} database by writing ONE read-only SQL query (SELECT or WITH … SELECT) \
         that a person can run to see the answer. The person may not know SQL and may write in any language.\n\n\
         Use only tables and columns from the schema. Prefer readable column aliases in the person's language. \
         Aggregate when the question asks for totals, counts, rankings or trends; order results meaningfully; \
         add {limit} to lists unless the question asks for everything. Never modify data. \
         Write dates relative to today when the question says \"last month\" and similar.\n\n\
         `explanation`: one short sentence in the person's language saying what the result shows.\n\
         `chart`: how to draw the result. kind: line for trends over time, bar for comparing categories, \
         pie only for parts of a whole with at most 6 parts, number for a single value, none for plain lists. \
         x: the result column for categories or time (empty for number/none). y: the result columns with the numbers.\n\n\
         If the question can't be answered from this schema, return an empty sql and say why in explanation.",
        engine = engine(d),
        limit = limit_hint(d)
    );
    let user = format!("Schema:\n{}\nToday is {today}.\n\nQuestion: {question}", describe_schema(snapshot, question));
    let text = complete_json(p, key, &system, &user, &ask_schema()).await?;
    #[derive(Deserialize)]
    struct Raw {
        sql: String,
        explanation: String,
        chart: Option<ChartHint>,
    }
    let raw: Raw = serde_json::from_str(&text).map_err(|e| Error::Invalid(format!("The AI answer was not understood: {e}")))?;
    if raw.sql.trim().is_empty() {
        return Err(Error::Invalid(if raw.explanation.trim().is_empty() { "The AI couldn't answer that from this database.".into() } else { raw.explanation }));
    }
    let sql = single_statement(d, &raw.sql)?;
    // The query runs as written, so it must only read.
    let check = dml::check_script(d, &sql);
    let first = sql.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    if check.writes || !matches!(first.as_str(), "SELECT" | "WITH" | "(") {
        return Err(Error::Invalid("The AI wrote a query that would change data, so Kiyi didn't run it. Try asking differently.".into()));
    }
    let chart = raw.chart.filter(|c| c.kind != "none");
    Ok(AskResult { sql, explanation: raw.explanation, chart })
}

// ---- the SQL editor

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlSuggestion {
    pub sql: String,
    pub explanation: String,
    /// The SQL changes data or structure (the editor reviews it on production anyway).
    pub writes: bool,
}

fn suggestion_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["sql", "explanation"],
        "properties": { "sql": { "type": "string" }, "explanation": { "type": "string" } }
    })
}

async fn suggest(p: &AiProvider, key: Option<&str>, d: Dialect, system: String, user: String) -> Result<SqlSuggestion> {
    let text = complete_json(p, key, &system, &user, &suggestion_schema()).await?;
    #[derive(Deserialize)]
    struct Raw {
        sql: String,
        explanation: String,
    }
    let raw: Raw = serde_json::from_str(&text).map_err(|e| Error::Invalid(format!("The AI answer was not understood: {e}")))?;
    if raw.sql.trim().is_empty() {
        return Err(Error::Invalid(if raw.explanation.trim().is_empty() { "The AI didn't write any SQL.".into() } else { raw.explanation }));
    }
    let sql = raw.sql.trim().to_string();
    Ok(SqlSuggestion { writes: dml::check_script(d, &sql).writes, sql, explanation: raw.explanation })
}

/// Writes SQL for an instruction ("add a column for phone numbers", "orders per city"), given the
/// editor's current SQL for context.
pub async fn write_sql(p: &AiProvider, key: Option<&str>, d: Dialect, snapshot: &SchemaSnapshot, instruction: &str, current: &str, today: &str) -> Result<SqlSuggestion> {
    let system = format!(
        "You write {engine} SQL for a person working in a SQL editor. Return the SQL that does what they ask, \
         using only tables and columns from the schema. Prefer reading queries; only write statements that change \
         data or structure when that is clearly what's asked. Format the SQL readably. \
         `explanation`: one short sentence in the person's language. If it can't be done, return empty sql and say why.",
        engine = engine(d)
    );
    let mut user = format!("Schema:\n{}\nToday is {today}.\n\n", describe_schema(snapshot, &format!("{instruction} {current}")));
    if !current.trim().is_empty() {
        user.push_str(&format!("Current SQL in the editor:\n{current}\n\n"));
    }
    user.push_str(&format!("Request: {instruction}"));
    suggest(p, key, d, system, user).await
}

/// Rewrites a failed statement using the database's error message.
pub async fn fix_sql(p: &AiProvider, key: Option<&str>, d: Dialect, snapshot: &SchemaSnapshot, sql: &str, error: &str) -> Result<SqlSuggestion> {
    let system = format!(
        "A {engine} statement failed. Return the corrected statement, changing as little as possible and keeping its intent. \
         Use only tables and columns from the schema. `explanation`: one short sentence on what was wrong.",
        engine = engine(d)
    );
    let user = format!("Schema:\n{}\nStatement:\n{sql}\n\nError:\n{error}", describe_schema(snapshot, sql));
    suggest(p, key, d, system, user).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryExplanation {
    /// A short paragraph on what the query returns or does.
    pub summary: String,
    /// The query's parts in plain words, in order.
    pub steps: Vec<String>,
    /// Anything risky or slow worth knowing (changes data, missing WHERE, SELECT * on a big table…).
    pub warnings: Vec<String>,
}

/// What a query does, in plain words, in the language of `language_hint` (the UI's or the person's).
pub async fn explain_sql(p: &AiProvider, key: Option<&str>, d: Dialect, snapshot: &SchemaSnapshot, sql: &str, language_hint: &str) -> Result<QueryExplanation> {
    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "steps", "warnings"],
        "properties": {
            "summary": { "type": "string" },
            "steps": { "type": "array", "items": { "type": "string" } },
            "warnings": { "type": "array", "items": { "type": "string" } }
        }
    });
    let system = format!(
        "Explain a {engine} query to someone who doesn't read SQL. `summary`: two sentences at most on what it returns or changes. \
         `steps`: its parts in order, one short plain sentence each. `warnings`: only real concerns (it changes or deletes data, \
         an UPDATE/DELETE without WHERE, a join that can multiply rows, reading a whole large table); empty when there are none. \
         Write in {language}.",
        engine = engine(d),
        language = language_hint
    );
    let user = format!("Schema:\n{}\nQuery:\n{sql}", describe_schema(snapshot, sql));
    let text = complete_json(p, key, &system, &user, &schema).await?;
    #[derive(Deserialize)]
    struct Raw {
        summary: String,
        steps: Vec<String>,
        warnings: Vec<String>,
    }
    let raw: Raw = serde_json::from_str(&text).map_err(|e| Error::Invalid(format!("The AI answer was not understood: {e}")))?;
    Ok(QueryExplanation { summary: raw.summary, steps: raw.steps, warnings: raw.warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ColumnInfo, SchemaInfo, TableInfo, TableKind};

    fn snapshot() -> SchemaSnapshot {
        let col = |n: &str, t: &str| ColumnInfo { name: n.into(), data_type: t.into(), nullable: true };
        SchemaSnapshot {
            default_schema: Some("public".into()),
            schemas: vec![SchemaInfo {
                name: "public".into(),
                tables: vec![
                    TableInfo { name: "customers".into(), kind: TableKind::Table, columns: vec![col("id", "bigint"), col("email", "text")], row_estimate: None },
                    TableInfo { name: "orders".into(), kind: TableKind::Table, columns: vec![col("id", "bigint"), col("total", "numeric")], row_estimate: None },
                ],
            }],
        }
    }

    #[test]
    fn mentioned_tables_come_first() {
        let text = describe_schema(&snapshot(), "how many orders last month?");
        assert!(text.starts_with("orders(id bigint, total numeric)"), "{text}");
        assert!(text.contains("customers(id bigint, email text)"));
    }
}
