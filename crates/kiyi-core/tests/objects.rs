//! Database objects, the sample database and moving connections between computers.
//! The SQLite tests always run; PostgreSQL and MySQL need `KIYI_LIVE=1` and the Docker databases.

use std::sync::Arc;

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::drivers::{self, DbDriver};
use kiyi_core::objects::{self, DbObject, ObjectKind};
use kiyi_core::workspace::Workspace;

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiyi-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn find<'a>(list: &'a [DbObject], kind: ObjectKind, name: &str) -> &'a DbObject {
    list.iter().find(|o| o.kind == kind && o.name == name).unwrap_or_else(|| panic!("no {kind:?} {name} in {list:#?}"))
}

#[tokio::test]
async fn sample_database_is_created_once_and_has_a_trigger() {
    let ws = Workspace::new(&scratch_dir("sample")).unwrap();
    let first = ws.create_sample().await.unwrap();
    let again = ws.create_sample().await.unwrap();
    assert_eq!(first.id, again.id, "recreating keeps the same saved connection");
    assert_eq!(ws.list().len(), 1);

    let db = drivers::open(&again, None).await.unwrap();
    let (_, rows) = db.fetch("SELECT (SELECT count(*) FROM customers), (SELECT count(*) FROM orders), (SELECT count(DISTINCT email) FROM customers)").await.unwrap();
    assert_eq!(rows[0], vec![Some("1200".into()), Some("9000".into()), Some("1200".into())]);
    let (_, rows) = db.fetch("SELECT count(*) FROM orders WHERE placed_at > datetime('now', '-30 days')").await.unwrap();
    assert!(rows[0][0].as_deref().unwrap().parse::<u32>().unwrap() > 300, "recent orders for charts");

    let list = objects::list(db.as_ref()).await;
    assert_eq!(list.kinds, vec![ObjectKind::Trigger]);
    assert!(list.notes.is_empty(), "{:?}", list.notes);
    let trigger = find(&list.objects, ObjectKind::Trigger, "products_updated_at");
    assert_eq!(trigger.detail, "on products");
    let source = objects::source(db.as_ref(), trigger).await.unwrap();
    assert!(source.definition.starts_with("CREATE TRIGGER products_updated_at"), "{}", source.definition);
    assert_eq!(source.drop, "DROP TRIGGER \"products_updated_at\";");

    // The trigger works.
    db.execute_script(&["UPDATE products SET updated_at = '2000-01-01' WHERE id = 1".into(), "UPDATE products SET price = 90 WHERE id = 1".into()], false, false)
        .await
        .unwrap();
    let (_, rows) = db.fetch("SELECT updated_at > '2001' FROM products WHERE id = 1").await.unwrap();
    assert_eq!(rows[0][0].as_deref(), Some("1"));
    db.close().await;
}

fn conn(name: &str, host: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: String::new(),
        name: name.into(),
        kind: DbKind::Postgres,
        host: host.into(),
        port: 5432,
        user: "app".into(),
        database: Some("shop".into()),
        ssl_mode: SslMode::Prefer,
        env: EnvTag::Production,
        read_only: true,
        driver: Some("postgres".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    }
}

#[tokio::test]
async fn connections_move_between_computers_without_secrets() {
    let dir = scratch_dir("export");
    let here = Workspace::new(&dir.join("here")).unwrap();
    let a = here.save(conn("Shop", "db.example.com"), None, None).unwrap();
    here.save(conn("Reports", "reports.example.com"), None, None).unwrap();

    let file = dir.join("connections.json");
    assert_eq!(here.export_connections(&[], &file).unwrap(), 2);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"kiyi\": \"connections\"") && !text.to_lowercase().contains("password\":"), "{text}");
    assert_eq!(here.export_connections(std::slice::from_ref(&a.id), &dir.join("one.json")).unwrap(), 1);
    // The sample database is made fresh on each computer, so it isn't exported.
    let sample = here.create_sample().await.unwrap();
    assert_eq!(here.export_connections(&[], &file).unwrap(), 2);
    assert_eq!(here.export_connections(std::slice::from_ref(&sample.id), &dir.join("none.json")).unwrap(), 0);

    let there = Workspace::new(&dir.join("there")).unwrap();
    there.save(conn("Shop", "other.example.com"), None, None).unwrap();
    let added = there.import_connections(&file).unwrap();
    assert_eq!(added.len(), 2);
    assert!(added.iter().all(|c| c.id != a.id && !c.id.is_empty()), "fresh ids");
    assert!(added.iter().any(|c| c.name == "Shop (imported)"), "a clashing name is marked");
    assert!(added.iter().all(|c| c.env == EnvTag::Production && c.read_only), "safety settings travel along");

    // Importing again adds nothing: the same databases are already there.
    assert!(there.import_connections(&file).unwrap().is_empty());
    assert_eq!(there.list().len(), 3);

    std::fs::write(dir.join("junk.json"), "{\"hello\": 1}").unwrap();
    assert!(there.import_connections(&dir.join("junk.json")).unwrap_err().to_string().contains("doesn't contain Kiyi connections"));
}

