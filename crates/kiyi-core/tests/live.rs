//! Driver tests against real servers. Start them with
//! `docker compose -f dev/docker-compose.yml up -d --wait` and run with `KIYI_LIVE=1 cargo test`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::drivers::{self, DbDriver};
use kiyi_core::types::{QueryEvent, ValueKind};

fn live() -> bool {
    std::env::var("KIYI_LIVE").is_ok()
}

fn config(kind: DbKind) -> ConnectionConfig {
    ConnectionConfig {
        id: "test".into(),
        name: "test".into(),
        kind,
        host: "127.0.0.1".into(),
        port: if kind == DbKind::Postgres { 55432 } else { 53306 },
        user: "kiyi".into(),
        // A copy of the dev seed reserved for tests; `shop` is for poking at in the app.
        database: Some("kiyi_test".into()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: None,
        tunnel: None,
    }
}

async fn open(kind: DbKind) -> Arc<dyn DbDriver> {
    drivers::open(&config(kind), Some("kiyi")).await.expect("connect")
}

async fn run(driver: &Arc<dyn DbDriver>, sql: &str) -> (Vec<QueryEvent>, Result<(), String>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = {
        let events = events.clone();
        move |e: QueryEvent| events.lock().unwrap().push(e)
    };
    let result = driver.execute(sql, &|_| {}, &sink).await.map_err(|e| e.to_string());
    let events = std::mem::take(&mut *events.lock().unwrap());
    (events, result)
}

type Table = (Vec<(String, String, ValueKind)>, Vec<Vec<Option<String>>>);

/// Flattens the events of a single-statement run into (columns, rows).
fn table(events: &[QueryEvent]) -> Table {
    let mut cols = Vec::new();
    let mut rows = Vec::new();
    for e in events {
        match e {
            QueryEvent::Columns { columns } => {
                cols = columns.iter().map(|c| (c.name.clone(), c.type_name.clone(), c.kind)).collect()
            }
            QueryEvent::Rows { rows: r } => rows.extend(r.iter().cloned()),
            _ => {}
        }
    }
    (cols, rows)
}

#[tokio::test]
async fn postgres_values_keep_their_canonical_text() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let (events, result) = run(
        &pg,
        "SELECT id, public_id, full_name, is_active, balance, tags, profile, avatar, created_at \
         FROM customers WHERE email = 'ayse@example.com'",
    )
    .await;
    result.unwrap();
    let (cols, rows) = table(&events);
    let kinds: Vec<ValueKind> = cols.iter().map(|c| c.2).collect();
    assert_eq!(
        kinds,
        [
            ValueKind::Number,
            ValueKind::Uuid,
            ValueKind::Text,
            ValueKind::Bool,
            ValueKind::Number,
            ValueKind::Array,
            ValueKind::Json,
            ValueKind::Binary,
            ValueKind::Temporal
        ]
    );
    let row = &rows[0];
    assert_eq!(row[2].as_deref(), Some("Ayşe Yılmaz"));
    assert_eq!(row[3].as_deref(), Some("true"));
    assert_eq!(row[4].as_deref(), Some("12345678901234567890.0123456789"));
    assert_eq!(row[5].as_deref(), Some("{vip,early}"));
    assert_eq!(row[6].as_deref(), Some(r#"{"plan": "pro", "seats": 5}"#));
    assert_eq!(row[7].as_deref(), Some(r"\xdeadbeef"));
}

#[tokio::test]
async fn postgres_nulls_enums_empty_results_and_affected_rows() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;

    let (events, _) = run(&pg, "SELECT full_name, tags FROM customers WHERE email = 'mehmet@example.com'").await;
    assert_eq!(table(&events).1, vec![vec![None, None]]);

    let (events, _) = run(&pg, "SELECT status FROM orders LIMIT 1").await;
    let (cols, rows) = table(&events);
    assert_eq!(cols[0].2, ValueKind::Other);
    assert!(cols[0].1.starts_with("oid "));
    assert!(matches!(rows[0][0].as_deref(), Some("pending" | "paid" | "shipped")));

    let (events, _) = run(&pg, "SELECT id, email FROM customers WHERE false").await;
    let (cols, rows) = table(&events);
    assert_eq!(cols.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["id", "email"]);
    assert!(rows.is_empty());

    let (events, result) = run(&pg, "BEGIN; UPDATE customers SET is_active = true WHERE id <= 10; ROLLBACK").await;
    result.unwrap();
    let affected: Vec<u64> = events
        .iter()
        .filter_map(|e| if let QueryEvent::StatementDone { rows_affected } = e { Some(*rows_affected) } else { None })
        .collect();
    assert_eq!(affected, [0, 10, 0]);
}

#[tokio::test]
async fn postgres_streams_large_results_in_batches() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let (events, _) = run(&pg, "SELECT * FROM orders").await;
    let batches = events.iter().filter(|e| matches!(e, QueryEvent::Rows { .. })).count();
    assert_eq!(table(&events).1.len(), 50_000);
    assert!(batches > 1, "expected multiple batches, got {batches}");
}

