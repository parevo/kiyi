//! A real provider end to end: list models, turn requests into filters for a real table, and run
//! the resulting query. Opt-in, since it spends tokens and needs the Docker databases:
//!
//!     XAI_API_KEY=… KIYI_LIVE=1 cargo test -p kiyi-core --test ai_live -- --nocapture
//!
//! `KIYI_AI_MODEL` picks the model; otherwise the first Grok model the account lists.

use kiyi_core::ai::{self, AiProvider, ProviderKind};
use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::dml::{self, BrowseRequest};
use kiyi_core::drivers;

fn config(kind: DbKind) -> ConnectionConfig {
    ConnectionConfig {
        id: "test".into(),
        name: "test".into(),
        kind,
        host: "127.0.0.1".into(),
        port: if kind == DbKind::Postgres { 55432 } else { 53306 },
        user: "kiyi".into(),
        database: Some("kiyi_test".into()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: true,
        driver: None,
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    }
}

#[tokio::test]
async fn xai_filters_run_against_real_tables() {
    let Ok(key) = std::env::var("XAI_API_KEY") else { return };
    if std::env::var("KIYI_LIVE").is_err() {
        return;
    }
    let mut provider = AiProvider {
        id: "live".into(),
        name: "xAI Grok".into(),
        kind: ProviderKind::OpenAi,
        base_url: "https://api.x.ai/v1".into(),
        model: String::new(),
        preset: Some("xai".into()),
    };
    let models = ai::list_models(&provider, Some(&key)).await.expect("list models");
    println!("models: {models:?}");
    provider.model = std::env::var("KIYI_AI_MODEL").ok().or_else(|| models.iter().find(|m| m.contains("grok")).cloned()).expect("a Grok model");
    println!("using {}", provider.model);

    for kind in [DbKind::Postgres, DbKind::Mysql] {
        let driver = drivers::open(&config(kind), Some("kiyi")).await.expect("connect");
        let schema = if kind == DbKind::Postgres { Some("public") } else { None };
        let details = driver.table_details(schema, "orders").await.expect("table details");
        for prompt in ["paid orders over 100, biggest first", "son 30 günde verilen iptal edilmemiş siparişler", "orders that are paid or shipped"] {
            let r = ai::filters_from_prompt(&provider, Some(&key), driver.dialect(), &details, prompt, "2026-10-09").await.unwrap_or_else(|e| panic!("{kind:?} {prompt:?}: {e}"));
            println!("{kind:?} {prompt:?} → {r:?}");
            let req = BrowseRequest {
                schema: schema.map(Into::into),
                table: "orders".into(),
                filters: r.filters,
                raw_where: r.condition,
                search: None,
                search_columns: vec![],
                sort: r.sort,
                tiebreak: vec!["id".into()],
                limit: 50,
                offset: 0,
            };
            let sql = dml::browse_sql(driver.dialect(), &req);
            driver.fetch(&sql).await.unwrap_or_else(|e| panic!("{kind:?} {prompt:?}: {sql}\n{e}"));
        }
        driver.close().await;
    }
}

#[tokio::test]
async fn xai_answers_questions_fixes_and_explains_sql() {
    use kiyi_core::ai_sql;
    let Ok(key) = std::env::var("XAI_API_KEY") else { return };
    if std::env::var("KIYI_LIVE").is_err() {
        return;
    }
    let mut provider = AiProvider { id: "live".into(), name: "xAI Grok".into(), kind: ProviderKind::OpenAi, base_url: "https://api.x.ai/v1".into(), model: String::new(), preset: Some("xai".into()) };
    let models = ai::list_models(&provider, Some(&key)).await.expect("list models");
    provider.model = std::env::var("KIYI_AI_MODEL").ok().or_else(|| models.iter().find(|m| m.contains("grok")).cloned()).unwrap();

    for kind in [DbKind::Postgres, DbKind::Mysql] {
        let db = drivers::open(&config(kind), Some("kiyi")).await.expect("connect");
        let schema = db.schema().await.unwrap();
        for q in ["Which 5 customers spent the most?", "aylara göre sipariş sayısı", "how many orders are there in total?"] {
            let r = ai_sql::ask(&provider, Some(&key), db.dialect(), &schema, q, "2026-10-09").await.unwrap_or_else(|e| panic!("{kind:?} {q:?}: {e}"));
            println!("{kind:?} {q:?}\n  {}\n  {} {:?}", r.sql, r.explanation, r.chart);
            db.fetch(&r.sql).await.unwrap_or_else(|e| panic!("{kind:?} {q:?} ran {}: {e}", r.sql));
        }
        let fixed = ai_sql::fix_sql(&provider, Some(&key), db.dialect(), &schema, "SELECT emial FROM customers", "column \"emial\" does not exist").await.unwrap();
        println!("{kind:?} fixed: {} ({})", fixed.sql, fixed.explanation);
        db.fetch(&fixed.sql).await.unwrap();
        let ex = ai_sql::explain_sql(&provider, Some(&key), db.dialect(), &schema, "DELETE FROM orders", "Turkish").await.unwrap();
        println!("{kind:?} explained: {} {:?}", ex.summary, ex.warnings);
        assert!(!ex.warnings.is_empty(), "a DELETE without WHERE deserves a warning");
        db.close().await;
    }
}
