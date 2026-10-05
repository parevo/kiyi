//! Natural-language filtering with the AI provider the user chose.
//!
//! Two wire protocols cover nearly everything: Anthropic's Messages API (Claude), and the
//! OpenAI-style `/chat/completions` that OpenAI, Gemini, OpenRouter, Groq, Mistral, Ollama,
//! LM Studio and most self-hosted servers speak. Providers are user-configured; presets only
//! pre-fill the form. Only table structure is ever sent, never row data.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::design::TableDetails;
use crate::dialect::Dialect;
use crate::dml::{self, Filter, Sort};
use crate::error::{Error, Result};
use crate::secrets;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    /// Anthropic Messages API.
    Anthropic,
    /// OpenAI-compatible `/chat/completions`.
    OpenAi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// Preset it was created from, for icons, help links and env-var fallback.
    #[serde(default)]
    pub preset: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiSettingsFile {
    providers: Vec<AiProvider>,
    active: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    #[serde(flatten)]
    pub provider: AiProvider,
    /// "keychain", "environment", or `None` (no key; fine for local servers).
    pub key_source: Option<&'static str>,
    pub needs_key: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub providers: Vec<ProviderView>,
    pub active: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub configured: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: ProviderKind,
    pub base_url: &'static str,
    /// Suggested model; empty means "pick one from the list".
    pub default_model: &'static str,
    pub needs_key: bool,
    /// Runs on this machine.
    pub local: bool,
    pub key_url: Option<&'static str>,
    pub env_var: Option<&'static str>,
    pub description: &'static str,
}

pub fn presets() -> Vec<ProviderPreset> {
    use ProviderKind::*;
    let p = |id, name, kind, base_url, default_model, needs_key, local, key_url, env_var, description| ProviderPreset {
        id,
        name,
        kind,
        base_url,
        default_model,
        needs_key,
        local,
        key_url,
        env_var,
        description,
    };
    vec![
        p("anthropic", "Anthropic", Anthropic, "https://api.anthropic.com", "claude-opus-5-5", true, false, Some("https://console.anthropic.com/settings/keys"), Some("ANTHROPIC_API_KEY"), "Claude models"),
        p("openai", "OpenAI", OpenAi, "https://api.openai.com/v1", "", true, false, Some("https://platform.openai.com/api-keys"), Some("OPENAI_API_KEY"), "GPT models"),
        p("gemini", "Google Gemini", OpenAi, "https://generativelanguage.googleapis.com/v1beta/openai", "", true, false, Some("https://aistudio.google.com/apikey"), Some("GEMINI_API_KEY"), "Gemini models"),
        p("openrouter", "OpenRouter", OpenAi, "https://openrouter.ai/api/v1", "", true, false, Some("https://openrouter.ai/keys"), Some("OPENROUTER_API_KEY"), "Hundreds of models with one key"),
        p("groq", "Groq", OpenAi, "https://api.groq.com/openai/v1", "", true, false, Some("https://console.groq.com/keys"), Some("GROQ_API_KEY"), "Very fast open models"),
        p("mistral", "Mistral", OpenAi, "https://api.mistral.ai/v1", "", true, false, Some("https://console.mistral.ai/api-keys"), Some("MISTRAL_API_KEY"), "Mistral models"),
        p("ollama", "Ollama", OpenAi, "http://localhost:11434/v1", "", false, true, None, None, "Models running on this Mac"),
        p("lmstudio", "LM Studio", OpenAi, "http://localhost:1234/v1", "", false, true, None, None, "Models running on this Mac"),
        p("custom", "Custom (OpenAI-compatible)", OpenAi, "", "", false, false, None, None, "Any server with a /chat/completions endpoint"),
    ]
}

fn preset(id: Option<&str>) -> Option<ProviderPreset> {
    id.and_then(|id| presets().into_iter().find(|p| p.id == id))
}

fn account(provider_id: &str) -> String {
    format!("ai:{provider_id}")
}

/// Keychain first, then the provider's conventional environment variable.
fn key_for(p: &AiProvider) -> Result<Option<(String, &'static str)>> {
    if let Some(k) = secrets::get_secret(&account(&p.id))? {
        return Ok(Some((k, "keychain")));
    }
    let env = preset(p.preset.as_deref()).and_then(|pr| pr.env_var);
    Ok(env.and_then(|v| std::env::var(v).ok()).filter(|k| !k.is_empty()).map(|k| (k, "environment")))
}