#[tokio::test]
async fn postgres_schema_and_cancel() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let schema = pg.schema().await.unwrap();
    assert_eq!(schema.default_schema.as_deref(), Some("public"));
    let public = schema.schemas.iter().find(|s| s.name == "public").unwrap();
    // Other tests create scratch tables concurrently; only check the seeded ones.
    let names: Vec<&str> = public.tables.iter().map(|t| t.name.as_str()).filter(|n| !n.starts_with("kiyi_")).collect();
    assert_eq!(names, ["active_customers", "customers", "orders"]);

    let session = Arc::new(Mutex::new(None));
    let runner = {
        let (pg, session) = (pg.clone(), session.clone());
        tokio::spawn(async move {
            let on_session = move |id| *session.lock().unwrap() = Some(id);
            pg.execute("SELECT pg_sleep(30)", &on_session, &|_| {}).await
        })
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pid = session.lock().unwrap().expect("session id reported");
    pg.cancel(pid).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), runner).await.expect("cancel took effect").unwrap();
    assert!(result.unwrap_err().to_string().contains("cancel"));
}

#[tokio::test]
async fn mysql_values_keep_their_canonical_text() {
    if !live() {
        return;
    }
    let my = open(DbKind::Mysql).await;
    let (events, result) = run(
        &my,
        "SELECT id, public_id, full_name, is_active, balance, profile, status, created_at FROM customers ORDER BY id",
    )
    .await;
    result.unwrap();
    let (cols, rows) = table(&events);
    let kinds: Vec<ValueKind> = cols.iter().map(|c| c.2).collect();
    assert_eq!(
        kinds,
        [
            ValueKind::Number,
            ValueKind::Binary,
            ValueKind::Text,
            ValueKind::Number,
            ValueKind::Number,
            ValueKind::Json,
            ValueKind::Text,
            ValueKind::Temporal
        ]
    );
    let ayse = &rows[0];
    let public_id = ayse[1].as_deref().unwrap();
    assert!(public_id.starts_with("0x") && public_id.len() == 34, "{public_id}");
    assert_eq!(ayse[2].as_deref(), Some("Ayşe Yılmaz"));
    assert_eq!(ayse[4].as_deref(), Some("12345678901234567890.0123456789"));
    assert_eq!(ayse[5].as_deref(), Some(r#"{"plan": "pro", "seats": 5}"#));
    assert_eq!(rows[1][2], None);

    let (events, _) = run(&my, "SELECT id, email FROM customers WHERE 1 = 0").await;
    assert_eq!(table(&events).0.len(), 2);

    let schema = my.schema().await.unwrap();
    assert_eq!(schema.default_schema.as_deref(), Some("kiyi_test"));
    let shop = schema.schemas.iter().find(|s| s.name == "kiyi_test").unwrap();
    let names: Vec<&str> = shop.tables.iter().map(|t| t.name.as_str()).filter(|n| !n.starts_with("kiyi_")).collect();
    assert_eq!(names, ["active_customers", "customers", "orders"]);
}

// ---------------------------------------------------------------- structure & edits

use kiyi_core::design::{self, ColumnDesign, FkAction, ForeignKeyDesign, IndexDesign, TableDesign};
use kiyi_core::dml::{self, ChangeSet, ColumnValue, RowChange};

fn new_col(name: &str, ty: &str) -> ColumnDesign {
    ColumnDesign {
        original: None,
        name: name.into(),
        data_type: ty.into(),
        nullable: true,
        default: None,
        primary_key: false,
        auto_increment: false,
        comment: None,
        generated: false,
        extra: None,
        enum_values: vec![],
    }
}

fn scratch_table(prefix: &str) -> String {
    format!("{prefix}_{}", std::process::id())
}

#[tokio::test]
async fn postgres_reading_a_table_and_saving_it_unchanged_is_a_no_op() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    for table in ["customers", "orders"] {
        let details = pg.table_details(Some("public"), table).await.unwrap();
        let sql = design::plan_alter(pg.dialect(), Some("public"), &details.design, &details.design).unwrap();
        assert!(sql.is_empty(), "{table}: {sql:?}");
    }
    let orders = pg.table_details(Some("public"), "orders").await.unwrap();
    assert_eq!(orders.design.primary_key(), ["id"]);
    assert_eq!(orders.design.primary_key_name.as_deref(), Some("orders_pkey"));
    assert_eq!(orders.design.indexes[0].columns, ["customer_id"]);
    let fk = &orders.design.foreign_keys[0];
    assert_eq!((fk.ref_table.as_str(), fk.ref_columns.clone()), ("customers", vec!["id".to_string()]));
    let customers = pg.table_details(Some("public"), "customers").await.unwrap();
    let email = customers.design.columns.iter().find(|c| c.name == "email").unwrap();
    assert_eq!(email.comment.as_deref(), Some("Giriş e-postası"));
    assert!(!email.nullable);
    let unique = customers.design.indexes.iter().find(|i| i.columns == ["email"]).unwrap();
    assert!(unique.unique && unique.is_constraint);
}

#[tokio::test]
async fn mysql_reading_a_table_and_saving_it_unchanged_is_a_no_op() {
    if !live() {
        return;
    }
    let my = open(DbKind::Mysql).await;
    for table in ["customers", "orders"] {
        let details = my.table_details(None, table).await.unwrap();
        let sql = design::plan_alter(my.dialect(), Some("kiyi_test"), &details.design, &details.design).unwrap();
        assert!(sql.is_empty(), "{table}: {sql:?}");
    }
    let orders = my.table_details(None, "orders").await.unwrap();
    let updated = orders.design.columns.iter().find(|c| c.name == "updated_at").unwrap();
    assert_eq!(updated.default.as_deref(), Some("CURRENT_TIMESTAMP"));
    assert_eq!(updated.extra.as_deref().map(str::to_ascii_lowercase).as_deref(), Some("on update current_timestamp"));
    let note = orders.design.columns.iter().find(|c| c.name == "note").unwrap();
    assert_eq!(note.comment.as_deref(), Some("Müşteri notu"));
    let fk = &orders.design.foreign_keys[0];
    assert_eq!((fk.name.as_str(), fk.on_delete), ("orders_customer_fk", FkAction::Cascade));
    let customers = my.table_details(None, "customers").await.unwrap();
    let public_id = customers.design.columns.iter().find(|c| c.name == "public_id").unwrap();
    assert_eq!(public_id.default.as_deref(), Some("(uuid_to_bin(uuid()))"));
}

/// Create a table, change most things about it, check the database agrees, then drop it.
async fn structure_cycle(driver: Arc<dyn DbDriver>, schema: Option<&str>, int: &str, text: &str) {
    let d = driver.dialect();
    let name = scratch_table("kiyi_cycle");
    let created = TableDesign {
        name: name.clone(),
        columns: vec![
            ColumnDesign { primary_key: true, auto_increment: true, nullable: false, ..new_col("id", int) },
            ColumnDesign { nullable: false, ..new_col("title", text) },
            new_col("score", int),
        ],
        indexes: vec![],
        foreign_keys: vec![],
        primary_key_name: None,
    };
    let sql = design::plan_create(d, schema, &created).unwrap();
    driver.execute_script(&sql, !d.is_mysql(), false).await.unwrap();

    let before = driver.table_details(schema, &name).await.unwrap().design;
    let mut after = before.clone();
    after.columns[1].name = "headline".into();
    after.columns[2].default = Some("7".into());
    after.columns.push(ColumnDesign { comment: Some("not".into()), ..new_col("customer_id", int) });
    after.indexes.push(IndexDesign {
        original: None,
        name: format!("{name}_headline_idx"),
        columns: vec!["headline".into()],
        unique: true,
        is_constraint: false,
    });
    let sql = design::plan_alter(d, schema, &before, &after).unwrap();
    driver.execute_script(&sql, !d.is_mysql(), false).await.unwrap();

    let reread = driver.table_details(schema, &name).await.unwrap().design;
    let names: Vec<&str> = reread.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "headline", "score", "customer_id"]);
    assert_eq!(reread.columns[2].default.as_deref().map(|v| v.trim_matches('\'')), Some("7"));
    assert_eq!(reread.columns[3].comment.as_deref(), Some("not"));
    assert!(reread.indexes.iter().any(|i| i.unique && i.columns == ["headline"]));
    // The reread structure is stable: saving it again changes nothing.
    assert!(design::plan_alter(d, schema, &reread, &reread).unwrap().is_empty());

    // Grid edits: insert, update, and a stale update that must roll back the whole batch.
    let cv = |c: &str, v: Option<&str>| ColumnValue { column: c.into(), value: v.map(Into::into) };
    let set = |changes| ChangeSet { schema: schema.map(Into::into), table: name.clone(), binary_columns: vec![], bool_columns: vec![], changes };
    let insert = dml::plan_changes(d, &set(vec![RowChange::Insert { values: vec![cv("headline", Some("O'Brien \\ ok"))] }]));
    driver.execute_script(&insert, true, true).await.unwrap();
    let (_, rows) = driver.fetch(&format!("SELECT id, headline, score FROM {}", d.table(schema, &name))).await.unwrap();
    assert_eq!(rows, vec![vec![Some("1".to_string()), Some("O'Brien \\ ok".to_string()), Some("7".to_string())]]);

    let stale = dml::plan_changes(
        d,
        &set(vec![
            RowChange::Update { key: vec![cv("id", Some("1"))], values: vec![cv("score", Some("99"))] },
            RowChange::Update { key: vec![cv("id", Some("12345"))], values: vec![cv("score", Some("1"))] },
        ]),
    );
    let err = driver.execute_script(&stale, true, true).await.unwrap_err();
    assert!(matches!(err, kiyi_core::Error::Script { index: 1, .. }), "{err:?}");
    let (_, rows) = driver.fetch(&format!("SELECT score FROM {}", d.table(schema, &name))).await.unwrap();
    assert_eq!(rows[0][0].as_deref(), Some("7"), "first update must have been rolled back");

    let drop = design::plan_action(d, schema, &name, false, &design::TableAction::Drop);
    driver.execute_script(&drop, false, false).await.unwrap();
}

