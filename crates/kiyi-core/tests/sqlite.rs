//! SQLite end to end against a temporary database file. Needs no server, so it always runs.

use kiyi_core::config::{ConnectionConfig, DbKind, EnvTag, SslMode};
use kiyi_core::design::{self, ColumnDesign, TableDesign};
use kiyi_core::dml::{self, BrowseRequest, ChangeSet, ColumnValue, Filter, FilterOp, RowChange};
use kiyi_core::drivers;
use kiyi_core::types::ValueKind;

fn col(name: &str, ty: &str) -> ColumnDesign {
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

#[tokio::test]
async fn sqlite_create_edit_browse_and_alter() {
    let path = std::env::temp_dir().join(format!("kiyi-sqlite-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let config = ConnectionConfig {
        id: "t".into(),
        name: "t".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(path.to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    let db = drivers::open(&config, None).await.unwrap();
    let d = db.dialect();
    assert!(db.server_version().await.unwrap().starts_with("SQLite 3"));

    let table = TableDesign {
        name: "notes".into(),
        columns: vec![
            ColumnDesign { primary_key: true, auto_increment: true, nullable: false, ..col("id", "INTEGER") },
            ColumnDesign { nullable: false, ..col("title", "TEXT") },
            ColumnDesign { default: Some("0".into()), ..col("done", "BOOLEAN") },
            col("data", "BLOB"),
            ColumnDesign { default: Some("CURRENT_TIMESTAMP".into()), ..col("created", "DATETIME") },
        ],
        indexes: vec![],
        foreign_keys: vec![],
        primary_key_name: None,
    };
    db.execute_script(&design::plan_create(d, Some("main"), &table).unwrap(), true, false).await.unwrap();

    let cv = |c: &str, v: Option<&str>| ColumnValue { column: c.into(), value: v.map(Into::into) };
    let insert = dml::plan_changes(
        d,
        &ChangeSet {
            schema: Some("main".into()),
            table: "notes".into(),
            binary_columns: vec!["data".into()],
            bool_columns: vec!["done".into()],
            changes: vec![
                RowChange::Insert { values: vec![cv("title", Some("50% off_sale")), cv("done", Some("true")), cv("data", Some("0xcafe"))] },
                RowChange::Insert { values: vec![cv("title", Some("plain"))] },
            ],
        },
    );
    db.execute_script(&insert, true, true).await.unwrap();

    // `%` and `_` in the search must be literal, which needs LIKE … ESCAPE in SQLite.
    let req = BrowseRequest {
        schema: Some("main".into()),
        table: "notes".into(),
        filters: vec![Filter { column: "title".into(), op: FilterOp::Contains, value: "% off_".into() }],
        raw_where: None,
        search: None,
        search_columns: vec![],
        sort: vec![],
        tiebreak: vec!["id".into()],
        limit: 50,
        offset: 0,
    };
    let (cols, rows) = db.fetch(&dml::browse_sql(d, &req)).await.unwrap();
    assert_eq!(rows.len(), 1);
    let kinds: Vec<ValueKind> = cols.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [ValueKind::Number, ValueKind::Text, ValueKind::Bool, ValueKind::Binary, ValueKind::Temporal]);
    assert_eq!(rows[0][0].as_deref(), Some("1"), "id assigned automatically");
    assert_eq!(rows[0][2].as_deref(), Some("true"), "BOOLEAN stored as 1, shown as true");
    assert_eq!(rows[0][3].as_deref(), Some("0xcafe"));
    assert!(rows[0][4].is_some(), "default timestamp applied");

    let details = db.table_details(Some("main"), "notes").await.unwrap();
    assert!(details.design.columns[0].auto_increment && details.design.columns[0].primary_key);
    assert!(design::plan_alter(d, Some("main"), &details.design, &details.design).unwrap().is_empty(), "unchanged design is a no-op");

    let mut changed = details.design.clone();
    changed.columns[1].name = "headline".into();
    changed.columns.remove(3);
    changed.columns.push(col("tags", "TEXT"));
    db.execute_script(&design::plan_alter(d, Some("main"), &details.design, &changed).unwrap(), true, false).await.unwrap();
    let after = db.table_details(Some("main"), "notes").await.unwrap();
    let names: Vec<&str> = after.design.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "headline", "done", "created", "tags"]);

    let mut retyped = after.design.clone();
    retyped.columns[2].data_type = "TEXT".into();
    assert!(design::plan_alter(d, Some("main"), &after.design, &retyped).is_err());

    db.close().await;
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn editor_results_stop_at_the_row_limit() {
    use kiyi_core::types::QueryEvent;
    use kiyi_core::workspace::{Workspace, MAX_RESULT_ROWS};
    use std::sync::{Arc, Mutex};

    let dir = std::env::temp_dir().join(format!("kiyi-limit-{}", std::process::id()));
    let ws = Workspace::new(&dir).unwrap();
    let db = dir.join("limit.db");
    let config = ConnectionConfig {
        id: String::new(),
        name: "limit".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(db.to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    // No password, so nothing is written to the keychain.
    let saved = ws.save(config, None, None).unwrap();
    ws.connect(&saved.id).await.unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Mutex::new(Some(tx));
    let sink = {
        let events = events.clone();
        Arc::new(move |e: QueryEvent| {
            let done = matches!(e, QueryEvent::Done { .. });
            events.lock().unwrap().push(e);
            if done {
                tx.lock().unwrap().take().map(|t| t.send(()));
            }
        })
    };
    let sql = format!("WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {}) SELECT x FROM c", MAX_RESULT_ROWS * 3);
    ws.run(&saved.id, "q".into(), sql, sink).unwrap();
    rx.await.unwrap();

    let events = std::mem::take(&mut *events.lock().unwrap());
    let rows: usize = events.iter().map(|e| if let QueryEvent::Rows { rows } = e { rows.len() } else { 0 }).sum();
    assert_eq!(rows, MAX_RESULT_ROWS);
    assert!(!events.iter().any(|e| matches!(e, QueryEvent::Error { .. })));
    assert!(matches!(events.last(), Some(QueryEvent::Done { truncated: true, cancelled: false, .. })));

    ws.disconnect(&saved.id).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn read_only_connections_refuse_changes() {
    use kiyi_core::workspace::{ScriptKind, Workspace};
    use std::sync::Arc;

    let dir = std::env::temp_dir().join(format!("kiyi-ro-{}", std::process::id()));
    let ws = Workspace::new(&dir).unwrap();
    let db = dir.join("ro.db");
    std::fs::create_dir_all(&dir).unwrap();
    {
        // Create the file with a table first; read-only can't create it.
        let rw = ConnectionConfig {
            id: "rw".into(),
            name: "rw".into(),
            kind: DbKind::Sqlite,
            host: String::new(),
            port: 0,
            user: String::new(),
            database: Some(db.to_string_lossy().into_owned()),
            ssl_mode: SslMode::Disable,
            env: EnvTag::Local,
            read_only: false,
            driver: Some("sqlite".into()),
            tunnel: None,
            ssl_root_cert: None,
            auth: Default::default(),
        };
        let d = drivers::open(&rw, None).await.unwrap();
        d.execute_script(&["CREATE TABLE t (id INTEGER PRIMARY KEY, comment TEXT)".into()], true, false).await.unwrap();
        d.close().await;
    }
    let config = ConnectionConfig {
        id: String::new(),
        name: "ro".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(db.to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Production,
        read_only: true,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    let saved = ws.save(config, None, None).unwrap();
    ws.connect(&saved.id).await.unwrap();
    let sink = Arc::new(|_| {});

    let refused = ws.run(&saved.id, "a".into(), "DELETE FROM t".into(), sink.clone()).unwrap_err();
    assert!(refused.to_string().contains("read-only"), "{refused}");
    assert!(ws.run(&saved.id, "b".into(), "SELECT comment FROM t".into(), sink).is_ok(), "reading a column named comment is fine");
    assert!(ws.execute_script(&saved.id, &["DELETE FROM t".into()], ScriptKind::Data).await.is_err());
    assert!(ws.check_sql(&saved.id, "drop table t").unwrap().destructive == 1);

    ws.disconnect(&saved.id).await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn imports_excel_csv_in_batches_and_all_or_nothing() {
    use kiyi_core::transfer::{self, ImportPlan};

    let dir = std::env::temp_dir().join(format!("kiyi-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config = ConnectionConfig {
        id: "imp".into(),
        name: "imp".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(dir.join("imp.db").to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    let db = drivers::open(&config, None).await.unwrap();
    db.execute_script(&["CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, city TEXT)".into()], true, false).await.unwrap();

    // What Turkish Excel writes: Windows-1254, semicolons, and the Turkish letters only near the end,
    // well past any "look at the first bit" heuristic.
    let mut text = String::from("Ad;Şehir\n");
    for i in 0..1_200 {
        text.push_str(&format!("Person {i};Ankara\n"));
    }
    text.push_str("Ayşe Yılmaz;İstanbul\nĞökçe Işık;Muğla\n");
    let (bytes, _, lossy) = encoding_rs::WINDOWS_1254.encode(&text);
    assert!(!lossy);
    let path = dir.join("excel.csv");
    std::fs::write(&path, &bytes).unwrap();

    let preview = transfer::preview(&path, None, None).unwrap();
    assert_eq!(preview.encoding, "windows-1254");
    assert_eq!(preview.headers, ["Ad", "Şehir"]);
    assert_eq!(preview.total, 1_202);

    let plan = ImportPlan {
        schema: None,
        table: "people".into(),
        mapping: vec![Some("name".into()), Some("city".into())],
        has_header: true,
        empty_as_null: true,
        encoding: None,
        sheet: None,
    };
    assert_eq!(transfer::import(&*db, &path, &plan).await.unwrap(), 1_202);
    let (_, rows) = db.fetch("SELECT name, city FROM people ORDER BY id DESC LIMIT 2").await.unwrap();
    assert_eq!(rows[0], vec![Some("Ğökçe Işık".into()), Some("Muğla".into())]);
    assert_eq!(rows[1], vec![Some("Ayşe Yılmaz".into()), Some("İstanbul".into())]);

    // A bad row in the third batch: nothing from the file may stay.
    let mut text = String::from("name,city\n");
    for i in 0..1_100 {
        text.push_str(&format!("Again {i},Izmir\n"));
    }
    text.push_str(",Nowhere\n"); // empty name → NULL → NOT NULL fails
    let bad = dir.join("bad.csv");
    std::fs::write(&bad, text).unwrap();
    assert_eq!(transfer::preview(&bad, None, None).unwrap().encoding, "UTF-8");
    let err = transfer::import(&*db, &bad, &plan).await.unwrap_err().to_string();
    assert!(err.contains("Nothing was imported") && err.contains("1001"), "{err}");
    let (_, rows) = db.fetch("SELECT count(*) FROM people").await.unwrap();
    assert_eq!(rows[0][0].as_deref(), Some("1202"));

    // Western Excel (Windows-1252) stays Western.
    let (bytes, _, _) = encoding_rs::WINDOWS_1252.encode("name,city\nRenée,Zürich\n");
    let west = dir.join("west.csv");
    std::fs::write(&west, &bytes).unwrap();
    let preview = transfer::preview(&west, None, None).unwrap();
    assert_eq!(preview.encoding, "windows-1252");
    assert_eq!(preview.rows[0], ["Renée", "Zürich"]);

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn excel_export_and_import_round_trip() {
    use kiyi_core::transfer::{self, ExportFormat, ImportPlan};

    let dir = std::env::temp_dir().join(format!("kiyi-xlsx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config = ConnectionConfig {
        id: "x".into(),
        name: "x".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(dir.join("x.db").to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    let db = drivers::open(&config, None).await.unwrap();
    db.execute_script(
        &[
            "CREATE TABLE src (id INTEGER PRIMARY KEY, name TEXT, amount NUMERIC, big TEXT, ok BOOLEAN, day TEXT)".into(),
            "CREATE TABLE dst (id INTEGER PRIMARY KEY, name TEXT, amount NUMERIC, big TEXT, ok BOOLEAN, day TEXT)".into(),
            "INSERT INTO src VALUES (1, 'Ayşe Yılmaz', 12.5, '12345678901234567890', 1, '2026-10-09'), (2, NULL, 3, '7', 0, NULL)".into(),
        ],
        true,
        false,
    )
    .await
    .unwrap();

    let path = dir.join("export.xlsx");
    assert_eq!(transfer::export(&*db, "SELECT * FROM src ORDER BY id", ExportFormat::Xlsx, &path).await.unwrap(), 2);

    let preview = transfer::preview(&path, None, None).unwrap();
    assert_eq!(preview.headers, ["id", "name", "amount", "big", "ok", "day"]);
    assert_eq!(preview.sheets, ["Sheet1"]);
    assert_eq!(preview.total, 2);
    // A 20-digit ID stays exact (kept as text), whole numbers have no ".0", booleans are real
    // Excel TRUE/FALSE cells, and the import turns them back into what the column stores.
    assert_eq!(preview.rows[0], ["1", "Ayşe Yılmaz", "12.5", "12345678901234567890", "true", "2026-10-09"]);

    let plan = ImportPlan {
        schema: None,
        table: "dst".into(),
        mapping: ["id", "name", "amount", "big", "ok", "day"].iter().map(|c| Some(c.to_string())).collect(),
        has_header: true,
        empty_as_null: true,
        encoding: None,
        sheet: None,
    };
    assert_eq!(transfer::import(&*db, &path, &plan).await.unwrap(), 2);
    let (_, a) = db.fetch("SELECT * FROM src ORDER BY id").await.unwrap();
    let (_, b) = db.fetch("SELECT * FROM dst ORDER BY id").await.unwrap();
    assert_eq!(a, b);

    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sqlite_schema_graph() {
    use kiyi_core::graph;
    let dir = std::env::temp_dir().join(format!("kiyi-graph-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let config = ConnectionConfig {
        id: "g".into(),
        name: "g".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(dir.join("g.db").to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    let db = drivers::open(&config, None).await.unwrap();
    db.execute_script(
        &["CREATE TABLE a (id INTEGER PRIMARY KEY)".into(), "CREATE TABLE b (id INTEGER PRIMARY KEY, a_id INTEGER REFERENCES a(id))".into()],
        true,
        false,
    )
    .await
    .unwrap();
    let (_, rows) = db.fetch(graph::graph_sql(DbKind::Sqlite)).await.unwrap();
    let g = graph::from_rows(DbKind::Sqlite, &rows);
    assert_eq!(g.relations.len(), 1);
    assert_eq!((g.relations[0].table.as_str(), g.relations[0].ref_table.as_str()), ("b", "a"));
    assert_eq!(g.relations[0].columns, ["a_id"]);
    assert_eq!(g.primary_keys.len(), 2);
    db.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sqlite_backup_copies_the_file_and_restores_dumps() {
    use kiyi_core::backup::{self, BackupMethod, Target};
    let dir = std::env::temp_dir().join(format!("kiyi-sqlite-backup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = |file: &str| ConnectionConfig {
        id: "b".into(),
        name: "b".into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(dir.join(file).to_string_lossy().into_owned()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: Default::default(),
    };
    let src = drivers::open(&cfg("src.db"), None).await.unwrap();
    src.execute_script(&["CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)".into(), "INSERT INTO t (name) VALUES ('Ayşe'), ('it''s; fine')".into()], true, false).await.unwrap();

    let copy = dir.join("copy.db");
    let report = backup::backup(&*src, &Target { config: cfg("src.db"), password: None }, &copy, false).await.unwrap();
    assert_eq!(report.method, BackupMethod::Native);
    let copied = drivers::open(&cfg("copy.db"), None).await.unwrap();
    assert_eq!(copied.fetch("SELECT name FROM t ORDER BY id").await.unwrap().1, src.fetch("SELECT name FROM t ORDER BY id").await.unwrap().1);

    // What `sqlite3 db .dump` writes, restored into an empty database.
    let dump = dir.join("dump.sql");
    std::fs::write(&dump, "PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\nCREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT);\nINSERT INTO t VALUES(1,'Ayşe');\nINSERT INTO t VALUES(2,'it''s; fine');\nCOMMIT;\n").unwrap();
    let empty = drivers::open(&cfg("empty.db"), None).await.unwrap();
    backup::restore(&*empty, &Target { config: cfg("empty.db"), password: None }, &dump).await.unwrap();
    assert_eq!(empty.fetch("SELECT name FROM t ORDER BY id").await.unwrap().1, src.fetch("SELECT name FROM t ORDER BY id").await.unwrap().1);

    for d in [src, copied, empty] {
        d.close().await;
    }
    let _ = std::fs::remove_dir_all(&dir);
}
