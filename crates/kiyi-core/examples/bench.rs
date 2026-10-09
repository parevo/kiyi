//! What the app does when you use it, timed against the 1M-row `kiyi_bench` databases.
//! Seed them with dev/bench/seed-*.sql (see dev/bench/README.md), then:
//!
//!   cargo run --release -p kiyi-core --example bench [-- out.json]
//!
//! Every scenario runs through the same code path as the app (SQL generation, driver,
//! value decoding) and, for streaming, the same JSON encoding the UI channel uses.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::dml::{self, BrowseRequest, Filter, FilterOp, Sort};
use kiyi_core::drivers::{self, DbDriver};
use kiyi_core::transfer::{self, ExportFormat};
use kiyi_core::types::QueryEvent;

const RUNS: usize = 7;

fn config(kind: DbKind) -> ConnectionConfig {
    ConnectionConfig {
        id: "bench".into(),
        name: "bench".into(),
        kind,
        host: "127.0.0.1".into(),
        port: if kind == DbKind::Postgres { 55432 } else { 53306 },
        user: "kiyi".into(),
        database: Some("kiyi_bench".into()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: None,
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    }
}

fn page(f: impl FnOnce(&mut BrowseRequest)) -> BrowseRequest {
    let mut req = BrowseRequest {
        schema: None,
        table: "events".into(),
        filters: vec![],
        raw_where: None,
        search: None,
        search_columns: vec![],
        sort: vec![],
        tiebreak: vec!["id".into()],
        limit: 300,
        offset: 0,
    };
    f(&mut req);
    req
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

struct Stream {
    first_row: Duration,
    total: Duration,
    rows: u64,
    bytes: u64,
}

/// Streams a whole query the way the SQL editor does: batches encoded to JSON for the UI.
async fn stream(driver: &Arc<dyn DbDriver>, sql: &str) -> Stream {
    let start = Instant::now();
    let first = Mutex::new(None);
    let rows = AtomicU64::new(0);
    let bytes = AtomicU64::new(0);
    let sink = |e: QueryEvent| {
        if let QueryEvent::Rows { rows: r } = &e {
            first.lock().unwrap().get_or_insert_with(|| start.elapsed());
            rows.fetch_add(r.len() as u64, Ordering::Relaxed);
        }
        bytes.fetch_add(serde_json::to_vec(&e).unwrap().len() as u64, Ordering::Relaxed);
    };
    driver.execute(sql, &|_| {}, &sink).await.expect("stream");
    Stream {
        first_row: first.into_inner().unwrap().unwrap_or_default(),
        total: start.elapsed(),
        rows: rows.into_inner(),
        bytes: bytes.into_inner(),
    }
}

async fn timed<F, Fut>(mut f: F) -> Duration
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    f().await; // warm-up
    let mut v = Vec::new();
    for _ in 0..RUNS {
        let t = Instant::now();
        f().await;
        v.push(t.elapsed());
    }
    median(v)
}

async fn engine(kind: DbKind) -> serde_json::Value {
    let name = if kind == DbKind::Postgres { "PostgreSQL" } else { "MySQL" };
    let cfg = config(kind);

    let connect = timed(|| async {
        let d = drivers::open(&cfg, Some("kiyi")).await.expect("connect");
        d.close().await;
    })
    .await;

    let driver = drivers::open(&cfg, Some("kiyi")).await.expect("connect");
    let d = driver.dialect();
    let fetch = |req: BrowseRequest| {
        let driver = driver.clone();
        async move {
            let (_, rows) = driver.fetch(&dml::browse_sql(d, &req)).await.expect("browse");
            assert!(!rows.is_empty());
        }
    };

    let open_table = timed(|| fetch(page(|_| {}))).await;
    let deep_page = timed(|| fetch(page(|r| r.offset = 500_000))).await;
    let sort = timed(|| fetch(page(|r| r.sort = vec![Sort { column: "created_at".into(), descending: true }]))).await;
    let filter = timed(|| {
        fetch(page(|r| {
            r.filters = vec![
                Filter { column: "kind".into(), op: FilterOp::Eq, value: "purchase".into() },
                Filter { column: "amount".into(), op: FilterOp::Gt, value: "500".into() },
            ]
        }))
    })
    .await;
    let search = timed(|| {
        fetch(page(|r| {
            r.search = Some("number 99999".into());
            r.search_columns = vec!["kind".into(), "note".into()];
        }))
    })
    .await;
    let details = timed(|| async {
        driver.table_details(None, "events").await.expect("details");
    })
    .await;
    let count = timed(|| async {
        driver.fetch(&dml::count_sql(d, &page(|_| {}))).await.expect("count");
    })
    .await;

    // Full scans: a few runs, median.
    let all = "SELECT * FROM events";
    stream(&driver, all).await;
    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(stream(&driver, all).await);
    }
    runs.sort_by_key(|s| s.total);
    let s = &runs[1];

    let path = std::env::temp_dir().join("kiyi-bench-export.csv");
    let export = {
        let mut v = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            let n = transfer::export(driver.as_ref(), &dml::export_sql(d, &page(|_| {})), ExportFormat::Csv, &path).await.expect("export");
            assert_eq!(n, 1_000_000);
            v.push(t.elapsed());
        }
        median(v)
    };
    let csv_mb = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
    let _ = std::fs::remove_file(&path);
    driver.close().await;

    let rows_per_s = s.rows as f64 / s.total.as_secs_f64();
    println!("\n{name}");
    println!("  connect                         {:>9.1} ms", ms(connect));
    println!("  open table (first 300 rows)     {:>9.1} ms", ms(open_table));
    println!("  jump to row 500,000             {:>9.1} ms", ms(deep_page));
    println!("  sort by date                    {:>9.1} ms", ms(sort));
    println!("  two filters                     {:>9.1} ms", ms(filter));
    println!("  search text columns             {:>9.1} ms", ms(search));
    println!("  exact row count (1M)            {:>9.1} ms", ms(count));
    println!("  table structure                 {:>9.1} ms", ms(details));
    println!("  stream 1M rows: first row       {:>9.1} ms", ms(s.first_row));
    println!("  stream 1M rows: all             {:>9.1} ms  ({:.0} rows/s, {:.0} MB to UI)", ms(s.total), rows_per_s, s.bytes as f64 / 1e6);
    println!("  export 1M rows to CSV           {:>9.1} ms  ({csv_mb:.0} MB)", ms(export));

    serde_json::json!({
        "engine": name,
        "connectMs": ms(connect),
        "openTableMs": ms(open_table),
        "deepPageMs": ms(deep_page),
        "sortMs": ms(sort),
        "filterMs": ms(filter),
        "searchMs": ms(search),
        "countMs": ms(count),
        "detailsMs": ms(details),
        "streamFirstRowMs": ms(s.first_row),
        "streamAllMs": ms(s.total),
        "streamRowsPerSec": rows_per_s.round(),
        "exportCsvMs": ms(export),
    })
}

#[tokio::main]
async fn main() {
    let out = std::env::args().nth(1);
    let results = vec![engine(DbKind::Postgres).await, engine(DbKind::Mysql).await];
    if let Some(out) = out {
        std::fs::write(&out, serde_json::to_string_pretty(&results).unwrap()).unwrap();
        println!("\nwrote {out}");
    }
}