#[tokio::test]
async fn postgres_structure_and_data_edit_cycle() {
    if !live() {
        return;
    }
    structure_cycle(open(DbKind::Postgres).await, Some("public"), "integer", "text").await;
}

#[tokio::test]
async fn mysql_structure_and_data_edit_cycle() {
    if !live() {
        return;
    }
    structure_cycle(open(DbKind::Mysql).await, Some("kiyi_test"), "int", "varchar(200)").await;
}

#[tokio::test]
async fn postgres_foreign_key_round_trip() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let d = pg.dialect();
    let name = scratch_table("kiyi_fk");
    let mut t = TableDesign {
        name: name.clone(),
        columns: vec![ColumnDesign { primary_key: true, nullable: false, ..new_col("id", "bigint") }, new_col("customer_id", "bigint")],
        indexes: vec![],
        foreign_keys: vec![],
        primary_key_name: None,
    };
    pg.execute_script(&design::plan_create(d, None, &t).unwrap(), true, false).await.unwrap();
    t = pg.table_details(None, &name).await.unwrap().design;
    let mut with_fk = t.clone();
    with_fk.foreign_keys.push(ForeignKeyDesign {
        original: None,
        name: format!("{name}_customer_fk"),
        columns: vec!["customer_id".into()],
        ref_schema: None,
        ref_table: "customers".into(),
        ref_columns: vec!["id".into()],
        on_delete: FkAction::SetNull,
        on_update: FkAction::NoAction,
    });
    pg.execute_script(&design::plan_alter(d, None, &t, &with_fk).unwrap(), true, false).await.unwrap();
    let reread = pg.table_details(None, &name).await.unwrap().design;
    assert_eq!(reread.foreign_keys[0].on_delete, FkAction::SetNull);
    assert!(design::plan_alter(d, None, &reread, &reread).unwrap().is_empty());
    pg.execute_script(&design::plan_action(d, None, &name, false, &design::TableAction::Drop), false, false).await.unwrap();
}

