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
