//! Natural-language filtering: "paid orders over 100 from last week" → structured filters.
//!
//! Calls the Claude Messages API over HTTP (Rust has no official SDK) with structured
//! outputs, so the reply is always JSON matching our schema. Only the table's structure is
//! sent — never row data.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::design::TableDetails;
use crate::dialect::Dialect;
use crate::dml::{self, Filter, Sort};
use crate::error::{Error, Result};
use crate::secrets;

pub const MODEL: &str = "claude-opus-5-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";
const KEY_ACCOUNT: &str = "ai:anthropic";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFilterResult {
    pub filters: Vec<Filter>,
    pub sort: Vec<Sort>,
    /// A SQL condition for what the structured filters can't express (OR, date math).
    pub condition: Option<String>,
    /// One sentence describing what is now shown, in the user's language.
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub configured: bool,
    /// "keychain" or "environment".
    pub source: Option<&'static str>,
    pub model: &'static str,
}

fn api_key() -> Result<Option<(String, &'static str)>> {
    if let Some(k) = secrets::get_secret(KEY_ACCOUNT)? {
        return Ok(Some((k, "keychain")));
    }
    Ok(std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty()).map(|k| (k, "environment")))
}

pub fn status() -> Result<AiStatus> {
    let source = api_key()?.map(|(_, s)| s);
    Ok(AiStatus { configured: source.is_some(), source, model: MODEL })
}

pub fn set_key(key: Option<&str>) -> Result<()> {
    secrets::set_secret(KEY_ACCOUNT, key.map(str::trim).filter(|k| !k.is_empty()))
}

const OPS: &[&str] =
    &["eq", "ne", "lt", "gt", "le", "ge", "contains", "notContains", "startsWith", "endsWith", "isNull", "notNull", "in"];

fn system_prompt(d: Dialect) -> String {
    let engine = if d.is_mysql() { "MySQL" } else { "PostgreSQL" };
    format!(
        "You turn a person's request about one database table into filters for a data browser. \
         The person may not know SQL and may write in any language.\n\n\
         Prefer `filters`. Each filter compares one column with a text value; the database converts the \
         text to the column's type. Operators: eq, ne, lt, gt, le, ge (comparisons), contains / notContains / \
         startsWith / endsWith (case-insensitive text match), isNull / notNull (value ignored), \
         in (value is a comma-separated list). Use exact enum values where the column lists them. \
         All filters are combined with AND.\n\n\
         Use `condition` only when filters can't express the request (OR between columns, date arithmetic, \
         functions). It must be a single {engine} boolean expression over this table's columns — no \
         semicolons, no subqueries that modify data. Otherwise leave it empty.\n\n\
         Use `sort` when the request implies an order (\"latest\", \"biggest\").\n\n\
         `explanation` is one short sentence, in the person's language, saying what will be shown. \
         If the request isn't about finding rows in this table, return no filters and say so in the \
         explanation."
    )
}

fn describe_table(details: &TableDetails) -> String {
    let d = &details.design;
    let mut out = format!("Table: {}\nColumns:\n", d.name);
    for c in &d.columns {
        out.push_str(&format!("- {} ({}{})", c.name, c.data_type, if c.nullable { ", nullable" } else { "" }));
        if !c.enum_values.is_empty() {
            out.push_str(&format!(" one of: {}", c.enum_values.join(", ")));
        }
        if let Some(fk) = d.foreign_keys.iter().find(|f| f.columns.len() == 1 && f.columns[0] == c.name) {
            out.push_str(&format!(" → references {}.{}", fk.ref_table, fk.ref_columns.join(",")));
        }
        if let Some(comment) = &c.comment {
            out.push_str(&format!(" — {comment}"));
        }
        out.push('\n');
    }
    out
}

fn output_schema(columns: &[String]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["filters", "sort", "condition", "explanation"],
        "properties": {
            "filters": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["column", "op", "value"],
                    "properties": {
                        "column": { "type": "string", "enum": columns },
                        "op": { "type": "string", "enum": OPS },
                        "value": { "type": "string" }
                    }
                }
            },
            "sort": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["column", "descending"],
                    "properties": {
                        "column": { "type": "string", "enum": columns },
                        "descending": { "type": "boolean" }
                    }
                }
            },
            "condition": { "type": "string" },
            "explanation": { "type": "string" }
        }
    })
}

#[derive(Deserialize)]
struct Raw {
    filters: Vec<Filter>,
    sort: Vec<Sort>,
    condition: String,
    explanation: String,
}

pub async fn filters_from_prompt(d: Dialect, details: &TableDetails, prompt: &str, today: &str) -> Result<AiFilterResult> {
    let (key, _) = api_key()?.ok_or_else(|| Error::Invalid("Add an Anthropic API key in Settings to use AI.".into()))?;
    let columns: Vec<String> = details.design.columns.iter().map(|c| c.name.clone()).collect();

    let body = json!({
        "model": MODEL,
        "max_tokens": 16000,
        // Server-side fallback: if this model declines, the request is retried on a suitable one.
        "fallbacks": "default",
        "output_config": { "effort": "low", "format": { "type": "json_schema", "schema": output_schema(&columns) } },
        "system": system_prompt(d),
        "messages": [{
            "role": "user",
            "content": format!("{}\nToday is {today}.\n\nRequest: {prompt}", describe_table(details)),
        }],
    });

    // reqwest is built without a bundled crypto provider; share sqlx's ring.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| Error::Invalid(e.to_string()))?
        .post(API_URL)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Invalid(format!("Could not reach the AI service: {e}")))?;

    let status = response.status();
    let reply: Value = response.json().await.map_err(|e| Error::Invalid(format!("Unexpected AI response: {e}")))?;
    if !status.is_success() {
        let message = reply["error"]["message"].as_str().unwrap_or("unknown error");
        return Err(Error::Invalid(match status.as_u16() {
            401 => "The Anthropic API key was rejected. Check it in Settings.".into(),
            429 => "The AI service is busy (rate limited). Try again in a moment.".into(),
            _ => format!("AI request failed ({status}): {message}"),
        }));
    }
    if reply["stop_reason"] == "refusal" {
        return Err(Error::Invalid("The AI declined this request. Try rephrasing it.".into()));
    }
    let text = reply["content"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
        .and_then(|b| b["text"].as_str())
        .ok_or_else(|| Error::Invalid("The AI returned no answer.".into()))?;
    let raw: Raw = serde_json::from_str(text).map_err(|e| Error::Invalid(format!("The AI answer was not understood: {e}")))?;

    let condition = Some(raw.condition.trim().to_string()).filter(|c| !c.is_empty());
    if let Some(c) = &condition {
        dml::validate_condition(d, c).map_err(|e| Error::Invalid(format!("The AI wrote a condition Kiyi can't run safely. {e}")))?;
    }
    Ok(AiFilterResult { filters: raw.filters, sort: raw.sort, condition, explanation: raw.explanation })
}
