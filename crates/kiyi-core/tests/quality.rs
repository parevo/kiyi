//! Data quality checks and their fixes on real databases: the fixes must change the rows they
//! describe, including trailing spaces that SQL Server and MariaDB ignore in comparisons.
//! SQLite always runs; PostgreSQL and MySQL need `KIYI_LIVE=1`.

use std::sync::Arc;

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::drivers::{self, DbDriver};
use kiyi_core::quality::{self, Kind};

fn live() -> bool {
    std::env::var("KIYI_LIVE").is_ok()
}

fn config(kind: DbKind) -> ConnectionConfig {
    let sqlite = kind == DbKind::Sqlite;
    let path = std::env::temp_dir().join(format!("kiyi-quality-{}.db", std::process::id()));
    if sqlite {
        let _ = std::fs::remove_file(&path);
    }
    ConnectionConfig {
        id: "q".into(),
        name: "q".into(),
        kind,
        host: if sqlite { String::new() } else { "127.0.0.1".into() },
        port: match kind {
            DbKind::Postgres => 55432,
            DbKind::Mysql => 53306,
            _ => 0,
        },
        user: if sqlite { String::new() } else { "kiyi".into() },
        database: Some(if sqlite { path.to_string_lossy().into_owned() } else { "kiyi_test".into() }),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: None,
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    }
}

async fn exec(db: &Arc<dyn DbDriver>, sql: &[String]) {
    db.execute_script(sql, false, false).await.unwrap_or_else(|e| panic!("{sql:?}: {e}"));
}

async fn cycle(kind: DbKind) {
    let db = drivers::open(&config(kind), Some("kiyi")).await.expect("connect");
    let d = db.dialect();
    let t = d.ident("quality_people");
    let (id, text) = match kind {
        DbKind::Postgres => ("id serial PRIMARY KEY", "text"),
        DbKind::Mysql => ("id INT AUTO_INCREMENT PRIMARY KEY", "VARCHAR(100)"),
        _ => ("id INTEGER PRIMARY KEY", "TEXT"),
    };
    exec(&db, &[format!("DROP TABLE IF EXISTS {t}"), format!("CREATE TABLE {t} ({id}, email {text}, city {text})")]).await;
    let mut rows = Vec::new();
    for i in 0..40 {
        let city = match i % 8 {
            0 => "Istanbul ",
            1 => "istanbul",
            2..=4 => "İstanbul",
            _ => "Ankara",
        };
        let email = if i < 2 { "same@x.io".to_string() } else { format!("p{i}@x.io") };
        rows.push(format!("({}, {})", d.string(&email), d.string(city)));
    }
    exec(&db, &[format!("INSERT INTO {t} (email, city) VALUES {}", rows.join(", "))]).await;

    let schema = if kind == DbKind::Sqlite { None } else if kind == DbKind::Mysql { Some("kiyi_test") } else { Some("public") };
    let report = quality::check_table(db.as_ref(), schema, "quality_people", quality::SAMPLE).await.unwrap();
    assert_eq!((report.rows, report.checked), (40, 40));
    let find = |k: Kind| report.findings.iter().find(|f| f.kind == k).unwrap_or_else(|| panic!("{kind:?}: no {k:?} in {:#?}", report.findings)).clone();
    assert_eq!(find(Kind::Duplicates).examples[0].value.as_deref(), Some("same@x.io"));
    let spaces = find(Kind::Spaces);
    let spellings = find(Kind::Spellings);

    // The fixes run as they are and leave nothing to find.
    exec(&db, &spaces.fix.unwrap().statements).await;
    let after = quality::check_table(db.as_ref(), schema, "quality_people", quality::SAMPLE).await.unwrap();
    assert!(after.findings.iter().all(|f| f.kind != Kind::Spaces), "{kind:?}: trailing spaces survived: {:#?}", after.findings);
    let spellings = after.findings.iter().find(|f| f.kind == Kind::Spellings).cloned().unwrap_or(spellings);
    exec(&db, &spellings.fix.unwrap().statements).await;
    let (_, cities) = db.fetch(&format!("SELECT city, COUNT(*) FROM {t} GROUP BY city ORDER BY city")).await.unwrap();
    let cities: Vec<String> = cities.iter().map(|r| format!("{}={}", r[0].clone().unwrap(), r[1].clone().unwrap())).collect();
    assert_eq!(cities, vec!["Ankara=15", "İstanbul=25"], "{kind:?}");
    exec(&db, &[format!("DROP TABLE {t}")]).await;
}

#[tokio::test]
async fn sqlite_fixes_clean_the_data() {
    cycle(DbKind::Sqlite).await;
}

#[tokio::test]
async fn postgres_fixes_clean_the_data() {
    if live() {
        cycle(DbKind::Postgres).await;
    }
}

#[tokio::test]
async fn mysql_fixes_clean_the_data() {
    if live() {
        cycle(DbKind::Mysql).await;
    }
}

/// The sample database has a little deliberate mess, so people trying Kiyi find Check data useful.
#[tokio::test]
async fn the_sample_database_has_something_to_find() {
    let path = std::env::temp_dir().join(format!("kiyi-quality-sample-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let db = drivers::open(&kiyi_core::sample::config(&path.to_string_lossy()), None).await.unwrap();
    db.execute_script(&kiyi_core::sample::statements(), true, false).await.unwrap();
    let customers = quality::check_table(db.as_ref(), None, "customers", quality::SAMPLE).await.unwrap();
    let kinds: Vec<Kind> = customers.findings.iter().map(|f| f.kind).collect();
    assert!(kinds.contains(&Kind::Spellings) && kinds.contains(&Kind::Spaces), "{:#?}", customers.findings);
    let orders = quality::check_table(db.as_ref(), None, "orders", quality::SAMPLE).await.unwrap();
    assert!(orders.findings.iter().any(|f| f.kind == Kind::Negative), "{:#?}", orders.findings);
}
