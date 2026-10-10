//! SQL Server end to end, against the optional `sqlserver` service in dev/docker-compose.yml:
//!
//!     docker compose -f dev/docker-compose.yml --profile sqlserver up -d --wait
//!     (seed it with the command at the top of dev/seed/sqlserver.sql)
//!     KIYI_LIVE_SQLSERVER=1 cargo test -p kiyi-core --test sqlserver_live

use std::sync::{Arc, Mutex};

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::design::{self, ColumnDesign, TableAction};
use kiyi_core::dml::{self, BrowseRequest, ChangeSet, ColumnValue, Filter, FilterOp, RowChange, Sort};
use kiyi_core::drivers::{self, DbDriver};
use kiyi_core::types::QueryEvent;

fn live() -> bool {
    std::env::var("KIYI_LIVE_SQLSERVER").is_ok()
}

fn config(db: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: "ms".into(),
        name: "ms".into(),
        kind: DbKind::Sqlserver,
        host: "127.0.0.1".into(),
        port: 51433,
        user: "sa".into(),
        database: Some(db.into()),
        ssl_mode: SslMode::Prefer,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlserver".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    }
}

async fn open(db: &str) -> Arc<dyn DbDriver> {
    drivers::open(&config(db), Some("Kiyi_pass1")).await.expect("connect to SQL Server")
}

fn req(filters: Vec<Filter>) -> BrowseRequest {
    BrowseRequest {
        schema: Some("dbo".into()),
        table: "orders".into(),
        filters,
        raw_where: None,
        search: None,
        search_columns: vec![],
        sort: vec![Sort { column: "total".into(), descending: true }],
        tiebreak: vec!["id".into()],
        limit: 50,
        offset: 10,
    }
}

