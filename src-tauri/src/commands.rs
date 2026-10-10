//! Thin bridges from the webview to `kiyi_core`. No logic lives here.

use std::sync::Arc;

use kiyi_core::config::{self, ConnectionConfig, ParsedUrl};
use kiyi_core::design::{TableAction, TableDesign, TableDetails};
use kiyi_core::dml::{BrowseRequest, ChangeSet};
use kiyi_core::error::ErrorInfo;
use kiyi_core::types::{QueryEvent, SchemaSnapshot};
use kiyi_core::workspace::{ConnectInfo, Page, ScriptKind, TestReport, Workspace};
use tauri::ipc::Channel;
use tauri::State;

type Ws<'a> = State<'a, Arc<Workspace>>;
type CmdResult<T> = Result<T, ErrorInfo>;

#[tauri::command]
pub fn list_drivers() -> Vec<kiyi_core::catalog::DriverInfo> {
    kiyi_core::catalog::drivers()
}

#[tauri::command]
pub fn list_connections(ws: Ws<'_>) -> Vec<ConnectionConfig> {
    ws.list()
}

#[tauri::command]
pub fn parse_connection_url(url: String) -> CmdResult<ParsedUrl> {
    Ok(config::parse_url(&url)?)
}

#[tauri::command]
pub fn save_connection(ws: Ws<'_>, config: ConnectionConfig, password: Option<String>, tunnel_secret: Option<String>) -> CmdResult<ConnectionConfig> {
    Ok(ws.save(config, password, tunnel_secret)?)
}

#[tauri::command]
pub async fn delete_connection(ws: Ws<'_>, id: String) -> CmdResult<()> {
    Ok(ws.delete(&id).await?)
}

#[tauri::command]
pub async fn test_connection(ws: Ws<'_>, config: ConnectionConfig, password: Option<String>, tunnel_secret: Option<String>) -> CmdResult<TestReport> {
    Ok(ws.test(config, password, tunnel_secret).await)
}

#[tauri::command]
pub fn forget_host_key(ws: Ws<'_>, host: String, port: u16) {
    ws.forget_host_key(&host, port);
}

#[tauri::command]
pub async fn connect(ws: Ws<'_>, id: String) -> CmdResult<ConnectInfo> {
    Ok(ws.connect(&id).await?)
}

#[tauri::command]
pub async fn disconnect(ws: Ws<'_>, id: String) -> CmdResult<()> {
    ws.disconnect(&id).await;
    Ok(())
}

#[tauri::command]
pub async fn load_schema(ws: Ws<'_>, id: String) -> CmdResult<SchemaSnapshot> {
    Ok(ws.schema(&id).await?)
}

#[tauri::command]
pub async fn run_query(
    ws: Ws<'_>,
    connection_id: String,
    query_id: String,
    sql: String,
    on_event: Channel<QueryEvent>,
) -> CmdResult<()> {
    let sink = Arc::new(move |event: QueryEvent| {
        let _ = on_event.send(event);
    });
    Ok(ws.run(&connection_id, query_id, sql, sink)?)
}

#[tauri::command]
pub async fn compare(ws: Ws<'_>, left: String, right: String) -> CmdResult<kiyi_core::compare::Comparison> {
    Ok(ws.compare(&left, &right).await?)
}

#[tauri::command]
pub fn backup_tools(ws: Ws<'_>, id: String, today: String) -> CmdResult<kiyi_core::backup::BackupTools> {
    Ok(ws.backup_tools(&id, &today)?)
}

#[tauri::command]
pub async fn backup(ws: Ws<'_>, id: String, path: String, prefer_kiyi: bool) -> CmdResult<kiyi_core::backup::BackupReport> {
    Ok(ws.backup(&id, std::path::Path::new(&path), prefer_kiyi).await?)
}

#[tauri::command]
pub async fn restore(ws: Ws<'_>, id: String, path: String) -> CmdResult<kiyi_core::backup::RestoreReport> {
    Ok(ws.restore(&id, std::path::Path::new(&path)).await?)
}

#[tauri::command]
pub async fn schema_graph(ws: Ws<'_>, id: String) -> CmdResult<kiyi_core::graph::SchemaGraph> {
    Ok(ws.schema_graph(&id).await?)
}

#[tauri::command]
pub async fn list_objects(ws: Ws<'_>, id: String) -> CmdResult<kiyi_core::objects::ObjectList> {
    Ok(ws.objects(&id).await?)
}

#[tauri::command]
pub async fn object_source(ws: Ws<'_>, id: String, object: kiyi_core::objects::DbObject) -> CmdResult<kiyi_core::objects::ObjectSource> {
    Ok(ws.object_source(&id, &object).await?)
}