#[tokio::test]
async fn enum_values_and_row_estimates() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let orders = pg.table_details(Some("public"), "orders").await.unwrap();
    let status = orders.design.columns.iter().find(|c| c.name == "status").unwrap();
    assert_eq!(status.enum_values, ["pending", "paid", "shipped"]);
    sqlx_analyze(&pg).await;
    let schema = pg.schema().await.unwrap();
    let public = schema.schemas.iter().find(|s| s.name == "public").unwrap();
    let orders_info = public.tables.iter().find(|t| t.name == "orders").unwrap();
    assert!(orders_info.row_estimate.unwrap_or(0) > 1000, "{:?}", orders_info.row_estimate);
    assert!(public.tables.iter().find(|t| t.name == "active_customers").unwrap().row_estimate.is_none());

    let my = open(DbKind::Mysql).await;
    let customers = my.table_details(None, "customers").await.unwrap();
    let status = customers.design.columns.iter().find(|c| c.name == "status").unwrap();
    assert_eq!(status.enum_values, ["pending", "paid", "shipped"]);
}

/// Fresh databases have no planner statistics until analyzed.
async fn sqlx_analyze(driver: &Arc<dyn DbDriver>) {
    driver.execute_script(&["ANALYZE".to_string()], false, false).await.unwrap();
}