#[tokio::test]
async fn sql_server_end_to_end() {
    if !live() {
        return;
    }
    let db = open("kiyi_test").await;
    let d = db.dialect();
    assert!(db.server_version().await.unwrap().contains("SQL Server"));

    // Schema and table details.
    let schema = db.schema().await.unwrap();
    assert_eq!(schema.default_schema.as_deref(), Some("dbo"));
    let dbo = schema.schemas.iter().find(|s| s.name == "dbo").unwrap();
    assert!(dbo.tables.iter().any(|t| t.name == "orders" && t.row_estimate == Some(2000)));
    let details = db.table_details(Some("dbo"), "customers").await.unwrap();
    let id = details.design.columns.iter().find(|c| c.name == "id").unwrap();
    assert!(id.primary_key && id.auto_increment);
    assert_eq!(details.design.columns.iter().find(|c| c.name == "full_name").unwrap().data_type, "nvarchar(120)");
    let orders = db.table_details(Some("dbo"), "orders").await.unwrap();
    assert_eq!(orders.design.foreign_keys[0].ref_table, "customers");
    assert_eq!(orders.design.indexes[0].columns, ["customer_id"]);

    // Values come back as the same text the other databases use.
    let (cols, rows) = db.fetch("SELECT id, full_name, is_active, balance, avatar, created_at FROM customers WHERE id = 1").await.unwrap();
    assert_eq!(cols[2].kind, kiyi_core::types::ValueKind::Bool);
    assert_eq!(rows[0][1].as_deref(), Some("Ayşe Yılmaz"));
    assert_eq!(rows[0][2].as_deref(), Some("true"));
    assert_eq!(rows[0][3].as_deref(), Some("12345678901234567890.0123456789"));
    assert_eq!(rows[0][4].as_deref(), Some("0xDEADBEEF"));

    // Browsing: a Turkish search with SQL Server's own LIKE specials, paging, counting.
    let f = vec![Filter { column: "note".into(), op: FilterOp::Contains, value: "50% indirim [kampanya]".into() }];
    let (_, page) = db.fetch(&dml::browse_sql(d, &req(f.clone()))).await.unwrap();
    assert_eq!(page.len(), 50);
    let (_, count) = db.fetch(&dml::count_sql(d, &req(f))).await.unwrap();
    assert_eq!(count[0][0].as_deref(), Some("200"));

    // Summaries by month.
    let s = dml::SummaryRequest {
        browse: req(vec![]),
        group_by: vec![dml::GroupBy { column: "placed_on".into(), date_part: Some(dml::DatePart::Month) }],
        measures: vec![dml::Measure { aggregate: dml::Aggregate::Sum, column: Some("total".into()) }],
    };
    let (_, months) = db.fetch(&dml::summary_sql(d, &s).unwrap()).await.unwrap();
    assert!(months.len() >= 3 && months[0][0].as_deref().unwrap().len() == 7);

    // The editor's stream: a result set, then DML counts.
    let events = Mutex::new(Vec::new());
    let sink = |e: QueryEvent| events.lock().unwrap().push(e);
    db.execute("SELECT TOP 3 id FROM orders ORDER BY id; SELECT COUNT(*) AS n FROM customers", &|_| {}, &sink).await.unwrap();
    let ev = events.into_inner().unwrap();
    assert_eq!(ev.iter().filter(|e| matches!(e, QueryEvent::Columns { .. })).count(), 2);

    // Scratch table: create, edit through the grid's SQL, alter, rename, drop.
    let name = format!("kiyi_scratch_{}", std::process::id());
    let col = |n: &str, ty: &str| ColumnDesign {
        original: None,
        name: n.into(),
        data_type: ty.into(),
        nullable: true,
        default: None,
        primary_key: false,
        auto_increment: false,
        comment: None,
        generated: false,
        extra: None,
        enum_values: vec![],
    };
    let mut created = design::TableDesign {
        name: name.clone(),
        columns: vec![ColumnDesign { primary_key: true, auto_increment: true, nullable: false, ..col("id", "int") }, col("title", "nvarchar(50)"), ColumnDesign { default: Some("(0)".into()), nullable: false, ..col("done", "bit") }],
        indexes: vec![],
        foreign_keys: vec![],
        primary_key_name: None,
    };
    db.execute_script(&design::plan_create(d, Some("dbo"), &created).unwrap(), true, false).await.unwrap();
    let changes = ChangeSet {
        schema: Some("dbo".into()),
        table: name.clone(),
        binary_columns: vec![],
        bool_columns: vec!["done".into()],
        changes: vec![RowChange::Insert { values: vec![ColumnValue { column: "title".into(), value: Some("İlk iş; 'tırnak'".into()) }, ColumnValue { column: "done".into(), value: Some("true".into()) }] }],
    };
    db.execute_script(&dml::plan_changes(d, &changes), true, true).await.unwrap();
    let (_, r) = db.fetch(&format!("SELECT title, done FROM [{name}]")).await.unwrap();
    assert_eq!(r[0], vec![Some("İlk iş; 'tırnak'".to_string()), Some("true".to_string())]);

    // Alter: rename a column, widen it, change a default, add and drop columns.
    let old = db.table_details(Some("dbo"), &name).await.unwrap().design;
    created = old.clone();
    created.columns[1].name = "heading".into();
    created.columns[1].data_type = "nvarchar(200)".into();
    created.columns[2].default = Some("(1)".into());
    created.columns.push(ColumnDesign { original: None, default: Some("(SYSDATETIME())".into()), nullable: false, ..col("added", "datetime2") });
    let alter = design::plan_alter(d, Some("dbo"), &old, &created).unwrap();
    db.execute_script(&alter, true, false).await.unwrap_or_else(|e| panic!("{e}\n{}", alter.join(";\n")));
    let after = db.table_details(Some("dbo"), &name).await.unwrap().design;
    assert_eq!(after.columns[1].name, "heading");
    assert_eq!(after.columns[1].data_type, "nvarchar(200)");
    assert_eq!(after.columns[2].default.as_deref(), Some("((1))"));
    let mut dropped = after.clone();
    dropped.columns.retain(|c| c.name != "done");
    db.execute_script(&design::plan_alter(d, Some("dbo"), &after, &dropped).unwrap(), true, false).await.unwrap();
    let renamed = format!("{name}_2");
    db.execute_script(&design::plan_action(d, Some("dbo"), &name, false, &TableAction::Rename { to: renamed.clone() }), true, false).await.unwrap();
    db.execute_script(&design::plan_action(d, Some("dbo"), &renamed, false, &TableAction::Drop), true, false).await.unwrap();

    // Explain and the schema graph.
    let rows = db.explain_rows("SELECT * FROM orders WHERE total > 100 ORDER BY total DESC").await.unwrap().unwrap();
    let plan = kiyi_core::explain::from_sqlserver_text(&rows).unwrap();
    assert!(format!("{plan:?}").contains("orders"), "{plan:#?}");
    let (_, g) = db.fetch(kiyi_core::graph::graph_sql(DbKind::Sqlserver)).await.unwrap();
    let graph = kiyi_core::graph::from_rows(DbKind::Sqlserver, &g);
    assert!(graph.relations.iter().any(|r| r.table == "orders" && r.ref_table == "customers" && r.columns == ["customer_id"]));
}