fn needs_key(p: &AiProvider) -> bool {
    preset(p.preset.as_deref()).map(|pr| pr.needs_key).unwrap_or(false)
}

/// The user's providers, persisted as JSON next to the connections file.
pub struct AiStore {
    path: PathBuf,
    file: AiSettingsFile,
}

impl AiStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        let mut file: AiSettingsFile = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AiSettingsFile::default(),
            Err(e) => return Err(e.into()),
        };
        // Earlier versions stored a single Anthropic key under `ai:anthropic`; adopt it.
        if file.providers.is_empty() && secrets::get_secret(&account("anthropic")).ok().flatten().is_some() {
            file.providers.push(AiProvider {
                id: "anthropic".into(),
                name: "Anthropic".into(),
                kind: ProviderKind::Anthropic,
                base_url: "https://api.anthropic.com".into(),
                model: "claude-opus-5-5".into(),
                preset: Some("anthropic".into()),
            });
            file.active = Some("anthropic".into());
        }
        Ok(Self { path, file })
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.file)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    pub fn settings(&self) -> AiSettings {
        AiSettings {
            providers: self
                .file
                .providers
                .iter()
                .map(|p| ProviderView {
                    key_source: key_for(p).ok().flatten().map(|(_, s)| s),
                    needs_key: needs_key(p),
                    provider: p.clone(),
                })
                .collect(),
            active: self.file.active.clone(),
        }
    }

    /// `key`: `None` keeps the stored key, `Some("")` removes it.
    pub fn upsert(&mut self, mut provider: AiProvider, key: Option<&str>) -> Result<AiProvider> {
        if provider.id.is_empty() {
            provider.id = uuid::Uuid::new_v4().to_string();
        }
        provider.base_url = provider.base_url.trim().trim_end_matches('/').to_string();
        if provider.base_url.is_empty() {
            return Err(Error::Invalid("The provider needs a base URL.".into()));
        }
        if provider.model.trim().is_empty() {
            return Err(Error::Invalid("Choose a model for this provider.".into()));
        }
        match key {
            None => {}
            Some(k) if k.trim().is_empty() => secrets::set_secret(&account(&provider.id), None)?,
            Some(k) => secrets::set_secret(&account(&provider.id), Some(k.trim()))?,
        }
        match self.file.providers.iter_mut().find(|p| p.id == provider.id) {
            Some(existing) => *existing = provider.clone(),
            None => self.file.providers.push(provider.clone()),
        }
        if self.file.active.is_none() {
            self.file.active = Some(provider.id.clone());
        }
        self.save()?;
        Ok(provider)
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.file.providers.retain(|p| p.id != id);
        secrets::set_secret(&account(id), None)?;
        if self.file.active.as_deref() == Some(id) {
            self.file.active = self.file.providers.first().map(|p| p.id.clone());
        }
        self.save()
    }

    pub fn set_active(&mut self, id: &str) -> Result<()> {
        if !self.file.providers.iter().any(|p| p.id == id) {
            return Err(Error::Invalid("Unknown AI provider.".into()));
        }
        self.file.active = Some(id.to_string());
        self.save()
    }

    pub fn active(&self) -> Option<AiProvider> {
        let id = self.file.active.as_deref()?;
        self.file.providers.iter().find(|p| p.id == id).cloned()
    }

    pub fn status(&self) -> AiStatus {
        match self.active() {
            Some(p) => {
                let configured = !needs_key(&p) || key_for(&p).ok().flatten().is_some();
                AiStatus { configured, provider: Some(p.name), model: Some(p.model) }
            }
            None => AiStatus { configured: false, provider: None, model: None },
        }
    }

    /// Resolves the key to use: an explicit one from the form, else the stored one.
    pub fn key(&self, p: &AiProvider, explicit: Option<&str>) -> Result<Option<String>> {
        if let Some(k) = explicit.map(str::trim).filter(|k| !k.is_empty()) {
            return Ok(Some(k.to_string()));
        }
        Ok(key_for(p)?.map(|(k, _)| k))
    }
}

// ---- HTTP

fn client() -> Result<reqwest::Client> {
    // reqwest is built without a bundled crypto provider; share sqlx's ring.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e| Error::Invalid(e.to_string()))
}