#[tokio::test]
async fn discovers_local_databases_by_protocol() {
    if !live() {
        return;
    }
    let found = kiyi_core::discover::local_databases().await;
    let pg = found.iter().find(|f| f.port == 55432).expect("postgres on 55432");
    assert_eq!(pg.driver, "postgres");
    let my = found.iter().find(|f| f.port == 53306).expect("mysql on 53306");
    assert_eq!(my.driver, "mysql");
    assert!(my.version.as_deref().unwrap_or("").starts_with('8'), "{:?}", my.version);
}

// ---------------------------------------------------------------- tunnels

use kiyi_core::config::{SshAuth, TunnelConfig};
use kiyi_core::workspace::Workspace;

fn via_bastion(auth: SshAuth, db_host: &str) -> ConnectionConfig {
    ConnectionConfig {
        host: db_host.into(),
        port: 5432,
        tunnel: Some(TunnelConfig::Ssh { host: "127.0.0.1".into(), port: 52222, user: "kiyi".into(), auth }),
        id: String::new(),
        ..config(DbKind::Postgres)
    }
}

fn scratch_workspace() -> Workspace {
    let dir = std::env::temp_dir().join(format!("kiyi-tunnel-test-{}", std::process::id()));
    Workspace::new(&dir).unwrap()
}

#[tokio::test]
async fn ssh_tunnel_with_password_and_with_key() {
    if !live() {
        return;
    }
    let ws = scratch_workspace();
    // `postgres` only resolves inside the Docker network: the bastion has to do the hop.
    let report = ws.test(via_bastion(SshAuth::Password, "postgres"), Some("kiyi".into()), Some("kiyi".into())).await;
    assert!(report.ok, "{:#?}", report.steps);
    assert!(report.steps[0].label.contains("SHA256:"), "{:?}", report.steps[0]);
    assert!(report.server_version.unwrap().starts_with("PostgreSQL"));

    let key = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/ssh/id_ed25519").to_string();
    let report = ws.test(via_bastion(SshAuth::Key { path: key }, "postgres"), Some("kiyi".into()), None).await;
    assert!(report.ok, "{:#?}", report.steps);
}