#[tokio::test]
async fn sql_server_cancel_stops_a_long_query() {
    if !live() {
        return;
    }
    let db = open("kiyi_test").await;
    let session = Arc::new(Mutex::new(None));
    let (db2, s2) = (db.clone(), session.clone());
    let started = std::time::Instant::now();
    let run = tokio::spawn(async move { db2.execute("WAITFOR DELAY '00:00:20'; SELECT 1", &move |id| *s2.lock().unwrap() = Some(id), &|_| {}).await });
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let id = session.lock().unwrap().expect("session id");
    db.cancel(id).await.unwrap();
    let err = run.await.unwrap().unwrap_err();
    assert!(matches!(err, kiyi_core::error::Error::Cancelled), "{err:?}");
    assert!(started.elapsed().as_secs() < 10);
    // The pool still works afterwards.
    assert_eq!(db.fetch("SELECT 1").await.unwrap().1[0][0].as_deref(), Some("1"));
}

#[tokio::test]
async fn sql_server_backup_restores_and_compares_equal() {
    use kiyi_core::backup::{self, Target};
    if !live() {
        return;
    }
    let src = open("kiyi_test").await;
    let path = std::env::temp_dir().join(format!("kiyi-ms-{}.sql", std::process::id()));
    let report = backup::backup(&*src, &Target { config: config("kiyi_test"), password: Some("Kiyi_pass1".into()) }, &path, false).await.unwrap();
    assert_eq!(report.rows, Some(3002));
    let master = open("master").await;
    let fresh = "kiyi_restore_test";
    master.execute_script(&[format!("IF DB_ID('{fresh}') IS NOT NULL DROP DATABASE {fresh}"), format!("CREATE DATABASE {fresh}")], false, false).await.unwrap();
    let dst = open(fresh).await;
    backup::restore(&*dst, &Target { config: config(fresh), password: Some("Kiyi_pass1".into()) }, &path).await.unwrap_or_else(|e| panic!("restore: {e}"));
    for q in ["SELECT * FROM customers ORDER BY id", "SELECT * FROM orders ORDER BY id"] {
        assert_eq!(src.fetch(q).await.unwrap().1, dst.fetch(q).await.unwrap().1, "{q}");
    }
    // Identity numbering continues, and the structure compares equal.
    dst.execute_script(&["INSERT INTO customers (email) VALUES (N'after@example.com')".into()], true, false).await.unwrap();
    let c = kiyi_core::compare::compare(&*src, &*dst).await.unwrap();
    let diff: Vec<_> = c.tables.iter().filter(|t| t.status != kiyi_core::compare::DiffStatus::Same).collect();
    assert!(diff.is_empty(), "{diff:#?}");
    dst.close().await;
    master.execute_script(&[format!("ALTER DATABASE {fresh} SET SINGLE_USER WITH ROLLBACK IMMEDIATE"), format!("DROP DATABASE {fresh}")], false, false).await.unwrap();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn objects_definitions_and_drops() {
    use kiyi_core::objects::{self, ObjectKind};
    if !live() {
        return;
    }
    let db = open("kiyi_test").await;
    // As the SQL editor runs them: each script as one plain batch.
    let run = |sql: &str| {
        let db = db.clone();
        let sql = sql.to_string();
        async move { db.execute(&sql, &|_| {}, &|_| {}).await.unwrap_or_else(|e| panic!("{sql}: {e}")) }
    };
    for sql in [
        "DROP TRIGGER IF EXISTS dbo.obj_touch",
        "DROP TABLE IF EXISTS dbo.obj_notes",
        "DROP FUNCTION IF EXISTS dbo.obj_add",
        "DROP PROCEDURE IF EXISTS dbo.obj_clean",
        "DROP SEQUENCE IF EXISTS dbo.obj_invoice",
        "DROP USER IF EXISTS obj_reader",
        "CREATE TABLE dbo.obj_notes (id int IDENTITY PRIMARY KEY, body nvarchar(100), updated_at datetime2)",
        "CREATE FUNCTION dbo.obj_add(@a int, @b int) RETURNS int AS BEGIN RETURN @a + @b; END",
        "CREATE PROCEDURE dbo.obj_clean @days int AS SELECT @days",
        "CREATE TRIGGER dbo.obj_touch ON dbo.obj_notes AFTER UPDATE AS SET NOCOUNT ON",
        "CREATE SEQUENCE dbo.obj_invoice AS bigint START WITH 1000",
        "CREATE USER obj_reader WITHOUT LOGIN",
        "GRANT SELECT ON dbo.obj_notes TO obj_reader",
        "ALTER ROLE db_datareader ADD MEMBER obj_reader",
    ] {
        run(sql).await;
    }

    let list = objects::list(db.as_ref()).await;
    assert!(list.notes.is_empty(), "{:?}", list.notes);
    let find = |kind: ObjectKind, name: &str| list.objects.iter().find(|o| o.kind == kind && o.name == name).cloned().unwrap_or_else(|| panic!("no {name}: {:#?}", list.objects));
    let f = find(ObjectKind::Function, "obj_add");
    assert_eq!(f.detail, "returns a value");
    let p = find(ObjectKind::Procedure, "obj_clean");
    let t = find(ObjectKind::Trigger, "obj_touch");
    assert_eq!(t.detail, "on obj_notes");
    let seq = find(ObjectKind::Sequence, "obj_invoice");
    assert_eq!(seq.detail, "at 1000");
    let user = find(ObjectKind::User, "obj_reader");

    assert!(objects::source(db.as_ref(), &f).await.unwrap().definition.contains("CREATE FUNCTION dbo.obj_add"));
    let src = objects::source(db.as_ref(), &seq).await.unwrap();
    assert!(src.definition.contains("START WITH 1000") && src.definition.contains("AS bigint"), "{}", src.definition);
    let src = objects::source(db.as_ref(), &user).await.unwrap();
    assert!(src.definition.contains("ALTER ROLE [db_datareader] ADD MEMBER [obj_reader];"), "{}", src.definition);
    assert!(src.definition.contains("GRANT SELECT ON [dbo].[obj_notes] TO [obj_reader];"), "{}", src.definition);
    assert_eq!(src.drop, "DROP USER [obj_reader];");

    for o in [&t, &f, &p, &seq, &user] {
        let drop = objects::source(db.as_ref(), o).await.unwrap().drop;
        run(&drop).await;
    }
    run("DROP TABLE dbo.obj_notes").await;
    let list = objects::list(db.as_ref()).await;
    assert!(!list.objects.iter().any(|o| o.name.starts_with("obj_")), "{:#?}", list.objects);
    let tpl = objects::template(db.dialect(), ObjectKind::Function, Some("dbo"));
    run(&tpl).await;
    run("DROP FUNCTION [dbo].[add_numbers]").await;
}