#[tauri::command]
pub fn object_template(ws: Ws<'_>, id: String, kind: kiyi_core::objects::ObjectKind, schema: Option<String>) -> CmdResult<String> {
    Ok(ws.object_template(&id, kind, schema.as_deref())?)
}

#[tauri::command]
pub fn export_connections(ws: Ws<'_>, ids: Vec<String>, path: String) -> CmdResult<usize> {
    Ok(ws.export_connections(&ids, std::path::Path::new(&path))?)
}

#[tauri::command]
pub fn import_connections(ws: Ws<'_>, path: String) -> CmdResult<Vec<ConnectionConfig>> {
    Ok(ws.import_connections(std::path::Path::new(&path))?)
}

#[tauri::command]
pub async fn create_sample(ws: Ws<'_>) -> CmdResult<ConnectionConfig> {
    Ok(ws.create_sample().await?)
}

/// What went wrong the last time Kiyi closed unexpectedly, once; None after a normal exit.
#[tauri::command]
pub fn take_crash_report(log: State<'_, crate::logging::LogFile>) -> Option<String> {
    crate::logging::take_crash(&log.0)
}

/// Opens a new GitHub issue in the browser with the title and text filled in. Nothing is sent
/// until the person submits it there.
#[tauri::command]
pub fn open_issue(title: String, body: String) -> CmdResult<()> {
    crate::logging::open_issue(&title, &body).map_err(|e| ErrorInfo::from(kiyi_core::error::Error::Io(e)))
}

#[tauri::command]
pub async fn explain(ws: Ws<'_>, id: String, sql: String) -> CmdResult<kiyi_core::explain::PlanNode> {
    Ok(ws.explain(&id, &sql).await?)
}

#[tauri::command]
pub fn check_sql(ws: Ws<'_>, id: String, sql: String) -> CmdResult<kiyi_core::dml::ScriptCheck> {
    Ok(ws.check_sql(&id, &sql)?)
}

#[tauri::command]
pub async fn cancel_query(ws: Ws<'_>, query_id: String) -> CmdResult<()> {
    ws.cancel(&query_id).await;
    Ok(())
}

#[tauri::command]
pub async fn table_details(ws: Ws<'_>, id: String, schema: Option<String>, table: String) -> CmdResult<TableDetails> {
    Ok(ws.table_details(&id, schema.as_deref(), &table).await?)
}

#[tauri::command]
pub async fn browse_table(ws: Ws<'_>, id: String, request: BrowseRequest) -> CmdResult<Page> {
    Ok(ws.browse(&id, &request).await?)
}

#[tauri::command]
pub async fn count_rows(ws: Ws<'_>, id: String, request: BrowseRequest) -> CmdResult<u64> {
    Ok(ws.count(&id, &request).await?)
}

#[tauri::command]
pub async fn summarize(ws: Ws<'_>, id: String, request: kiyi_core::dml::SummaryRequest) -> CmdResult<Page> {
    Ok(ws.summarize(&id, &request).await?)
}

#[tauri::command]
pub async fn plan_replace(ws: Ws<'_>, id: String, request: BrowseRequest, column: String, find: String, replacement: String) -> CmdResult<kiyi_core::workspace::ReplacePlan> {
    Ok(ws.plan_replace(&id, &request, &column, &find, &replacement).await?)
}

#[tauri::command]
pub fn plan_row_changes(ws: Ws<'_>, id: String, changes: ChangeSet) -> CmdResult<Vec<String>> {
    Ok(ws.plan_changes(&id, &changes)?)
}

#[tauri::command]
pub fn plan_table(ws: Ws<'_>, id: String, schema: Option<String>, old: Option<TableDesign>, new: TableDesign) -> CmdResult<Vec<String>> {
    Ok(ws.plan_table(&id, schema.as_deref(), old.as_ref(), &new)?)
}

#[tauri::command]
pub fn plan_table_action(
    ws: Ws<'_>,
    id: String,
    schema: Option<String>,
    table: String,
    is_view: bool,
    action: TableAction,
) -> CmdResult<Vec<String>> {
    Ok(ws.plan_action(&id, schema.as_deref(), &table, is_view, &action)?)
}

#[tauri::command]
pub async fn execute_script(ws: Ws<'_>, id: String, statements: Vec<String>, kind: ScriptKind) -> CmdResult<Vec<u64>> {
    Ok(ws.execute_script(&id, &statements, kind).await?)
}

#[tauri::command]
pub fn ai_status(ws: Ws<'_>) -> kiyi_core::ai::AiStatus {
    ws.ai().status()
}