#[tokio::test]
async fn ssh_tunnel_explains_failures() {
    if !live() {
        return;
    }
    let ws = scratch_workspace();
    let wrong_password = ws.test(via_bastion(SshAuth::Password, "postgres"), Some("kiyi".into()), Some("nope".into())).await;
    assert!(!wrong_password.ok);
    assert!(wrong_password.steps.last().unwrap().detail.as_deref().unwrap().contains("didn't accept these credentials"));

    let unreachable = ws.test(via_bastion(SshAuth::Password, "no-such-db"), Some("kiyi".into()), Some("kiyi".into())).await;
    assert!(!unreachable.ok);
    assert!(unreachable.steps.last().unwrap().detail.as_deref().unwrap().contains("can't reach no-such-db:5432"), "{:#?}", unreachable.steps);
}

// ---------------------------------------------------------------- export / import

use kiyi_core::dml::{BrowseRequest, Filter, FilterOp};
use kiyi_core::transfer::{self, ExportFormat, ImportPlan};

#[tokio::test]
async fn export_filtered_rows_to_csv_and_json() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let req = BrowseRequest {
        schema: Some("public".into()),
        table: "orders".into(),
        filters: vec![Filter { column: "status".into(), op: FilterOp::Eq, value: "paid".into() }],
        raw_where: None,
        search: None,
        search_columns: vec![],
        sort: vec![],
        tiebreak: vec!["id".into()],
        limit: 300,
        offset: 0,
    };
    let sql = kiyi_core::dml::export_sql(pg.dialect(), &req);
    let dir = std::env::temp_dir();
    let csv_path = dir.join(format!("kiyi-export-{}.csv", std::process::id()));
    let n = transfer::export(pg.as_ref(), &sql, ExportFormat::Csv, &csv_path).await.unwrap();
    assert_eq!(n, 16_667, "all matching rows, not just one page");
    let text = std::fs::read_to_string(&csv_path).unwrap();
    assert!(text.starts_with("id,customer_id,status,total,placed_on\n"));
    assert_eq!(text.lines().count() as u64, n + 1);

    let json_path = dir.join(format!("kiyi-export-{}.json", std::process::id()));
    transfer::export(pg.as_ref(), &sql, ExportFormat::Json, &json_path).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    assert_eq!(json.as_array().unwrap().len() as u64, n);
    assert!(json[0]["id"].is_number() && json[0]["total"].is_number());
    assert_eq!(json[0]["status"], "paid");
}

#[tokio::test]
async fn import_a_messy_csv() {
    if !live() {
        return;
    }
    let pg = open(DbKind::Postgres).await;
    let table = scratch_table("kiyi_import");
    pg.execute_script(&[format!("CREATE TABLE {table} (id serial PRIMARY KEY, name text NOT NULL, note text, amount numeric)")], false, false)
        .await
        .unwrap();
    // BOM, semicolons, quoted delimiters and newlines, a non-ASCII name, an empty field, a column to skip.
    let path = std::env::temp_dir().join(format!("{table}.csv"));
    std::fs::write(&path, "\u{feff}Name;Ignored;Note;Amount\nAyşe;x;\"has; semicolon\";12.50\n\"O'Brien\";y;\"two\nlines\";\nZoë;z;;3\n").unwrap();

    let preview = transfer::preview(&path).unwrap();
    assert_eq!(preview.headers, ["Name", "Ignored", "Note", "Amount"]);
    assert_eq!(preview.total, 3);

    let plan = ImportPlan {
        schema: None,
        table: table.clone(),
        mapping: vec![Some("name".into()), None, Some("note".into()), Some("amount".into())],
        has_header: true,
        empty_as_null: true,
    };
    let (statements, count) = transfer::plan_import(pg.dialect(), &path, &plan).unwrap();
    assert_eq!(count, 3);
    pg.execute_script(&statements, true, false).await.unwrap();

    let (_, rows) = pg.fetch(&format!("SELECT name, note, amount FROM {table} ORDER BY id")).await.unwrap();
    assert_eq!(rows[0], vec![Some("Ayşe".into()), Some("has; semicolon".into()), Some("12.50".into())]);
    assert_eq!(rows[1], vec![Some("O'Brien".into()), Some("two\nlines".into()), None]);
    assert_eq!(rows[2][1], None, "empty field imported as NULL");
    pg.execute_script(&[format!("DROP TABLE {table}")], false, false).await.unwrap();
}
