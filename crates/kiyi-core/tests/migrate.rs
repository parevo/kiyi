//! Moving data between databases whose tables differ. The SQLite test always runs; the others
//! need `KIYI_LIVE=1` and the Docker databases.
//!
//! The scenario: another company's system ("clients", "items", "purchases": UUID keys, one-letter
//! status codes, a single full-name field, time zones) moves into yours ("customers",
//! "products", "orders": numeric keys, first and last names, enums, UTC datetimes), which
//! already has a customer of its own.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::drivers::{self, DbDriver};
use kiyi_core::migrate::{self, ColumnMapping, IdMode, MapPair, MigrationPlan, Otherwise, Progress, Severity, Step, TableMapping, ValueSource, WriteMode};

fn live() -> bool {
    std::env::var("KIYI_LIVE").is_ok()
}

fn server(kind: DbKind, database: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: "t".into(),
        name: "t".into(),
        kind,
        host: "127.0.0.1".into(),
        port: if kind == DbKind::Postgres { 55432 } else { 53306 },
        user: if kind == DbKind::Postgres { "kiyi".into() } else { "root".into() },
        database: Some(database.into()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: None,
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    }
}

fn sqlite(name: &str) -> ConnectionConfig {
    let path = std::env::temp_dir().join(format!("kiyi-move-{name}-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    ConnectionConfig { kind: DbKind::Sqlite, host: String::new(), port: 0, user: String::new(), database: Some(path.to_string_lossy().into_owned()), driver: Some("sqlite".into()), ..server(DbKind::Sqlite, "") }
}

async fn open(c: &ConnectionConfig) -> Arc<dyn DbDriver> {
    drivers::open(c, Some("kiyi")).await.expect("connect")
}

async fn exec(db: &Arc<dyn DbDriver>, statements: &[&str]) {
    for s in statements {
        db.execute_script(&[s.to_string()], false, false).await.unwrap_or_else(|e| panic!("{s}: {e}"));
    }
}

async fn rows(db: &Arc<dyn DbDriver>, sql: &str) -> Vec<Vec<Option<String>>> {
    db.fetch(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}")).1
}

async fn one(db: &Arc<dyn DbDriver>, sql: &str) -> String {
    rows(db, sql).await[0][0].clone().unwrap_or_default()
}

fn table<'a>(plan: &'a mut MigrationPlan, target: &str) -> &'a mut TableMapping {
    plan.tables.iter_mut().find(|t| t.target_table == target).unwrap_or_else(|| panic!("no {target}"))
}

fn column<'a>(t: &'a mut TableMapping, target: &str) -> &'a mut ColumnMapping {
    t.columns.iter_mut().find(|c| c.target == target).unwrap_or_else(|| panic!("no {target}"))
}

async fn run(source: &Arc<dyn DbDriver>, target: &Arc<dyn DbDriver>, plan: &MigrationPlan, test_run: bool) -> Result<migrate::MigrationReport, String> {
    let seen = Mutex::new(Vec::<Progress>::new());
    let progress = |p: Progress| seen.lock().unwrap().push(p);
    let r = migrate::run(source.as_ref(), target.as_ref(), plan, test_run, &std::env::temp_dir(), &progress, &AtomicBool::new(false)).await.map_err(|e| e.to_string());
    if r.is_ok() {
        let seen = seen.into_inner().unwrap();
        let last = seen.last().unwrap();
        assert_eq!(last.done, last.total, "progress ends complete: {last:?}");
    }
    r
}

/// The legacy system, in PostgreSQL.
async fn legacy_postgres() -> Arc<dyn DbDriver> {
    let admin = open(&server(DbKind::Postgres, "kiyi_test")).await;
    exec(&admin, &["DROP DATABASE IF EXISTS kiyi_legacy WITH (FORCE)", "CREATE DATABASE kiyi_legacy"]).await;
    admin.close().await;
    let pg = open(&server(DbKind::Postgres, "kiyi_legacy")).await;
    exec(
        &pg,
        &[
            "CREATE TABLE clients (client_id uuid PRIMARY KEY, full_name text NOT NULL, e_posta text NOT NULL, phone text, state char(1) NOT NULL, joined timestamptz NOT NULL, vip boolean NOT NULL DEFAULT false)",
            "CREATE TABLE items (code text PRIMARY KEY, title text NOT NULL, unit_price numeric(10,2) NOT NULL, stock int, specs jsonb, tags text[])",
            "CREATE TABLE purchases (id serial PRIMARY KEY, client uuid NOT NULL REFERENCES clients, item text NOT NULL REFERENCES items, qty int NOT NULL, amount numeric(10,2) NOT NULL, placed timestamptz NOT NULL, status text NOT NULL)",
            "INSERT INTO clients VALUES
               ('11111111-1111-1111-1111-111111111111', 'Ada María Lovelace', 'ada@example.com', '+44 20 1234', 'A', '2024-03-01 09:30:00+03', true),
               ('22222222-2222-2222-2222-222222222222', 'Zeynep Yılmaz', 'zeynep@example.com', NULL, 'P', '2025-12-31 23:30:00-05', false),
               ('33333333-3333-3333-3333-333333333333', 'Cher', 'cher@example.com', '555', 'a', '2026-01-15 12:00:00+00', false)",
            "INSERT INTO items VALUES ('LAMP-1', 'Desk lamp', 89.90, 12, '{\"watts\": 9}', '{office,\"warm light\"}'), ('MUG-2', 'Mug', 12.00, NULL, NULL, NULL)",
            "INSERT INTO purchases (client, item, qty, amount, placed, status) VALUES
               ('11111111-1111-1111-1111-111111111111', 'LAMP-1', 1, 89.90, '2026-02-01 10:00:00+03', 'done'),
               ('22222222-2222-2222-2222-222222222222', 'MUG-2', 3, 36.00, '2026-02-02 08:15:00+00', 'refunded'),
               ('33333333-3333-3333-3333-333333333333', 'MUG-2', 1, 12.00, '2026-02-03 18:45:30.5+01', 'done'),
               ('11111111-1111-1111-1111-111111111111', 'MUG-2', 2, 24.00, '2026-02-04 00:00:00+00', 'done')",
        ],
    )
    .await;
    pg
}