async fn send(req: reqwest::RequestBuilder, who: &str) -> Result<Value> {
    let response = req.send().await.map_err(|e| {
        Error::Invalid(if e.is_connect() { format!("Couldn't reach {who}. Is the address right and the server running?") } else { format!("{who}: {e}") })
    })?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(body);
    }
    let message = body["error"]["message"].as_str().or_else(|| body["error"].as_str()).or_else(|| body["message"].as_str()).unwrap_or("").to_string();
    Err(Error::Invalid(match status.as_u16() {
        401 | 403 => format!("{who} rejected the API key."),
        404 if message.is_empty() => format!("{who}: not found. Check the base URL and model."),
        429 => format!("{who} is rate limiting requests. Try again in a moment."),
        _ => format!("{who} returned {status}: {message}"),
    }))
}

fn anthropic(req: reqwest::RequestBuilder, key: Option<&str>) -> reqwest::RequestBuilder {
    req.header("x-api-key", key.unwrap_or_default()).header("anthropic-version", "2023-06-01")
}

fn bearer(req: reqwest::RequestBuilder, key: Option<&str>) -> reqwest::RequestBuilder {
    match key {
        Some(k) => req.bearer_auth(k),
        None => req,
    }
}

/// Model ids the provider offers; also serves as the connection test.
pub async fn list_models(p: &AiProvider, key: Option<&str>) -> Result<Vec<String>> {
    let http = client()?;
    let body = match p.kind {
        ProviderKind::Anthropic => send(anthropic(http.get(format!("{}/v1/models?limit=100", p.base_url)), key), &p.name).await?,
        ProviderKind::OpenAi => send(bearer(http.get(format!("{}/models", p.base_url)), key), &p.name).await?,
    };
    let mut ids: Vec<String> = body["data"]
        .as_array()
        .or_else(|| body["models"].as_array())
        .map(|items| items.iter().filter_map(|m| m["id"].as_str().or_else(|| m["name"].as_str())).map(|s| s.trim_start_matches("models/").to_string()).collect())
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Asks for a JSON object matching `schema` and returns it as text.
async fn complete_json(p: &AiProvider, key: Option<&str>, system: &str, user: &str, schema: &Value) -> Result<String> {
    let http = client()?;
    match p.kind {
        ProviderKind::Anthropic => {
            let mut body = json!({
                "model": p.model,
                "max_tokens": 16000,
                "output_config": { "format": { "type": "json_schema", "schema": schema } },
                "system": system,
                "messages": [{ "role": "user", "content": user }],
            });
            let current = ["claude-opus-5-5", "claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"].contains(&p.model.as_str());
            if current {
                // Filters are a small task; and if this model declines, retry on a suitable one.
                body["output_config"]["effort"] = json!("low");
                body["fallbacks"] = json!("default");
            }
            let mut req = anthropic(http.post(format!("{}/v1/messages", p.base_url)), key).json(&body);
            if current {
                req = req.header("anthropic-beta", "server-side-fallback-2026-07-01");
            }
            let reply = send(req, &p.name).await?;
            if reply["stop_reason"] == "refusal" {
                return Err(Error::Invalid("The AI declined this request. Try rephrasing it.".into()));
            }
            reply["content"]
                .as_array()
                .and_then(|blocks| blocks.iter().find(|b| b["type"] == "text"))
                .and_then(|b| b["text"].as_str())
                .map(str::to_string)
                .ok_or_else(|| Error::Invalid("The AI returned no answer.".into()))
        }
        ProviderKind::OpenAi => {
            let url = format!("{}/chat/completions", p.base_url);
            let messages = |sys: &str| json!([{ "role": "system", "content": sys }, { "role": "user", "content": user }]);
            let strict = json!({
                "model": p.model,
                "messages": messages(system),
                "response_format": { "type": "json_schema", "json_schema": { "name": "filters", "strict": true, "schema": schema } },
            });
            let reply = match send(bearer(http.post(&url), key).json(&strict), &p.name).await {
                Ok(r) => r,
                // Not every compatible server supports JSON schemas; fall back to plain JSON mode
                // with the schema spelled out in the prompt.
                Err(Error::Invalid(msg)) if msg.contains("returned 400") || msg.contains("returned 422") => {
                    let sys = format!("{system}\n\nReply with only a JSON object matching this JSON Schema:\n{schema}");
                    let loose = json!({ "model": p.model, "messages": messages(&sys), "response_format": { "type": "json_object" } });
                    send(bearer(http.post(&url), key).json(&loose), &p.name).await?
                }
                Err(e) => return Err(e),
            };
            let text = reply["choices"][0]["message"]["content"].as_str().ok_or_else(|| Error::Invalid("The AI returned no answer.".into()))?;
            Ok(strip_fences(text).to_string())
        }
    }
}

/// Some models wrap JSON in ```json fences even in JSON mode.
fn strip_fences(text: &str) -> &str {
    let t = text.trim();
    let t = t.strip_prefix("```json").or_else(|| t.strip_prefix("```")).unwrap_or(t);
    t.strip_suffix("```").unwrap_or(t).trim()
}

// ---- filters

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
         functions). It must be a single {engine} boolean expression over this table's columns, with no \
         semicolons. Otherwise leave it empty.\n\n\
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
    #[serde(default)]
    sort: Vec<Sort>,
    #[serde(default)]
    condition: String,
    #[serde(default)]
    explanation: String,
}