#[tauri::command]
pub fn ai_presets() -> Vec<kiyi_core::ai::ProviderPreset> {
    kiyi_core::ai::presets()
}

#[tauri::command]
pub fn ai_settings(ws: Ws<'_>) -> kiyi_core::ai::AiSettings {
    ws.ai().settings()
}

#[tauri::command]
pub fn save_ai_provider(ws: Ws<'_>, provider: kiyi_core::ai::AiProvider, key: Option<String>) -> CmdResult<kiyi_core::ai::AiProvider> {
    Ok(ws.ai().upsert(provider, key.as_deref())?)
}

#[tauri::command]
pub fn delete_ai_provider(ws: Ws<'_>, id: String) -> CmdResult<()> {
    Ok(ws.ai().remove(&id)?)
}

#[tauri::command]
pub fn set_active_ai(ws: Ws<'_>, id: String) -> CmdResult<()> {
    Ok(ws.ai().set_active(&id)?)
}

#[tauri::command]
pub async fn ai_models(ws: Ws<'_>, provider: kiyi_core::ai::AiProvider, key: Option<String>) -> CmdResult<Vec<String>> {
    Ok(ws.ai_models(&provider, key.as_deref()).await?)
}

/// Errors the interface caught (render failures, unhandled rejections), for the log file.
#[tauri::command]
pub fn log_ui_error(message: String) {
    tracing::error!(target: "ui", "{message}");
}

#[tauri::command]
pub fn diagnostics(app: tauri::AppHandle, log: State<'_, crate::logging::LogFile>) -> String {
    crate::logging::diagnostics(&log.0, &app.package_info().version.to_string())
}

#[tauri::command]
pub fn ssh_config_hosts() -> Vec<kiyi_core::ssh_config::SshHost> {
    kiyi_core::ssh_config::hosts()
}

#[tauri::command]
pub async fn discover_local() -> Vec<kiyi_core::discover::LocalDatabase> {
    kiyi_core::discover::local_databases().await
}

#[tauri::command]
pub async fn ai_filters(
    ws: Ws<'_>,
    id: String,
    schema: Option<String>,
    table: String,
    prompt: String,
    today: String,
) -> CmdResult<kiyi_core::ai::AiFilterResult> {
    Ok(ws.ai_filters(&id, schema.as_deref(), &table, &prompt, &today).await?)
}

#[tauri::command]
pub async fn export_query(ws: Ws<'_>, id: String, sql: String, format: kiyi_core::transfer::ExportFormat, path: String) -> CmdResult<u64> {
    Ok(ws.export_query(&id, &sql, format, std::path::Path::new(&path)).await?)
}

#[tauri::command]
pub async fn ai_ask(ws: Ws<'_>, id: String, question: String, today: String) -> CmdResult<kiyi_core::ai_sql::AskResult> {
    Ok(ws.ai_ask(&id, &question, &today).await?)
}

#[tauri::command]
pub async fn ai_write_sql(ws: Ws<'_>, id: String, instruction: String, current: String, today: String) -> CmdResult<kiyi_core::ai_sql::SqlSuggestion> {
    Ok(ws.ai_write_sql(&id, &instruction, &current, &today).await?)
}

#[tauri::command]
pub async fn ai_fix_sql(ws: Ws<'_>, id: String, sql: String, error: String) -> CmdResult<kiyi_core::ai_sql::SqlSuggestion> {
    Ok(ws.ai_fix_sql(&id, &sql, &error).await?)
}

#[tauri::command]
pub async fn ai_explain_sql(ws: Ws<'_>, id: String, sql: String, language: String) -> CmdResult<kiyi_core::ai_sql::QueryExplanation> {
    Ok(ws.ai_explain_sql(&id, &sql, &language).await?)
}

#[tauri::command]
pub async fn export_table(ws: Ws<'_>, id: String, request: BrowseRequest, format: kiyi_core::transfer::ExportFormat, path: String) -> CmdResult<u64> {
    Ok(ws.export(&id, &request, format, std::path::Path::new(&path)).await?)
}

#[tauri::command]
pub fn csv_preview(path: String, encoding: Option<String>, sheet: Option<String>) -> CmdResult<kiyi_core::transfer::CsvPreview> {
    Ok(kiyi_core::transfer::preview(std::path::Path::new(&path), encoding.as_deref(), sheet.as_deref())?)
}

#[tauri::command]
pub async fn import_csv(ws: Ws<'_>, id: String, path: String, plan: kiyi_core::transfer::ImportPlan) -> CmdResult<u64> {
    Ok(ws.import_csv(&id, std::path::Path::new(&path), &plan).await?)
}