/// Your system, in MySQL, with a customer already in it.
async fn modern_mysql() -> Arc<dyn DbDriver> {
    let root = open(&server(DbKind::Mysql, "kiyi_test")).await;
    exec(&root, &["DROP DATABASE IF EXISTS kiyi_move", "CREATE DATABASE kiyi_move"]).await;
    let my = open(&server(DbKind::Mysql, "kiyi_move")).await;
    exec(
        &my,
        &[
            "CREATE TABLE customers (id INT AUTO_INCREMENT PRIMARY KEY, first_name VARCHAR(50) NOT NULL, last_name VARCHAR(50), email VARCHAR(120) NOT NULL UNIQUE, phone VARCHAR(30), status ENUM('active','passive') NOT NULL, is_vip TINYINT(1) NOT NULL DEFAULT 0, created_at DATETIME NOT NULL)",
            "CREATE TABLE products (id INT AUTO_INCREMENT PRIMARY KEY, sku VARCHAR(20) NOT NULL UNIQUE, name VARCHAR(100) NOT NULL, price DECIMAL(10,2) NOT NULL, stock INT NOT NULL DEFAULT 0, attributes JSON, tags JSON)",
            "CREATE TABLE orders (id INT AUTO_INCREMENT PRIMARY KEY, customer_id INT NOT NULL, product_id INT NOT NULL, quantity INT NOT NULL, total DECIMAL(10,2) NOT NULL, ordered_at DATETIME NOT NULL, refunded TINYINT(1) NOT NULL DEFAULT 0,
               FOREIGN KEY (customer_id) REFERENCES customers (id), FOREIGN KEY (product_id) REFERENCES products (id))",
            "INSERT INTO customers (first_name, last_name, email, status, created_at) VALUES ('Existing', 'Customer', 'me@mine.com', 'active', '2020-01-01 00:00:00')",
        ],
    )
    .await;
    my
}

