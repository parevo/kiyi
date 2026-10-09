//! The OpenAI-compatible path against a local stand-in server: model listing, the JSON-schema
//! request, and the fallback to plain JSON mode for servers that reject schemas.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use kiyi_core::ai::{self, AiProvider, ProviderKind};
use kiyi_core::design::{ColumnDesign, TableDesign, TableDetails};
use kiyi_core::dialect::Dialect;
use serde_json::{json, Value};

async fn serve() -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/models", get(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer test-key");
            Json(json!({ "data": [{ "id": "local-b" }, { "id": "local-a" }] }))
        }))
        .route(
            "/v1/chat/completions",
            post(|State(calls): State<Arc<AtomicUsize>>, Json(body): Json<Value>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                if body["response_format"]["type"] == "json_schema" {
                    return (StatusCode::BAD_REQUEST, Json(json!({ "error": { "message": "response_format json_schema not supported" } })));
                }
                assert!(body["messages"][0]["content"].as_str().unwrap().contains("JSON Schema"));
                let answer = "```json\n{\"filters\":[{\"column\":\"status\",\"op\":\"eq\",\"value\":\"paid\"}],\"sort\":[],\"condition\":\"\",\"explanation\":\"Paid orders\"}\n```";
                (StatusCode::OK, Json(json!({ "choices": [{ "message": { "content": answer } }] })))
            }),
        )
        .with_state(calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, calls)
}

fn details() -> TableDetails {
    let col = |name: &str| ColumnDesign {
        original: Some(name.into()),
        name: name.into(),
        data_type: "text".into(),
        nullable: true,
        default: None,
        primary_key: false,
        auto_increment: false,
        comment: None,
        generated: false,
        extra: None,
        enum_values: vec![],
    };
    TableDetails {
        schema: None,
        design: TableDesign { name: "orders".into(), columns: vec![col("id"), col("status")], indexes: vec![], foreign_keys: vec![], primary_key_name: None },
        is_view: false,
        row_estimate: None,
    }
}

#[tokio::test]
async fn openai_compatible_models_and_filters() {
    let (base_url, calls) = serve().await;
    let provider = AiProvider { id: "t".into(), name: "Local".into(), kind: ProviderKind::OpenAi, base_url, model: "local-a".into(), preset: Some("custom".into()) };

    assert_eq!(ai::list_models(&provider, Some("test-key")).await.unwrap(), ["local-a", "local-b"]);

    let r = ai::filters_from_prompt(&provider, Some("test-key"), Dialect::POSTGRES, &details(), "paid ones", "2026-10-05").await.unwrap();
    assert_eq!(r.filters[0].column, "status");
    assert_eq!(r.explanation, "Paid orders");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "schema request, then the JSON-mode fallback");
}

#[tokio::test]
async fn unreachable_server_gives_a_clear_error() {
    let provider = AiProvider { id: "t".into(), name: "Ollama".into(), kind: ProviderKind::OpenAi, base_url: "http://127.0.0.1:9/v1".into(), model: "x".into(), preset: Some("ollama".into()) };
    let err = ai::list_models(&provider, None).await.unwrap_err().to_string();
    assert!(err.contains("Couldn't reach Ollama"), "{err}");
}

#[tokio::test]
async fn retries_after_a_rate_limit() {
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/models",
            get(|State(hits): State<Arc<AtomicUsize>>| async move {
                if hits.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "0")], Json(json!({ "error": { "message": "slow down" } })));
                }
                (StatusCode::OK, [("retry-after", "0")], Json(json!({ "data": [{ "id": "m" }] })))
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = AiProvider { id: "t".into(), name: "Local".into(), kind: ProviderKind::OpenAi, base_url, model: "m".into(), preset: Some("custom".into()) };
    assert_eq!(ai::list_models(&provider, None).await.unwrap(), ["m"]);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}