// ---------------------------------------------------------------- live servers

fn live() -> bool {
    std::env::var("KIYI_LIVE").is_ok()
}

async fn open(kind: DbKind) -> Arc<dyn DbDriver> {
    let config = ConnectionConfig {
        id: "test".into(),
        name: "test".into(),
        kind,
        host: "127.0.0.1".into(),
        port: if kind == DbKind::Postgres { 55432 } else { 53306 },
        user: if kind == DbKind::Postgres { "kiyi".into() } else { "root".into() },
        database: Some("kiyi_test".into()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: None,
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    drivers::open(&config, Some("kiyi")).await.expect("connect")
}

async fn script(db: &Arc<dyn DbDriver>, statements: &[&str]) {
    let s: Vec<String> = statements.iter().map(|s| s.to_string()).collect();
    db.execute_script(&s, false, false).await.unwrap();
}

#[tokio::test]
async fn postgres_objects_definitions_and_drops() {
    if !live() {
        return;
    }
    let db = open(DbKind::Postgres).await;
    script(
        &db,
        &[
            "DROP TABLE IF EXISTS obj_notes",
            "DROP FUNCTION IF EXISTS obj_add(integer, integer)",
            "DROP FUNCTION IF EXISTS obj_touch()",
            "DROP PROCEDURE IF EXISTS obj_clean(integer)",
            "DROP SEQUENCE IF EXISTS obj_invoice",
            "DROP ROLE IF EXISTS obj_reader",
            "CREATE TABLE obj_notes (id serial PRIMARY KEY, body text, updated_at timestamptz)",
            "CREATE FUNCTION obj_add(a integer, b integer) RETURNS integer LANGUAGE sql AS 'SELECT a + b'",
            "CREATE PROCEDURE obj_clean(days integer) LANGUAGE sql AS 'SELECT 1'",
            "CREATE FUNCTION obj_touch() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN NEW.updated_at := now(); RETURN NEW; END$$",
            "CREATE TRIGGER obj_touch_notes BEFORE UPDATE ON obj_notes FOR EACH ROW EXECUTE FUNCTION obj_touch()",
            "CREATE SEQUENCE obj_invoice START WITH 1000",
            "SELECT nextval('obj_invoice')",
            "CREATE ROLE obj_reader NOLOGIN",
            "GRANT SELECT, INSERT ON obj_notes TO obj_reader",
        ],
    )
    .await;

    let list = objects::list(db.as_ref()).await;
    assert!(list.notes.is_empty(), "{:?}", list.notes);
    let f = find(&list.objects, ObjectKind::Function, "obj_add");
    assert_eq!(f.detail, "(a integer, b integer) → integer");
    find(&list.objects, ObjectKind::Procedure, "obj_clean");
    let t = find(&list.objects, ObjectKind::Trigger, "obj_touch_notes");
    assert_eq!(t.detail, "on obj_notes");
    let seq = find(&list.objects, ObjectKind::Sequence, "obj_invoice");
    assert_eq!(seq.detail, "at 1000");
    assert_eq!(find(&list.objects, ObjectKind::Sequence, "obj_notes_id_seq").detail, "for obj_notes.id · not used yet");
    let role = find(&list.objects, ObjectKind::User, "obj_reader");
    assert_eq!(role.detail, "role");

    let src = objects::source(db.as_ref(), f).await.unwrap();
    assert!(src.definition.contains("CREATE OR REPLACE FUNCTION public.obj_add"), "{}", src.definition);
    assert_eq!(src.drop, "DROP FUNCTION public.obj_add(a integer, b integer);");
    let src = objects::source(db.as_ref(), t).await.unwrap();
    assert!(src.definition.starts_with("CREATE TRIGGER obj_touch_notes") && src.definition.contains("The function it runs"), "{}", src.definition);
    assert_eq!(src.drop, "DROP TRIGGER obj_touch_notes ON obj_notes;");
    let src = objects::source(db.as_ref(), seq).await.unwrap();
    assert!(src.definition.contains("START WITH 1000") && src.definition.contains("Current value: 1000"), "{}", src.definition);
    let src = objects::source(db.as_ref(), role).await.unwrap();
    assert!(src.definition.starts_with("CREATE ROLE obj_reader NOLOGIN;"), "{}", src.definition);
    assert!(src.definition.contains("GRANT INSERT, SELECT ON public.obj_notes TO obj_reader;"), "{}", src.definition);

    // The generated drops run as they are.
    for o in [t, f, seq] {
        let drop = objects::source(db.as_ref(), o).await.unwrap().drop;
        script(&db, &[&drop]).await;
    }
    script(&db, &["DROP TABLE obj_notes", "DROP ROLE obj_reader", "DROP FUNCTION obj_touch()", "DROP PROCEDURE obj_clean(integer)"]).await;
    let list = objects::list(db.as_ref()).await;
    assert!(!list.objects.iter().any(|o| o.name.starts_with("obj_")), "{:#?}", list.objects);

    // Templates are valid SQL.
    let tpl = objects::template(db.dialect(), ObjectKind::Sequence, Some("public"));
    script(&db, &[&tpl, "DROP SEQUENCE public.invoice_numbers"]).await;
    db.close().await;
}

#[tokio::test]
async fn mysql_objects_definitions_and_drops() {
    if !live() {
        return;
    }
    let db = open(DbKind::Mysql).await;
    script(
        &db,
        &[
            "DROP TABLE IF EXISTS obj_notes",
            "DROP FUNCTION IF EXISTS obj_add",
            "DROP PROCEDURE IF EXISTS obj_clean",
            "DROP USER IF EXISTS 'obj_reader'@'%'",
            "CREATE TABLE obj_notes (id INT AUTO_INCREMENT PRIMARY KEY, body TEXT, updated_at DATETIME)",
            "CREATE FUNCTION obj_add(a INT, b INT) RETURNS INT DETERMINISTIC RETURN a + b",
            "CREATE PROCEDURE obj_clean(IN days INT) BEGIN SELECT days; END",
            "CREATE TRIGGER obj_touch_notes BEFORE UPDATE ON obj_notes FOR EACH ROW SET NEW.updated_at = NOW()",
            "CREATE USER 'obj_reader'@'%' IDENTIFIED BY 'x'",
            "GRANT SELECT ON kiyi_test.obj_notes TO 'obj_reader'@'%'",
        ],
    )
    .await;

    let list = objects::list(db.as_ref()).await;
    assert!(list.notes.is_empty(), "{:?}", list.notes);
    let f = find(&list.objects, ObjectKind::Function, "obj_add");
    assert_eq!(f.detail, "→ int");
    let p = find(&list.objects, ObjectKind::Procedure, "obj_clean");
    let t = find(&list.objects, ObjectKind::Trigger, "obj_touch_notes");
    assert_eq!(t.detail, "before update on obj_notes");
    let user = find(&list.objects, ObjectKind::User, "obj_reader@%");
    assert_eq!(user.detail, "no server-wide privileges");

    let src = objects::source(db.as_ref(), f).await.unwrap();
    assert!(src.definition.contains("FUNCTION `obj_add`"), "{}", src.definition);
    assert_eq!(src.drop, "DROP FUNCTION `kiyi_test`.`obj_add`;");
    assert!(objects::source(db.as_ref(), p).await.unwrap().definition.contains("PROCEDURE `obj_clean`"));
    let src = objects::source(db.as_ref(), t).await.unwrap();
    assert!(src.definition.contains("TRIGGER `obj_touch_notes`") || src.definition.contains("TRIGGER obj_touch_notes"), "{}", src.definition);
    let src = objects::source(db.as_ref(), user).await.unwrap();
    assert!(src.definition.contains("GRANT SELECT ON `kiyi_test`.`obj_notes` TO `obj_reader`@`%`;"), "{}", src.definition);
    assert_eq!(src.drop, "DROP USER 'obj_reader'@'%';");

    for o in [t, f, p, user] {
        let drop = objects::source(db.as_ref(), o).await.unwrap().drop;
        script(&db, &[&drop]).await;
    }
    script(&db, &["DROP TABLE obj_notes"]).await;
    let list = objects::list(db.as_ref()).await;
    assert!(!list.objects.iter().any(|o| o.name.starts_with("obj_")), "{:#?}", list.objects);
    db.close().await;
}
