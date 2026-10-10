//! Development-only HTTP bridge to `kiyi_core`.
//!
//! Lets the UI run in an ordinary browser (for screenshots and end-to-end checks) against
//! real databases: `src/dev/bridge.ts` forwards every Tauri `invoke` here. Mirrors the
//! commands in `src-tauri/src/commands.rs`. Never shipped.
//!
//!     cargo run -p kiyi-devbridge   # listens on 127.0.0.1:1421

// Dev tool: an error is an HTTP response, and its size doesn't matter here.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use kiyi_core::error::ErrorInfo;
use kiyi_core::types::QueryEvent;
use kiyi_core::workspace::Workspace;
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;

type Ws = Arc<Workspace>;

fn arg<T: serde::de::DeserializeOwned>(args: &Value, name: &str) -> Result<T, Response> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad argument {name}: {e}")).into_response())
}

fn fail(e: impl Into<ErrorInfo>) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(e.into())).into_response()
}

fn ok(v: impl serde::Serialize) -> Response {
    Json(json!(v)).into_response()
}

async fn invoke(State(ws): State<Ws>, Path(cmd): Path<String>, Json(args): Json<Value>) -> Response {
    match dispatch(&ws, &cmd, &args).await {
        Ok(r) | Err(r) => r,
    }
}