fn issues_text(c: &migrate::MigrationCheck) -> String {
    c.issues.iter().map(|i| format!("[{:?}] {} {:?}: {}", i.severity, i.table, i.column, i.message)).collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn another_companys_postgres_moves_into_your_mysql() {
    if !live() {
        return;
    }
    let (pg, my) = (legacy_postgres().await, modern_mysql().await);

    // 1. Kiyi's own suggestion.
    let mut plan = migrate::suggest(pg.as_ref(), my.as_ref(), "pg", "my").await.unwrap();
    let mut pairs: Vec<(&str, &str)> = plan.tables.iter().map(|t| (t.source_table.as_str(), t.target_table.as_str())).collect();
    pairs.sort();
    assert_eq!(pairs, vec![("clients", "customers"), ("items", "products"), ("purchases", "orders")]);
    {
        let c = table(&mut plan, "customers");
        assert_eq!(c.ids, IdMode::Renumber, "UUID keys can't go into an INT key");
        assert_eq!(column(c, "first_name").steps, vec![Step::Split { separator: " ".into(), part: 1, rest: false }]);
        assert_eq!(column(c, "last_name").source, ValueSource::Column { column: "full_name".into() });
        assert_eq!(column(c, "email").source, ValueSource::Column { column: "e_posta".into() });
        assert_eq!(column(c, "status").source, ValueSource::Column { column: "state".into() });
        assert_eq!(column(c, "is_vip").source, ValueSource::Column { column: "vip".into() });
        assert_eq!(column(c, "created_at").source, ValueSource::Column { column: "joined".into() });
        let o = table(&mut plan, "orders");
        assert_eq!(o.ids, IdMode::Keep, "integer keys into an empty table are kept");
        assert_eq!(column(o, "customer_id").source, ValueSource::Reference { column: "client".into(), schema: Some("public".into()), table: "clients".into() });
        assert_eq!(column(o, "product_id").source, ValueSource::Reference { column: "item".into(), schema: Some("public".into()), table: "items".into() });
        assert_eq!(column(o, "quantity").source, ValueSource::Column { column: "qty".into() });
        assert_eq!(column(o, "total").source, ValueSource::Column { column: "amount".into() });
        assert_eq!(column(o, "ordered_at").source, ValueSource::Default, "“placed” isn't a name Kiyi knows");
    }

    // 2. The check finds what a person has to decide.
    let check = migrate::check(pg.as_ref(), my.as_ref(), &plan).await.unwrap();
    assert!(!check.ready);
    assert_eq!(check.order, vec!["customers", "products", "orders"]);
    let text = issues_text(&check);
    assert!(text.contains("status: all 3 rows checked") && text.contains("allowed values (active, passive)"), "{text}");
    assert!(text.contains("ordered_at is required but gets no value"), "{text}");

    // 3. They decide.
    {
        let c = table(&mut plan, "customers");
        column(c, "status").steps = vec![Step::Map { pairs: vec![MapPair { from: "A".into(), to: Some("active".into()) }, MapPair { from: "P".into(), to: Some("passive".into()) }], otherwise: Otherwise::Keep }];
        let o = table(&mut plan, "orders");
        column(o, "ordered_at").source = ValueSource::Column { column: "placed".into() };
        let r = column(o, "refunded");
        r.source = ValueSource::Column { column: "status".into() };
        r.steps = vec![Step::Map { pairs: vec![MapPair { from: "refunded".into(), to: Some("yes".into()) }], otherwise: Otherwise::Value { value: "no".into() } }];
    }
    let check = migrate::check(pg.as_ref(), my.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    let customers = check.tables.iter().find(|t| t.target == "customers").unwrap();
    assert_eq!(customers.rows, 3);
    let at = |name: &str| customers.columns.iter().position(|c| c == name).unwrap();
    let ada = &customers.preview[0];
    assert_eq!(ada[at("id")].as_deref(), Some("2"), "numbered after the existing customer");
    assert_eq!((ada[at("first_name")].as_deref(), ada[at("last_name")].as_deref()), (Some("Ada"), Some("María Lovelace")));
    assert_eq!(ada[at("created_at")].as_deref(), Some("2024-03-01 06:30:00"), "converted to UTC");
    assert_eq!(ada[at("is_vip")].as_deref(), Some("1"));

    // 4. A test run writes everything and keeps nothing.
    let report = run(&pg, &my, &plan, true).await.unwrap();
    assert!(report.test_run);
    assert_eq!(report.rows, 9);
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "1");
    assert_eq!(one(&my, "SELECT COUNT(*) FROM orders").await, "0");

    // 5. The real move.
    let report = run(&pg, &my, &plan, false).await.unwrap();
    assert_eq!(report.rows, 9);
    assert_eq!(report.tables.iter().map(|t| t.rows).collect::<Vec<_>>(), vec![3, 2, 4]);
    let got = rows(
        &my,
        "SELECT c.first_name, c.last_name, c.status, c.is_vip, CAST(c.created_at AS CHAR), p.sku, p.name, CAST(p.tags AS CHAR), o.quantity, CAST(o.total AS CHAR), CAST(o.ordered_at AS CHAR), o.refunded
         FROM orders o JOIN customers c ON c.id = o.customer_id JOIN products p ON p.id = o.product_id ORDER BY o.id",
    )
    .await;
    let s = |r: &Vec<Option<String>>| r.iter().map(|v| v.clone().unwrap_or("NULL".into())).collect::<Vec<_>>().join("|");
    assert_eq!(s(&got[0]), r#"Ada|María Lovelace|active|1|2024-03-01 06:30:00|LAMP-1|Desk lamp|["office", "warm light"]|1|89.90|2026-02-01 07:00:00|0"#);
    assert_eq!(s(&got[1]), "Zeynep|Yılmaz|passive|0|2026-01-01 04:30:00|MUG-2|Mug|NULL|3|36.00|2026-02-02 08:15:00|1");
    assert_eq!(s(&got[2]), "Cher||active|0|2026-01-15 12:00:00|MUG-2|Mug|NULL|1|12.00|2026-02-03 17:45:30|0");
    assert_eq!(one(&my, "SELECT stock FROM products WHERE sku = 'MUG-2'").await, "0", "a missing required value takes the column's default");
    // The database keeps numbering after the moved rows.
    exec(&my, &["INSERT INTO customers (first_name, email, status, created_at) VALUES ('New', 'new@mine.com', 'active', NOW())"]).await;
    assert_eq!(one(&my, "SELECT MAX(id) FROM customers").await, "5");

    // 6. Running it again with "skip existing" adds nothing.
    for (t, key) in [("customers", "email"), ("products", "sku"), ("orders", "id")] {
        let m = table(&mut plan, t);
        m.write = WriteMode::Skip;
        m.match_on = vec![key.into()];
    }
    let check = migrate::check(pg.as_ref(), my.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    run(&pg, &my, &plan, false).await.unwrap();
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "5");
    assert_eq!(one(&my, "SELECT COUNT(*) FROM orders").await, "4");

    // …and "update existing" refreshes values in place.
    exec(&pg, &["UPDATE items SET unit_price = 99.00 WHERE code = 'LAMP-1'"]).await;
    let p = table(&mut plan, "products");
    p.write = WriteMode::Update;
    run(&pg, &my, &plan, false).await.unwrap();
    assert_eq!(one(&my, "SELECT CAST(price AS CHAR) FROM products WHERE sku = 'LAMP-1'").await, "99.00");
    assert_eq!(one(&my, "SELECT COUNT(*) FROM products").await, "2");

    // Orders moved again later link to the customers and products that are already there,
    // not to numbers that were never written.
    exec(&my, &["DELETE FROM orders"]).await;
    table(&mut plan, "products").write = WriteMode::Skip;
    table(&mut plan, "orders").write = WriteMode::Insert;
    run(&pg, &my, &plan, false).await.unwrap();
    let linked = rows(&my, "SELECT c.email, p.sku FROM orders o JOIN customers c ON c.id = o.customer_id JOIN products p ON p.id = o.product_id ORDER BY o.id").await;
    let linked: Vec<String> = linked.iter().map(|r| format!("{}/{}", r[0].clone().unwrap(), r[1].clone().unwrap())).collect();
    assert_eq!(linked, vec!["ada@example.com/LAMP-1", "zeynep@example.com/MUG-2", "cher@example.com/MUG-2", "ada@example.com/MUG-2"]);
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "5", "no customer was added twice");

    // 7. A failure leaves nothing behind, and says where it happened.
    exec(&my, &["DELETE FROM orders", "DELETE FROM products", "DELETE FROM customers WHERE email <> 'me@mine.com'"]).await;
    exec(&pg, &["INSERT INTO clients VALUES ('44444444-4444-4444-4444-444444444444', 'Ada Again', 'ADA@example.com', NULL, 'A', now(), false)"]).await;
    for t in ["customers", "products", "orders"] {
        table(&mut plan, t).write = WriteMode::Insert;
    }
    column(table(&mut plan, "customers"), "email").steps = vec![Step::Lower];
    let err = run(&pg, &my, &plan, false).await.unwrap_err();
    assert!(err.starts_with("Nothing was moved. clients → customers, rows 1–4:") && err.contains("Duplicate entry"), "{err}");
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "1");
    assert_eq!(one(&my, "SELECT COUNT(*) FROM products").await, "0");

    // A value that can't be converted stops it before anything is written.
    exec(&pg, &["DELETE FROM clients WHERE client_id = '44444444-4444-4444-4444-444444444444'", "UPDATE clients SET state = 'X' WHERE full_name = 'Cher'"]).await;
    let err = run(&pg, &my, &plan, false).await.unwrap_err();
    assert!(err.contains("the row with key 33333333-3333-3333-3333-333333333333: status “X” isn't one of the allowed values"), "{err}");

    // Plans are saved and opened as files.
    let path = std::env::temp_dir().join(format!("kiyi-plan-{}.json", std::process::id()));
    migrate::save(&plan, &path).unwrap();
    assert_eq!(migrate::open(&path).unwrap().tables, plan.tables);
}

