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
pub fn list_connections(ws: Ws<'_>) -> Vec<ConnectionConfig> {
    ws.list()
}

#[tauri::command]
pub fn parse_connection_url(url: String) -> CmdResult<ParsedUrl> {
    Ok(config::parse_url(&url)?)
}

#[tauri::command]
pub fn save_connection(ws: Ws<'_>, config: ConnectionConfig, password: Option<String>) -> CmdResult<ConnectionConfig> {
    Ok(ws.save(config, password)?)
}

#[tauri::command]
pub async fn delete_connection(ws: Ws<'_>, id: String) -> CmdResult<()> {
    Ok(ws.delete(&id).await?)
}

#[tauri::command]
pub async fn test_connection(ws: Ws<'_>, config: ConnectionConfig, password: Option<String>) -> CmdResult<TestReport> {
    Ok(ws.test(config, password).await)
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