async fn dispatch(ws: &Ws, cmd: &str, a: &Value) -> Result<Response, Response> {
    let id = || arg::<String>(a, "id");
    Ok(match cmd {
        "list_drivers" => ok(kiyi_core::catalog::drivers()),
        "list_connections" => ok(ws.list()),
        "parse_connection_url" => kiyi_core::config::parse_url(&arg::<String>(a, "url")?).map(ok).unwrap_or_else(fail),
        "save_connection" => ws.save(arg(a, "config")?, arg(a, "password")?, arg(a, "tunnelSecret")?).map(ok).unwrap_or_else(fail),
        "delete_connection" => ws.delete(&id()?).await.map(ok).unwrap_or_else(fail),
        "test_connection" => ok(ws.test(arg(a, "config")?, arg(a, "password")?, arg(a, "tunnelSecret")?).await),
        "forget_host_key" => {
            ws.forget_host_key(&arg::<String>(a, "host")?, arg(a, "port")?);
            ok(())
        }
        "connect" => ws.connect(&id()?).await.map(ok).unwrap_or_else(fail),
        "disconnect" => {
            ws.disconnect(&id()?).await;
            ok(())
        }
        "load_schema" => ws.schema(&id()?).await.map(ok).unwrap_or_else(fail),
        "table_details" => {
            let schema: Option<String> = arg(a, "schema")?;
            ws.table_details(&id()?, schema.as_deref(), &arg::<String>(a, "table")?).await.map(ok).unwrap_or_else(fail)
        }
        "browse_table" => ws.browse(&id()?, &arg(a, "request")?).await.map(ok).unwrap_or_else(fail),
        "count_rows" => ws.count(&id()?, &arg(a, "request")?).await.map(ok).unwrap_or_else(fail),
        "summarize" => ws.summarize(&id()?, &arg(a, "request")?).await.map(ok).unwrap_or_else(fail),
        "plan_replace" => ws
            .plan_replace(&id()?, &arg(a, "request")?, &arg::<String>(a, "column")?, &arg::<String>(a, "find")?, &arg::<String>(a, "replacement")?)
            .await
            .map(ok)
            .unwrap_or_else(fail),
        "plan_row_changes" => ws.plan_changes(&id()?, &arg(a, "changes")?).map(ok).unwrap_or_else(fail),
        "plan_table" => {
            let schema: Option<String> = arg(a, "schema")?;
            let old: Option<kiyi_core::design::TableDesign> = arg(a, "old")?;
            ws.plan_table(&id()?, schema.as_deref(), old.as_ref(), &arg(a, "new")?).map(ok).unwrap_or_else(fail)
        }
        "plan_table_action" => {
            let schema: Option<String> = arg(a, "schema")?;
            ws.plan_action(&id()?, schema.as_deref(), &arg::<String>(a, "table")?, arg(a, "isView")?, &arg(a, "action")?)
                .map(ok)
                .unwrap_or_else(fail)
        }
        "execute_script" => {
            let statements: Vec<String> = arg(a, "statements")?;
            ws.execute_script(&id()?, &statements, arg(a, "kind")?).await.map(ok).unwrap_or_else(fail)
        }
        "ai_status" => ok(ws.ai().status()),
        "ai_presets" => ok(kiyi_core::ai::presets()),
        "ai_settings" => ok(ws.ai().settings()),
        "save_ai_provider" => {
            let key: Option<String> = arg(a, "key")?;
            ws.ai().upsert(arg(a, "provider")?, key.as_deref()).map(ok).unwrap_or_else(fail)
        }
        "delete_ai_provider" => ws.ai().remove(&id()?).map(ok).unwrap_or_else(fail),
        "set_active_ai" => ws.ai().set_active(&id()?).map(ok).unwrap_or_else(fail),
        "ai_models" => {
            let key: Option<String> = arg(a, "key")?;
            ws.ai_models(&arg(a, "provider")?, key.as_deref()).await.map(ok).unwrap_or_else(fail)
        }
        "discover_local" => ok(kiyi_core::discover::local_databases().await),
        "ssh_config_hosts" => ok(kiyi_core::ssh_config::hosts()),
        "log_ui_error" => {
            eprintln!("ui error: {}", arg::<String>(a, "message")?);
            ok(())
        }
        "diagnostics" => ok("Kiyi (devbridge)\nLogs go to the terminal running kiyi-devbridge."),
        "compare" => ws.compare(&arg::<String>(a, "left")?, &arg::<String>(a, "right")?).await.map(ok).unwrap_or_else(fail),
        "backup_tools" => ws.backup_tools(&id()?, &arg::<String>(a, "today")?).map(ok).unwrap_or_else(fail),
        "backup" => ws.backup(&id()?, std::path::Path::new(&arg::<String>(a, "path")?), arg(a, "preferKiyi")?).await.map(ok).unwrap_or_else(fail),
        "restore" => ws.restore(&id()?, std::path::Path::new(&arg::<String>(a, "path")?)).await.map(ok).unwrap_or_else(fail),
        "schema_graph" => ws.schema_graph(&id()?).await.map(ok).unwrap_or_else(fail),
        "list_objects" => ws.objects(&id()?).await.map(ok).unwrap_or_else(fail),
        "object_source" => ws.object_source(&id()?, &arg(a, "object")?).await.map(ok).unwrap_or_else(fail),
        "object_template" => {
            let schema: Option<String> = arg(a, "schema")?;
            ws.object_template(&id()?, arg(a, "kind")?, schema.as_deref()).map(ok).unwrap_or_else(fail)
        }
        "export_connections" => ws.export_connections(&arg::<Vec<String>>(a, "ids")?, std::path::Path::new(&arg::<String>(a, "path")?)).map(ok).unwrap_or_else(fail),
        "import_connections" => ws.import_connections(std::path::Path::new(&arg::<String>(a, "path")?)).map(ok).unwrap_or_else(fail),
        "create_sample" => ws.create_sample().await.map(ok).unwrap_or_else(fail),
        "take_crash_report" => ok(Option::<String>::None),
        "open_issue" => {
            println!("open_issue: {}", arg::<String>(a, "title")?);
            ok(())
        }
        "explain" => ws.explain(&id()?, &arg::<String>(a, "sql")?).await.map(ok).unwrap_or_else(fail),
        "check_sql" => ws.check_sql(&id()?, &arg::<String>(a, "sql")?).map(ok).unwrap_or_else(fail),
        "ai_filters" => {
            let schema: Option<String> = arg(a, "schema")?;
            ws.ai_filters(&id()?, schema.as_deref(), &arg::<String>(a, "table")?, &arg::<String>(a, "prompt")?, &arg::<String>(a, "today")?)
                .await
                .map(ok)
                .unwrap_or_else(fail)
        }
        "ai_ask" => ws.ai_ask(&id()?, &arg::<String>(a, "question")?, &arg::<String>(a, "today")?).await.map(ok).unwrap_or_else(fail),
        "ai_write_sql" => ws
            .ai_write_sql(&id()?, &arg::<String>(a, "instruction")?, &arg::<String>(a, "current")?, &arg::<String>(a, "today")?)
            .await
            .map(ok)
            .unwrap_or_else(fail),
        "ai_fix_sql" => ws.ai_fix_sql(&id()?, &arg::<String>(a, "sql")?, &arg::<String>(a, "error")?).await.map(ok).unwrap_or_else(fail),
        "ai_explain_sql" => ws.ai_explain_sql(&id()?, &arg::<String>(a, "sql")?, &arg::<String>(a, "language")?).await.map(ok).unwrap_or_else(fail),
        "export_query" => ws
            .export_query(&id()?, &arg::<String>(a, "sql")?, arg(a, "format")?, std::path::Path::new(&arg::<String>(a, "path")?))
            .await
            .map(ok)
            .unwrap_or_else(fail),
        "export_table" => ws
            .export(&id()?, &arg(a, "request")?, arg(a, "format")?, std::path::Path::new(&arg::<String>(a, "path")?))
            .await
            .map(ok)
            .unwrap_or_else(fail),
        "csv_preview" => {
            let encoding: Option<String> = arg(a, "encoding")?;
            let sheet: Option<String> = arg(a, "sheet")?;
            kiyi_core::transfer::preview(std::path::Path::new(&arg::<String>(a, "path")?), encoding.as_deref(), sheet.as_deref()).map(ok).unwrap_or_else(fail)
        }
        "import_csv" => ws.import_csv(&id()?, std::path::Path::new(&arg::<String>(a, "path")?), &arg(a, "plan")?).await.map(ok).unwrap_or_else(fail),
        "cancel_query" => {
            ws.cancel(&arg::<String>(a, "queryId")?).await;
            ok(())
        }
        // Streaming in the app; here the whole run is returned and replayed by the client.
        "run_query" => {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let sink = Arc::new(move |e: QueryEvent| {
                let _ = tx.send(e);
            });
            if let Err(e) = ws.run(&arg::<String>(a, "connectionId")?, arg(a, "queryId")?, arg(a, "sql")?, sink) {
                return Ok(fail(e));
            }
            let mut events = Vec::new();
            while let Some(e) = rx.recv().await {
                let done = matches!(e, QueryEvent::Done { .. });
                events.push(e);
                if done {
                    break;
                }
            }
            ok(events)
        }
        _ => (StatusCode::NOT_FOUND, format!("unknown command {cmd}")).into_response(),
    })
}

#[tokio::main]
async fn main() {
    let dir = std::env::temp_dir().join("kiyi-devbridge");
    let ws: Ws = Arc::new(Workspace::new(&dir).expect("workspace"));
    // Only the Vite dev server may call in. Any other page open in the browser could otherwise run
    // SQL on every saved connection. Requests must be JSON, which forces a CORS preflight.
    let cors = CorsLayer::new()
        .allow_origin(["http://localhost:1420".parse::<HeaderValue>().unwrap(), "http://127.0.0.1:1420".parse().unwrap()])
        .allow_methods([Method::POST])
        .allow_headers([header::CONTENT_TYPE]);
    let app = Router::new().route("/invoke/{cmd}", post(invoke)).layer(cors).with_state(ws);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:1421").await.expect("port 1421");
    println!("kiyi-devbridge on http://127.0.0.1:1421 (config in {})", dir.display());
    axum::serve(listener, app).await.unwrap();
}