/// Your MySQL into a PostgreSQL with identity keys, timestamptz and booleans: the sequence must
/// continue after the moved rows.
#[tokio::test]
async fn mysql_moves_into_postgres_identity_columns() {
    if !live() {
        return;
    }
    let my = open(&server(DbKind::Mysql, "kiyi_test")).await;
    exec(
        &my,
        &[
            "DROP TABLE IF EXISTS move_people",
            "CREATE TABLE move_people (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(50) NOT NULL, active TINYINT(1) NOT NULL, born DATE, seen DATETIME(3), data JSON, photo BLOB)",
            "INSERT INTO move_people (name, active, born, seen, data, photo) VALUES ('Ana', 1, '1990-05-01', '2026-01-02 03:04:05.678', '{\"a\": 1}', X'89504E47'), ('Bo', 0, NULL, NULL, NULL, NULL)",
        ],
    )
    .await;
    let pg = open(&server(DbKind::Postgres, "kiyi_test")).await;
    exec(&pg, &["DROP TABLE IF EXISTS move_people", "CREATE TABLE move_people (id int GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name text NOT NULL, active boolean NOT NULL, born date, seen timestamptz, data jsonb, photo bytea)"]).await;
    let mut plan = migrate::suggest(my.as_ref(), pg.as_ref(), "my", "pg").await.unwrap();
    plan.tables.retain(|t| t.target_table == "move_people");
    assert_eq!(plan.tables[0].ids, IdMode::Keep);
    assert_eq!(column(&mut plan.tables[0], "id").source, ValueSource::Column { column: "id".into() });
    let check = migrate::check(my.as_ref(), pg.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    run(&my, &pg, &plan, false).await.unwrap();
    let got = rows(&pg, "SELECT id, name, active::text, born::text, seen AT TIME ZONE 'UTC', data::text, encode(photo, 'hex') FROM move_people ORDER BY id").await;
    assert_eq!(got[0], vec![Some("1".into()), Some("Ana".into()), Some("true".into()), Some("1990-05-01".into()), Some("2026-01-02 03:04:05.678".into()), Some(r#"{"a": 1}"#.into()), Some("89504e47".into())]);
    assert_eq!(got[1][2].as_deref(), Some("false"));
    exec(&pg, &["INSERT INTO move_people (name, active) VALUES ('Cy', true)"]).await;
    assert_eq!(one(&pg, "SELECT max(id) FROM move_people").await, "3", "the identity continues after the moved keys");
    exec(&pg, &["DROP TABLE move_people"]).await;
    exec(&my, &["DROP TABLE move_people"]).await;
}

/// Without any server: the sample store into a differently shaped SQLite database, with renumbering,
/// links, skipping and updating. Runs everywhere, including CI without Docker.
#[tokio::test]
async fn sqlite_store_moves_into_a_different_shape() {
    let src_cfg = sqlite("src");
    let dst_cfg = sqlite("dst");
    let src = open(&src_cfg).await;
    src.execute_script(&kiyi_core::sample::statements(), true, false).await.unwrap();
    let dst = open(&dst_cfg).await;
    exec(
        &dst,
        &[
            "CREATE TABLE clients (id INTEGER PRIMARY KEY AUTOINCREMENT, full_name TEXT NOT NULL, email TEXT NOT NULL UNIQUE, tier TEXT NOT NULL CHECK (tier IN ('basic','premium')), since DATE NOT NULL)",
            "CREATE TABLE purchases (id INTEGER PRIMARY KEY AUTOINCREMENT, client_id INTEGER NOT NULL REFERENCES clients(id), amount NUMERIC NOT NULL, status TEXT NOT NULL DEFAULT 'new')",
            "INSERT INTO clients (full_name, email, tier, since) VALUES ('Already Here', 'here@x.io', 'basic', '2020-01-01')",
        ],
    )
    .await;
    let mut plan = migrate::suggest(src.as_ref(), dst.as_ref(), "a", "b").await.unwrap();
    let pairs: Vec<(&str, &str)> = plan.tables.iter().map(|t| (t.source_table.as_str(), t.target_table.as_str())).collect();
    assert_eq!(pairs, vec![("customers", "clients"), ("orders", "purchases")]);
    {
        let c = table(&mut plan, "clients");
        assert_eq!(c.ids, IdMode::Renumber, "the target already has rows");
        assert_eq!(column(c, "full_name").source, ValueSource::Column { column: "name".into() });
        assert_eq!(column(c, "since").source, ValueSource::Column { column: "signed_up_at".into() });
        let tier = column(c, "tier");
        tier.source = ValueSource::Column { column: "plan".into() };
        tier.steps = vec![Step::Map { pairs: vec![MapPair { from: "pro".into(), to: Some("premium".into()) }, MapPair { from: "enterprise".into(), to: Some("premium".into()) }], otherwise: Otherwise::Value { value: "basic".into() } }];
        let p = table(&mut plan, "purchases");
        assert_eq!(column(p, "client_id").source, ValueSource::Reference { column: "customer_id".into(), schema: None, table: "customers".into() });
        assert_eq!(column(p, "amount").source, ValueSource::Column { column: "total".into() });
        assert_eq!(column(p, "status").source, ValueSource::Column { column: "status".into() });
        // Keep only paid orders' status, the rest fall back to the default.
        column(p, "status").steps = vec![Step::Map { pairs: vec![MapPair { from: "paid".into(), to: Some("paid".into()) }], otherwise: Otherwise::Null }];
    }
    let check = migrate::check(src.as_ref(), dst.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    assert!(check.issues.iter().all(|i| i.severity != Severity::Error));
    let report = run(&src, &dst, &plan, false).await.unwrap();
    assert_eq!(report.rows, 1200 + 9000);
    assert_eq!(one(&dst, "SELECT COUNT(*) FROM clients").await, "1201");
    assert_eq!(one(&dst, "SELECT MIN(id) FROM clients WHERE email <> 'here@x.io'").await, "2");
    // Every purchase points at the client it belonged to.
    let src_spent = one(&src, "SELECT CAST(ROUND(SUM(o.total), 2) AS TEXT) FROM orders o JOIN customers c ON c.id = o.customer_id WHERE c.email = 'leo.okafor1@mail.dev'").await;
    let dst_spent = one(&dst, "SELECT CAST(ROUND(SUM(p.amount), 2) AS TEXT) FROM purchases p JOIN clients c ON c.id = p.client_id WHERE c.email = 'leo.okafor1@mail.dev'").await;
    assert_eq!(src_spent, dst_spent);
    assert_eq!(one(&dst, "SELECT COUNT(*) FROM purchases WHERE status = 'new'").await, one(&src, "SELECT COUNT(*) FROM orders WHERE status <> 'paid'").await, "missing required values take the default");
    assert_eq!(one(&dst, "SELECT since FROM clients WHERE id = 2").await.len(), 10, "a date, not a date and time");

    // Update existing clients by email: no duplicates, values refreshed.
    exec(&src, &["UPDATE customers SET name = 'Renamed Person' WHERE id = 1"]).await;
    let c = table(&mut plan, "clients");
    c.write = WriteMode::Update;
    c.match_on = vec!["email".into()];
    c.ids = IdMode::Keep;
    column(c, "id").source = ValueSource::Default;
    table(&mut plan, "purchases").enabled = false;
    let check = migrate::check(src.as_ref(), dst.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    run(&src, &dst, &plan, false).await.unwrap();
    assert_eq!(one(&dst, "SELECT COUNT(*) FROM clients").await, "1201");
    assert_eq!(one(&dst, "SELECT full_name FROM clients WHERE email = (SELECT email FROM clients ORDER BY id LIMIT 1 OFFSET 1)").await, "Renamed Person");

    // Stopping midway keeps nothing.
    let cancel = AtomicBool::new(true);
    let err = migrate::run(src.as_ref(), dst.as_ref(), &plan, false, &std::env::temp_dir(), &|_| {}, &cancel).await.unwrap_err();
    assert!(err.to_string().contains("cancelled") || err.to_string().contains("Stopped"), "{err}");
}

/// The AI path against a local stand-in for an OpenAI-compatible server: what's sent (structure
/// only, unless examples are allowed), and how its answer becomes a plan that checks out.
#[tokio::test]
async fn the_ai_improves_a_plan_and_sees_rows_only_when_allowed() {
    use axum::routing::post;
    use axum::{Json, Router};
    use kiyi_core::ai::{AiProvider, ProviderKind};
    use serde_json::{json, Value};

    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = requests.clone();
    let answer = json!({
        "explanation": "Customers become clients; plans map to tiers.",
        "tables": [{ "sourceTable": "customers", "targetTable": "clients", "ids": "renumber", "columns": [
            { "target": "full_name", "from": "column", "columns": ["name"], "separator": "", "value": null, "table": "", "steps": [
                { "op": "trim", "find": "", "with": "", "separator": "", "part": 0, "rest": false, "pairs": [], "otherwise": "keep", "value": null } ] },
            { "target": "email", "from": "column", "columns": ["email"], "separator": "", "value": null, "table": "", "steps": [] },
            { "target": "tier", "from": "column", "columns": ["plan"], "separator": "", "value": null, "table": "", "steps": [
                { "op": "map", "find": "", "with": "", "separator": "", "part": 0, "rest": false, "pairs": [{ "from": "pro", "to": "premium" }, { "from": "enterprise", "to": "premium" }], "otherwise": "value", "value": "basic" } ] },
            { "target": "since", "from": "column", "columns": ["signed_up_at"], "separator": "", "value": null, "table": "", "steps": [] }
        ]}]
    });
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let seen = seen.clone();
            let answer = answer.clone();
            async move {
                seen.lock().unwrap().push(body["messages"][1]["content"].as_str().unwrap_or_default().to_string());
                Json(json!({ "choices": [{ "message": { "content": answer.to_string() } }] }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = AiProvider { id: "t".into(), name: "Local".into(), kind: ProviderKind::OpenAi, base_url, model: "m".into(), preset: Some("custom".into()) };

    let src = open(&sqlite("ai-src")).await;
    src.execute_script(&kiyi_core::sample::statements(), true, false).await.unwrap();
    let dst = open(&sqlite("ai-dst")).await;
    exec(&dst, &["CREATE TABLE clients (id INTEGER PRIMARY KEY AUTOINCREMENT, full_name TEXT NOT NULL, email TEXT NOT NULL UNIQUE, tier TEXT NOT NULL, since DATE NOT NULL)"]).await;
    let empty = MigrationPlan { source: "a".into(), target: "b".into(), source_name: String::new(), target_name: String::new(), tables: vec![] };

    let (plan, explanation) = migrate::improve_with_ai(&provider, None, src.as_ref(), dst.as_ref(), &empty, false).await.unwrap();
    assert_eq!(explanation, "Customers become clients; plans map to tiers.");
    let sent = requests.lock().unwrap()[0].clone();
    assert!(sent.contains("customers") && sent.contains("signed_up_at") && sent.contains("required"), "the structure is sent");
    assert!(!sent.contains("@mail.dev") && !sent.contains("e.g."), "no row data without permission:\n{sent}");
    let check = migrate::check(src.as_ref(), dst.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    let tier = check.tables[0].columns.iter().position(|c| c == "tier").unwrap();
    assert!(check.tables[0].preview.iter().all(|r| matches!(r[tier].as_deref(), Some("basic" | "premium"))));

    migrate::improve_with_ai(&provider, None, src.as_ref(), dst.as_ref(), &empty, true).await.unwrap();
    let sent = requests.lock().unwrap()[1].clone();
    assert!(sent.contains("e.g.") && sent.contains("@"), "examples when allowed");
}

fn self_link_plan(source_table: &str, target_table: &str, ids: IdMode, link: ValueSource) -> MigrationPlan {
    let col = |target: &str, source: ValueSource| ColumnMapping { target: target.into(), source, steps: vec![] };
    MigrationPlan {
        source: "a".into(),
        target: "b".into(),
        source_name: String::new(),
        target_name: String::new(),
        tables: vec![TableMapping {
            enabled: true,
            source_schema: None,
            source_table: source_table.into(),
            target_schema: None,
            target_table: target_table.into(),
            columns: vec![
                col("id", if ids == IdMode::Keep { ValueSource::Column { column: "id".into() } } else { ValueSource::Default }),
                col("name", ValueSource::Column { column: "name".into() }),
                col("boss_id", link),
            ],
            ids,
            write: WriteMode::Insert,
            match_on: vec![],
        }],
    }
}

/// Rows that point at other rows of the same table (a manager, a parent category), in an order
/// where the one pointed at comes later: written empty first, then linked once all exist.
#[tokio::test]
async fn rows_linking_to_later_rows_of_the_same_table() {
    let src = open(&sqlite("self-src")).await;
    exec(
        &src,
        &[
            "CREATE TABLE staff (id INTEGER PRIMARY KEY, name TEXT NOT NULL, manager_id INTEGER REFERENCES staff(id))",
            "PRAGMA foreign_keys = OFF",
            // 1 and 2 report to 3, who reports to 4 (the boss): every link points forward.
            "INSERT INTO staff VALUES (1, 'Ada', 3), (2, 'Bo', 3), (3, 'Cy', 4), (4, 'Di', NULL)",
        ],
    )
    .await;
    let dst = open(&sqlite("self-dst")).await;
    exec(&dst, &["CREATE TABLE people (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, boss_id INTEGER REFERENCES people(id))", "INSERT INTO people (name) VALUES ('Already here')"]).await;
    // Foreign keys are enforced, so a forward link written as-is would fail.
    assert!(dst.execute_script(&["INSERT INTO people (name, boss_id) VALUES ('x', 999)".into()], false, false).await.is_err());

    let plan = self_link_plan("staff", "people", IdMode::Renumber, ValueSource::Reference { column: "manager_id".into(), schema: None, table: "staff".into() });
    let check = migrate::check(src.as_ref(), dst.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    run(&src, &dst, &plan, false).await.unwrap();
    let got = rows(&dst, "SELECT p.name, b.name FROM people p LEFT JOIN people b ON b.id = p.boss_id WHERE p.id > 1 ORDER BY p.id").await;
    let got: Vec<String> = got.iter().map(|r| format!("{}→{}", r[0].clone().unwrap(), r[1].clone().unwrap_or("-".into()))).collect();
    assert_eq!(got, vec!["Ada→Cy", "Bo→Cy", "Cy→Di", "Di→-"]);

    // A link that can't be empty is reported before anything runs.
    let strict = open(&sqlite("self-strict")).await;
    exec(&strict, &["CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, boss_id INTEGER NOT NULL REFERENCES people(id))"]).await;
    let plan = self_link_plan("staff", "people", IdMode::Keep, ValueSource::Column { column: "manager_id".into() });
    let check = migrate::check(src.as_ref(), strict.as_ref(), &plan).await.unwrap();
    assert!(issues_text(&check).contains("can't be empty, so each row has to come after the one it points to"), "{}", issues_text(&check));
}

#[tokio::test]
async fn postgres_categories_link_to_parents_that_come_later() {
    if !live() {
        return;
    }
    let pg = open(&server(DbKind::Postgres, "kiyi_test")).await;
    exec(
        &pg,
        &[
            "DROP TABLE IF EXISTS move_tree_src, move_tree_dst",
            "CREATE TABLE move_tree_src (id int PRIMARY KEY, name text NOT NULL, manager_id int)",
            "INSERT INTO move_tree_src VALUES (1, 'Lamps', 3), (2, 'Mugs', 3), (3, 'Home', NULL)",
            "CREATE TABLE move_tree_dst (id int GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, name text NOT NULL, boss_id int REFERENCES move_tree_dst(id))",
        ],
    )
    .await;
    let plan = self_link_plan("move_tree_src", "move_tree_dst", IdMode::Keep, ValueSource::Column { column: "manager_id".into() });
    let mut plan = plan;
    plan.tables[0].source_schema = Some("public".into());
    plan.tables[0].target_schema = Some("public".into());
    let check = migrate::check(pg.as_ref(), pg.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    run(&pg, &pg, &plan, false).await.unwrap();
    let got = rows(&pg, "SELECT c.name, p.name FROM move_tree_dst c LEFT JOIN move_tree_dst p ON p.id = c.boss_id ORDER BY c.id").await;
    assert_eq!(got[0], vec![Some("Lamps".into()), Some("Home".into())]);
    assert_eq!(got[2], vec![Some("Home".into()), None]);
    exec(&pg, &["DROP TABLE move_tree_src, move_tree_dst"]).await;
}

fn col(target: &str, source: &str) -> ColumnMapping {
    ColumnMapping { target: target.into(), source: ValueSource::Column { column: source.into() }, steps: vec![] }
}

fn mapping(source_schema: Option<&str>, source: &str, target: &str, columns: Vec<ColumnMapping>) -> TableMapping {
    TableMapping {
        enabled: true,
        source_schema: source_schema.map(str::to_string),
        source_table: source.into(),
        target_schema: None,
        target_table: target.into(),
        columns,
        ids: IdMode::Keep,
        write: WriteMode::Insert,
        match_on: vec![],
    }
}

/// Whether a row already exists is the target's call: a case-sensitive unique column keeps
/// "ada@x.com" apart from "Ada@x.com", a NOCASE one doesn't, and either way the move goes through.
/// Rows repeating an earlier row are moved once and links to them follow; a two-column key pages.
#[tokio::test]
async fn existing_rows_are_matched_the_way_the_target_compares_them() {
    let src = open(&sqlite("match-src")).await;
    exec(
        &src,
        &[
            "CREATE TABLE people (id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
            "INSERT INTO people VALUES (10, 'ada@x.com'), (11, 'bob@x.com'), (12, 'bob@x.com'), (13, 'Cem@x.com')",
            "CREATE TABLE notes (id INTEGER PRIMARY KEY, person_id INTEGER NOT NULL, body TEXT)",
            "INSERT INTO notes VALUES (1, 10, 'a'), (2, 12, 'b'), (3, 13, 'c')",
            "CREATE TABLE lines (a INTEGER NOT NULL, b INTEGER NOT NULL, qty INTEGER NOT NULL, PRIMARY KEY (a, b))",
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4500) INSERT INTO lines SELECT i % 3, i, i * 2 FROM n",
        ],
    )
    .await;
    for (collate, people) in [("", 5), (" COLLATE NOCASE", 3)] {
        let dst = open(&sqlite(if collate.is_empty() { "match-cs" } else { "match-ci" })).await;
        exec(
            &dst,
            &[
                &format!("CREATE TABLE people (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE{collate})"),
                "INSERT INTO people VALUES (1, 'Ada@x.com'), (2, 'cem@x.com')",
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, person_id INTEGER NOT NULL REFERENCES people (id), body TEXT)",
                "CREATE TABLE lines (a INTEGER NOT NULL, b INTEGER NOT NULL, qty INTEGER NOT NULL, PRIMARY KEY (a, b))",
            ],
        )
        .await;
        let mut people_map = mapping(None, "people", "people", vec![col("id", "id"), col("email", "email")]);
        people_map.ids = IdMode::Renumber;
        people_map.write = WriteMode::Skip;
        people_map.match_on = vec!["email".into()];
        let notes = mapping(
            None,
            "notes",
            "notes",
            vec![ColumnMapping { target: "person_id".into(), source: ValueSource::Reference { column: "person_id".into(), schema: None, table: "people".into() }, steps: vec![] }, col("body", "body")],
        );
        let lines = mapping(None, "lines", "lines", vec![col("a", "a"), col("b", "b"), col("qty", "qty")]);
        let plan = MigrationPlan { source: "s".into(), target: "t".into(), source_name: String::new(), target_name: String::new(), tables: vec![notes, people_map, lines] };
        let check = migrate::check(src.as_ref(), dst.as_ref(), &plan).await.unwrap();
        assert!(check.ready, "{}", issues_text(&check));
        assert!(issues_text(&check).contains("repeat the email"), "{}", issues_text(&check));
        let report = run(&src, &dst, &plan, false).await.unwrap_or_else(|e| panic!("{collate}: {e}"));
        assert_eq!(report.tables.iter().find(|t| t.target == "people").unwrap().repeated, 1);
        assert_eq!(one(&dst, "SELECT COUNT(*) FROM people").await, people.to_string(), "{collate}");
        let owner = |body: &str| format!("SELECT p.email FROM notes n JOIN people p ON p.id = n.person_id WHERE n.body = '{body}'");
        assert_eq!(one(&dst, &owner("a")).await, if collate.is_empty() { "ada@x.com" } else { "Ada@x.com" });
        assert_eq!(one(&dst, &owner("b")).await, "bob@x.com", "a link to the repeated row follows the first one");
        assert_eq!(one(&dst, &owner("c")).await, if collate.is_empty() { "Cem@x.com" } else { "cem@x.com" });
        assert_eq!(one(&dst, "SELECT COUNT(*) || ' ' || SUM(qty) FROM lines").await, one(&src, "SELECT COUNT(*) || ' ' || SUM(qty) FROM lines").await);
    }
}

fn sqlserver_live() -> bool {
    std::env::var("KIYI_LIVE_SQLSERVER").is_ok() && live()
}

fn sqlserver(database: &str) -> ConnectionConfig {
    ConnectionConfig { kind: DbKind::Sqlserver, port: 51433, user: "sa".into(), ssl_mode: SslMode::Prefer, driver: Some("sqlserver".into()), ..server(DbKind::Sqlserver, database) }
}

/// A legacy SQL Server system (Turkish names, IDENTITY keys, bit, money, uniqueidentifier,
/// datetimeoffset, ntext, a table without a key, a two-column key) into a MySQL schema that
/// already has a customer.
#[tokio::test]
async fn sql_server_moves_into_mysql() {
    if !sqlserver_live() {
        return;
    }
    let master = drivers::open(&sqlserver("master"), Some("Kiyi_pass1")).await.expect("connect to SQL Server");
    exec(
        &master,
        &["IF DB_ID('kiyi_move_src') IS NOT NULL ALTER DATABASE kiyi_move_src SET SINGLE_USER WITH ROLLBACK IMMEDIATE", "DROP DATABASE IF EXISTS kiyi_move_src", "CREATE DATABASE kiyi_move_src"],
    )
    .await;
    let ms = drivers::open(&sqlserver("kiyi_move_src"), Some("Kiyi_pass1")).await.expect("connect to SQL Server");
    exec(
        &ms,
        &[
            "CREATE TABLE dbo.Musteriler (MusteriID int IDENTITY PRIMARY KEY, AdSoyad nvarchar(100) NOT NULL, Eposta nvarchar(120) NOT NULL, Aktif bit NOT NULL, Notlar ntext NULL, Kayit datetime2(7) NULL, Kimlik uniqueidentifier NOT NULL, Puan tinyint NULL)",
            "INSERT INTO dbo.Musteriler (AdSoyad, Eposta, Aktif, Notlar, Kayit, Kimlik, Puan) VALUES
               (N'Şükrü Öğün', N'sukru@x.com', 1, N'çok iyi müşteri 😀', '2024-01-02 03:04:05.1234567', '6F9619FF-8B86-D011-B42D-00C04FC964FF', 200),
               (N'Me Again', N'ME@mine.com', 0, NULL, NULL, '11111111-2222-3333-4444-555555555555', NULL),
               (N'Dup One', N'dup@x.com', 1, NULL, '2023-05-06', 'AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE', 5),
               (N'Dup Two', N'dup@x.com', 0, NULL, NULL, 'AAAAAAAA-BBBB-CCCC-DDDD-FFFFFFFFFFFF', 6),
               (N'Ayşe Çelik', N'ayse@x.com', 0, N'', '1999-12-31 23:59:59', '12345678-1234-1234-1234-123456789012', 0)",
            "CREATE TABLE dbo.Siparisler (SiparisID int IDENTITY PRIMARY KEY, MusteriID int NOT NULL REFERENCES dbo.Musteriler, Tutar money NOT NULL, Tarih datetimeoffset NOT NULL, Durum char(1) NOT NULL)",
            "INSERT INTO dbo.Siparisler (MusteriID, Tutar, Tarih, Durum) VALUES
               (1, 1234.5678, '2026-02-01 10:00:00 +03:00', 'T'), (2, 10.00, '2026-02-02 08:15:00 +00:00', 'I'), (4, 99.99, '2026-02-03 18:45:30.5 +01:00', 'T')",
            "CREATE TABLE dbo.SiparisKalem (SiparisID int NOT NULL REFERENCES dbo.Siparisler, Sira int NOT NULL, Adet int NOT NULL, PRIMARY KEY (SiparisID, Sira))",
            "WITH n AS (SELECT TOP 5000 ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS i FROM sys.all_objects a CROSS JOIN sys.all_objects b)
               INSERT INTO dbo.SiparisKalem SELECT i % 3 + 1, i, i % 7 FROM n",
            "CREATE TABLE dbo.Kayitlar (Mesaj ntext NULL, Zaman datetime NOT NULL)",
            "WITH n AS (SELECT TOP 4500 ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS i FROM sys.all_objects a CROSS JOIN sys.all_objects b)
               INSERT INTO dbo.Kayitlar SELECT N'kayıt ' + CAST(i AS nvarchar(10)), DATEADD(second, i, '2025-01-01') FROM n",
        ],
    )
    .await;

    let root = open(&server(DbKind::Mysql, "kiyi_test")).await;
    exec(&root, &["DROP DATABASE IF EXISTS kiyi_from_mssql", "CREATE DATABASE kiyi_from_mssql"]).await;
    let my = open(&server(DbKind::Mysql, "kiyi_from_mssql")).await;
    exec(
        &my,
        &[
            "CREATE TABLE customers (id INT AUTO_INCREMENT PRIMARY KEY, full_name VARCHAR(100) NOT NULL, email VARCHAR(120) NOT NULL UNIQUE, active BIT(1) NOT NULL, notes TEXT, created_at DATETIME(3), uid BINARY(16) NOT NULL, score TINYINT UNSIGNED)",
            "INSERT INTO customers (full_name, email, active, uid) VALUES ('Existing', 'me@mine.com', b'1', UUID_TO_BIN(UUID()))",
            "CREATE TABLE orders (id INT AUTO_INCREMENT PRIMARY KEY, customer_id INT NOT NULL, amount DECIMAL(12,4) NOT NULL, placed_at DATETIME NOT NULL, status ENUM('done','cancelled') NOT NULL, FOREIGN KEY (customer_id) REFERENCES customers (id))",
            "CREATE TABLE order_lines (order_id INT NOT NULL, line INT NOT NULL, qty INT NOT NULL, PRIMARY KEY (order_id, line), FOREIGN KEY (order_id) REFERENCES orders (id))",
            "CREATE TABLE logs (message LONGTEXT, at DATETIME NOT NULL)",
        ],
    )
    .await;

    let dbo = Some("dbo");
    let mut customers = mapping(
        dbo,
        "Musteriler",
        "customers",
        vec![col("id", "MusteriID"), col("full_name", "AdSoyad"), col("email", "Eposta"), col("active", "Aktif"), col("notes", "Notlar"), col("created_at", "Kayit"), col("uid", "Kimlik"), col("score", "Puan")],
    );
    customers.ids = IdMode::Renumber;
    customers.write = WriteMode::Skip;
    customers.match_on = vec!["email".into()];
    let reference = |target: &str, column: &str, table: &str| ColumnMapping { target: target.into(), source: ValueSource::Reference { column: column.into(), schema: Some("dbo".into()), table: table.into() }, steps: vec![] };
    let mut orders = mapping(dbo, "Siparisler", "orders", vec![col("id", "SiparisID"), reference("customer_id", "MusteriID", "Musteriler"), col("amount", "Tutar"), col("placed_at", "Tarih"), col("status", "Durum")]);
    orders.ids = IdMode::Renumber;
    orders.columns[4].steps = vec![Step::Map { pairs: vec![MapPair { from: "T".into(), to: Some("done".into()) }, MapPair { from: "I".into(), to: Some("cancelled".into()) }], otherwise: Otherwise::Keep }];
    let lines = mapping(dbo, "SiparisKalem", "order_lines", vec![reference("order_id", "SiparisID", "Siparisler"), col("line", "Sira"), col("qty", "Adet")]);
    let logs = mapping(dbo, "Kayitlar", "logs", vec![col("message", "Mesaj"), col("at", "Zaman")]);
    let plan = MigrationPlan { source: "ms".into(), target: "my".into(), source_name: String::new(), target_name: String::new(), tables: vec![logs, lines, orders, customers] };

    let check = migrate::check(ms.as_ref(), my.as_ref(), &plan).await.unwrap();
    assert!(check.ready, "{}", issues_text(&check));
    let at = |t: &str| check.order.iter().position(|o| o == t).unwrap();
    assert!(at("customers") < at("orders") && at("orders") < at("order_lines"), "referenced tables first: {:?}", check.order);
    let text = issues_text(&check);
    assert!(text.contains("Kayitlar has no key"), "{text}");
    assert!(text.contains("repeat the email"), "{text}");

    let test = run(&ms, &my, &plan, true).await.unwrap();
    assert!(test.test_run);
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "1", "a test run keeps nothing");

    let report = run(&ms, &my, &plan, false).await.unwrap();
    assert_eq!(report.rows, 4 + 3 + 5000 + 4500, "rows sent; MySQL skips the existing customer itself");
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "4", "ME@mine.com is the existing customer; dup@x.com moves once");
    assert_eq!(one(&my, "SELECT CONCAT(full_name, '|', notes, '|', active + 0, '|', created_at, '|', HEX(uid), '|', score) FROM customers WHERE email = 'sukru@x.com'").await, "Şükrü Öğün|çok iyi müşteri 😀|1|2024-01-02 03:04:05.123|6F9619FF8B86D011B42D00C04FC964FF|200");
    assert_eq!(one(&my, "SELECT CONCAT(active + 0, '|', notes = '', '|', score) FROM customers WHERE email = 'ayse@x.com'").await, "0|1|0");
    let owner = |amount: &str| format!("SELECT c.email FROM orders o JOIN customers c ON c.id = o.customer_id WHERE o.amount = {amount}");
    assert_eq!(one(&my, &owner("10")).await, "me@mine.com", "the order follows the customer who was already there");
    assert_eq!(one(&my, &owner("99.99")).await, "dup@x.com");
    assert_eq!(one(&my, "SELECT CONCAT(amount, '|', placed_at, '|', status) FROM orders WHERE amount > 1000").await, "1234.5678|2026-02-01 07:00:00|done");
    assert_eq!(one(&my, "SELECT CONCAT(COUNT(*), '|', SUM(qty), '|', COUNT(DISTINCT order_id)) FROM order_lines").await, one(&ms, "SELECT CONCAT(COUNT(*), '|', SUM(Adet), '|', COUNT(DISTINCT SiparisID)) FROM dbo.SiparisKalem").await);
    assert_eq!(one(&my, "SELECT CONCAT(COUNT(*), '|', COUNT(DISTINCT message), '|', MAX(at)) FROM logs").await, "4500|4500|2025-01-01 01:15:00");

    // Moving the customers again finds every one of them, whatever the case of the email.
    let mut again = plan.clone();
    again.tables.retain(|t| t.target_table == "customers");
    let report = run(&ms, &my, &again, false).await.unwrap();
    assert_eq!(report.tables[0].repeated, 1);
    assert_eq!(one(&my, "SELECT COUNT(*) FROM customers").await, "4");
}
