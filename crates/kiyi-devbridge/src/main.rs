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
use axum::http::StatusCode;
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
        "save_connection" => ws.save(arg(a, "config")?, arg(a, "password")?).map(ok).unwrap_or_else(fail),
        "delete_connection" => ws.delete(&id()?).await.map(ok).unwrap_or_else(fail),
        "test_connection" => ok(ws.test(arg(a, "config")?, arg(a, "password")?).await),
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
        "ai_filters" => {
            let schema: Option<String> = arg(a, "schema")?;
            ws.ai_filters(&id()?, schema.as_deref(), &arg::<String>(a, "table")?, &arg::<String>(a, "prompt")?, &arg::<String>(a, "today")?)
                .await
                .map(ok)
                .unwrap_or_else(fail)
        }
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
    let app = Router::new().route("/invoke/{cmd}", post(invoke)).layer(CorsLayer::permissive()).with_state(ws);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:1421").await.expect("port 1421");
    println!("kiyi-devbridge on http://127.0.0.1:1421 (config in {})", dir.display());
    axum::serve(listener, app).await.unwrap();
}