pub async fn filters_from_prompt(
    p: &AiProvider,
    key: Option<&str>,
    d: Dialect,
    details: &TableDetails,
    prompt: &str,
    today: &str,
) -> Result<AiFilterResult> {
    let columns: Vec<String> = details.design.columns.iter().map(|c| c.name.clone()).collect();
    let user = format!("{}\nToday is {today}.\n\nRequest: {prompt}", describe_table(details));
    let text = complete_json(p, key, &system_prompt(d), &user, &output_schema(&columns)).await?;
    parse_result(d, &columns, &text)
}

/// Parses and checks the model's answer: columns must exist and any condition must be one expression.
fn parse_result(d: Dialect, columns: &[String], text: &str) -> Result<AiFilterResult> {
    let raw: Raw = serde_json::from_str(text).map_err(|e| Error::Invalid(format!("The AI answer was not understood: {e}")))?;
    let known = |c: &str| columns.iter().any(|x| x == c);
    if let Some(bad) = raw.filters.iter().map(|f| f.column.as_str()).chain(raw.sort.iter().map(|s| s.column.as_str())).find(|c| !known(c)) {
        return Err(Error::Invalid(format!("The AI referred to a column that doesn't exist: {bad}")));
    }
    let condition = Some(raw.condition.trim().to_string()).filter(|c| !c.is_empty());
    if let Some(c) = &condition {
        dml::validate_condition(d, c).map_err(|e| Error::Invalid(format!("The AI wrote a condition Kiyi can't run safely. {e}")))?;
    }
    Ok(AiFilterResult { filters: raw.filters, sort: raw.sort, condition, explanation: raw.explanation })
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: &[&str] = &["id", "status", "total"];

    fn cols() -> Vec<String> {
        COLS.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_a_good_answer_even_in_fences() {
        let text = "```json\n{\"filters\":[{\"column\":\"status\",\"op\":\"eq\",\"value\":\"paid\"}],\"sort\":[{\"column\":\"total\",\"descending\":true}],\"condition\":\"total > 100\",\"explanation\":\"Paid orders over 100\"}\n```";
        let r = parse_result(Dialect::POSTGRES, &cols(), strip_fences(text)).unwrap();
        assert_eq!(r.filters[0].value, "paid");
        assert_eq!(r.condition.as_deref(), Some("total > 100"));
        assert!(r.sort[0].descending);
    }

    #[test]
    fn rejects_unknown_columns_and_unsafe_conditions() {
        let unknown = r#"{"filters":[{"column":"price","op":"gt","value":"1"}],"sort":[],"condition":"","explanation":""}"#;
        assert!(parse_result(Dialect::POSTGRES, &cols(), unknown).is_err());
        let unsafe_ = r#"{"filters":[],"sort":[],"condition":"1=1; DROP TABLE orders","explanation":""}"#;
        assert!(parse_result(Dialect::POSTGRES, &cols(), unsafe_).is_err());
    }

    #[test]
    fn presets_are_complete() {
        for p in presets() {
            assert!(!p.name.is_empty());
            if p.id != "custom" {
                assert!(p.base_url.starts_with("http"), "{}", p.id);
            }
        }
    }
}
