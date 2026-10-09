//! SQL for browsing table data (filters, sorting, paging) and for applying grid edits.

use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;
use crate::types::Cell;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    IsNull,
    NotNull,
    In,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseRequest {
    pub schema: Option<String>,
    pub table: String,
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Hand-written condition, ANDed with the filters.
    #[serde(default)]
    pub raw_where: Option<String>,
    /// Free text matched (case-insensitively) against any of `search_columns`.
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub search_columns: Vec<String>,
    #[serde(default)]
    pub sort: Vec<Sort>,
    /// Appended to `sort` so paging is stable (normally the primary key).
    #[serde(default)]
    pub tiebreak: Vec<String>,
    pub limit: u32,
    pub offset: u64,
}

/// Checks that a hand-written (or AI-written) condition is a single boolean expression,
/// so it can't smuggle in a second statement such as `1=1; DROP TABLE t`, close the WHERE
/// clause early (`1=1) UNION SELECT …`), or call functions with side effects.
pub fn validate_condition(d: Dialect, condition: &str) -> Result<(), String> {
    use sqlparser::dialect::{Dialect as SqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
    use sqlparser::parser::Parser;
    use sqlparser::tokenizer::{Token, Tokenizer};
    let dialect: &dyn SqlDialect = match d.kind {
        crate::config::DbKind::Mysql => &MySqlDialect {},
        crate::config::DbKind::Sqlite => &SQLiteDialect {},
        crate::config::DbKind::Postgres => &PostgreSqlDialect {},
    };
    let invalid = |e: &dyn std::fmt::Display| format!("The condition is not valid SQL: {e}");
    let mut parser = Parser::new(dialect).try_with_sql(condition).map_err(|e| invalid(&e))?;
    parser.parse_expr().map_err(|e| invalid(&e))?;
    if parser.peek_token().token != Token::EOF {
        return Err("The condition must be a single expression.".into());
    }
    // Anything shaped like a call to a function that changes state, sleeps or reads files.
    let tokens = Tokenizer::new(dialect, condition).tokenize().map_err(|e| invalid(&e))?;
    let significant: Vec<&Token> = tokens.iter().filter(|t| !matches!(t, Token::Whitespace(_))).collect();
    for pair in significant.windows(2) {
        if let (Token::Word(w), Token::LParen) = (pair[0], pair[1]) {
            let name = w.value.to_ascii_lowercase();
            if UNSAFE_FUNCTIONS.iter().any(|f| name == *f || (f.ends_with('_') && name.starts_with(f))) {
                return Err(format!("The condition can't call {name}()."));
            }
        }
    }
    Ok(())
}

/// Functions a filter has no business calling. A trailing `_` matches a whole family.
const UNSAFE_FUNCTIONS: &[&str] = &[
    "set_config", "pg_terminate_backend", "pg_cancel_backend", "pg_reload_conf", "pg_rotate_logfile", "pg_sleep", "pg_sleep_for",
    "pg_sleep_until", "pg_read_file", "pg_read_binary_file", "pg_ls_dir", "pg_stat_file", "pg_advisory_", "pg_try_advisory_",
    "pg_notify", "lo_", "dblink", "dblink_", "nextval", "setval", "txid_current", "pg_create_", "pg_drop_", "pg_switch_wal",
    "query_to_xml", "query_to_json", "sleep", "benchmark", "load_file", "get_lock", "release_lock", "release_all_locks",
    "master_pos_wait", "source_pos_wait", "load_extension", "readfile", "writefile", "edit",
];

/// What a script would do, judged from its keywords (outside strings and comments), so it
/// works for SQL the parser doesn't fully understand.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptCheck {
    /// Something other than reading: data or schema changes, procedures, or switching the
    /// session out of read-only.
    pub writes: bool,
    /// Statements that delete or overwrite data (DROP, TRUNCATE, DELETE, UPDATE, ALTER … DROP).
    pub destructive: usize,
}

const WRITE_KEYWORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "MERGE", "UPSERT", "REPLACE", "CREATE", "ALTER", "DROP", "TRUNCATE", "RENAME", "GRANT", "REVOKE", "COPY",
    "CALL", "DO", "EXECUTE", "EXEC", "VACUUM", "ANALYZE", "REINDEX", "CLUSTER", "REFRESH", "COMMENT", "LOCK", "IMPORT", "LOAD", "ATTACH",
    "DETACH", "REASSIGN", "SECURITY", "DISCARD", "HANDLER", "OPTIMIZE", "REPAIR", "INSTALL", "UNINSTALL", "FLUSH", "PURGE", "RESET",
];

pub fn check_script(d: Dialect, sql: &str) -> ScriptCheck {
    use sqlparser::dialect::{Dialect as SqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
    use sqlparser::tokenizer::{Token, Tokenizer};
    let dialect: &dyn SqlDialect = match d.kind {
        crate::config::DbKind::Mysql => &MySqlDialect {},
        crate::config::DbKind::Sqlite => &SQLiteDialect {},
        crate::config::DbKind::Postgres => &PostgreSqlDialect {},
    };
    let Ok(tokens) = Tokenizer::new(dialect, sql).tokenize() else {
        // Can't even tokenize: assume the worst so read-only and production checks still apply.
        return ScriptCheck { writes: true, destructive: 0 };
    };
    let mut check = ScriptCheck::default();
    let mut statement: Vec<String> = Vec::new();
    let mut finish = |words: &mut Vec<String>| {
        let first = words.first().map(String::as_str).unwrap_or("");
        let has = |w: &str| words.iter().any(|x| x == w);
        // Session switches that would undo read-only protection.
        let unprotects = (has("READ") && has("WRITE"))
            || words.iter().any(|w| w.contains("READ_ONLY") || w == "SQL_SAFE_UPDATES");
        // Judge by the statement's command word: `SELECT comment, replace(name, …)` only reads.
        let dml = || ["INSERT", "UPDATE", "DELETE", "MERGE"].iter().any(|w| has(w));
        let writes = WRITE_KEYWORDS.contains(&first)
            || (first == "WITH" && dml())
            || (first == "EXPLAIN" && has("ANALYZE") && dml())
            || (first == "SELECT" && has("INTO"))
            || unprotects;
        if writes {
            check.writes = true;
        }
        if matches!(first, "DROP" | "TRUNCATE" | "DELETE" | "UPDATE") || (first == "ALTER" && has("DROP")) || (first == "WITH" && (has("DELETE") || has("UPDATE"))) {
            check.destructive += 1;
        }
        words.clear();
    };
    for token in tokens {
        match token {
            Token::SemiColon => finish(&mut statement),
            // Only bare words are keywords; "update" in quotes is a name.
            Token::Word(w) if w.quote_style.is_none() => statement.push(w.value.to_ascii_uppercase()),
            _ => {}
        }
    }
    finish(&mut statement);
    check
}

fn like_pattern(value: &str, prefix: bool, suffix: bool) -> String {
    let mut out = String::new();
    if prefix {
        out.push('%');
    }
    for c in value.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    if suffix {
        out.push('%');
    }
    out
}

fn condition(d: Dialect, f: &Filter) -> String {
    let col = d.ident(&f.column);
    // Text matching works on any type: compare the column's text form.
    let as_text = match d.kind {
        crate::config::DbKind::Mysql => format!("CAST({col} AS CHAR)"),
        crate::config::DbKind::Sqlite => format!("CAST({col} AS TEXT)"),
        crate::config::DbKind::Postgres => format!("{col}::text"),
    };
    let like = if d.kind == crate::config::DbKind::Postgres { "ILIKE" } else { "LIKE" };
    // SQLite has no default LIKE escape character.
    let esc = if d.is_sqlite() { " ESCAPE '\\'" } else { "" };
    let v = &f.value;
    match f.op {
        FilterOp::Eq => format!("{col} = {}", d.string(v)),
        FilterOp::Ne => format!("{col} <> {}", d.string(v)),
        FilterOp::Lt => format!("{col} < {}", d.string(v)),
        FilterOp::Gt => format!("{col} > {}", d.string(v)),
        FilterOp::Le => format!("{col} <= {}", d.string(v)),
        FilterOp::Ge => format!("{col} >= {}", d.string(v)),
        FilterOp::Contains => format!("{as_text} {like} {}{esc}", d.string(&like_pattern(v, true, true))),
        FilterOp::NotContains => format!("{as_text} NOT {like} {}{esc}", d.string(&like_pattern(v, true, true))),
        FilterOp::StartsWith => format!("{as_text} {like} {}{esc}", d.string(&like_pattern(v, false, true))),
        FilterOp::EndsWith => format!("{as_text} {like} {}{esc}", d.string(&like_pattern(v, true, false))),
        FilterOp::IsNull => format!("{col} IS NULL"),
        FilterOp::NotNull => format!("{col} IS NOT NULL"),
        FilterOp::In => {
            let items: Vec<String> =
                v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| d.string(s)).collect();
            if items.is_empty() { "FALSE".into() } else { format!("{col} IN ({})", items.join(", ")) }
        }
    }
}

fn where_clause(d: Dialect, req: &BrowseRequest) -> String {
    let mut parts: Vec<String> = req.filters.iter().map(|f| condition(d, f)).collect();
    if let Some(term) = req.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let any: Vec<String> = req
            .search_columns
            .iter()
            .map(|c| condition(d, &Filter { column: c.clone(), op: FilterOp::Contains, value: term.to_string() }))
            .collect();
        if !any.is_empty() {
            parts.push(format!("({})", any.join(" OR ")));
        }
    }
    if let Some(raw) = req.raw_where.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("({raw})"));
    }
    if parts.is_empty() { String::new() } else { format!(" WHERE {}", parts.join(" AND ")) }
}

pub fn browse_sql(d: Dialect, req: &BrowseRequest) -> String {
    let mut sql = format!("SELECT * FROM {}{}", d.table(req.schema.as_deref(), &req.table), where_clause(d, req));
    let mut order: Vec<String> =
        req.sort.iter().map(|s| format!("{}{}", d.ident(&s.column), if s.descending { " DESC" } else { "" })).collect();
    // The tiebreak follows the direction of the last sort, so `ORDER BY created_at DESC, id DESC`
    // can be read backwards off an index on `created_at` (MySQL can't do that with mixed directions).
    let tie_desc = req.sort.last().is_some_and(|s| s.descending);
    for key in &req.tiebreak {
        if !req.sort.iter().any(|s| &s.column == key) {
            order.push(format!("{}{}", d.ident(key), if tie_desc { " DESC" } else { "" }));
        }
    }
    if !order.is_empty() {
        sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
    }
    sql.push_str(&format!(" LIMIT {}", req.limit));
    if req.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", req.offset));
    }
    sql
}

/// Every row matching the request, in order, without paging (for export).
pub fn export_sql(d: Dialect, req: &BrowseRequest) -> String {
    let paged = browse_sql(d, &BrowseRequest { offset: 0, ..req.clone() });
    paged.rsplit_once(" LIMIT ").map(|(head, _)| head.to_string()).unwrap_or(paged)
}

/// Multi-row INSERTs, `batch` rows per statement, for imports.
pub fn plan_bulk_insert(d: Dialect, schema: Option<&str>, table: &str, columns: &[String], rows: &[Vec<Cell>], batch: usize) -> Vec<String> {
    let head = format!("INSERT INTO {} ({}) VALUES ", d.table(schema, table), d.ident_list(columns));
    rows.chunks(batch.max(1))
        .map(|chunk| {
            let values: Vec<String> = chunk
                .iter()
                .map(|row| format!("({})", row.iter().map(|v| d.value(v.as_deref(), false)).collect::<Vec<_>>().join(", ")))
                .collect();
            format!("{head}{}", values.join(", "))
        })
        .collect()
}

pub fn count_sql(d: Dialect, req: &BrowseRequest) -> String {
    format!("SELECT COUNT(*) FROM {}{}", d.table(req.schema.as_deref(), &req.table), where_clause(d, req))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnValue {
    pub column: String,
    pub value: Cell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RowChange {
    /// `key` identifies the row by its primary-key values as last read.
    Update { key: Vec<ColumnValue>, values: Vec<ColumnValue> },
    /// Columns left out get their default.
    Insert { values: Vec<ColumnValue> },
    Delete { key: Vec<ColumnValue> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    pub schema: Option<String>,
    pub table: String,
    /// Columns whose values are shown as `0x…` hex and must be written as binary.
    #[serde(default)]
    pub binary_columns: Vec<String>,
    /// True/false columns; SQLite stores these as 1/0.
    #[serde(default)]
    pub bool_columns: Vec<String>,
    pub changes: Vec<RowChange>,
}

pub fn plan_changes(d: Dialect, set: &ChangeSet) -> Vec<String> {
    let table = d.table(set.schema.as_deref(), &set.table);
    let value = |cv: &ColumnValue| {
        if d.is_sqlite() && set.bool_columns.contains(&cv.column) {
            match cv.value.as_deref() {
                Some("true") => return "1".to_string(),
                Some("false") => return "0".to_string(),
                _ => {}
            }
        }
        d.value(cv.value.as_deref(), set.binary_columns.contains(&cv.column))
    };
    let key_where = |key: &[ColumnValue]| {
        key.iter()
            .map(|k| match &k.value {
                None => format!("{} IS NULL", d.ident(&k.column)),
                Some(_) => format!("{} = {}", d.ident(&k.column), value(k)),
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    // MySQL allows LIMIT on single-table UPDATE/DELETE: a guard in case the key isn't unique.
    let limit = if d.is_mysql() { " LIMIT 1" } else { "" };

    set.changes
        .iter()
        .map(|change| match change {
            RowChange::Update { key, values } => format!(
                "UPDATE {table} SET {} WHERE {}{limit}",
                values.iter().map(|v| format!("{} = {}", d.ident(&v.column), value(v))).collect::<Vec<_>>().join(", "),
                key_where(key)
            ),
            RowChange::Insert { values } if values.is_empty() => {
                if d.is_mysql() { format!("INSERT INTO {table} () VALUES ()") } else { format!("INSERT INTO {table} DEFAULT VALUES") }
            }
            RowChange::Insert { values } => format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                values.iter().map(|v| d.ident(&v.column)).collect::<Vec<_>>().join(", "),
                values.iter().map(value).collect::<Vec<_>>().join(", ")
            ),
            RowChange::Delete { key } => format!("DELETE FROM {table} WHERE {}{limit}", key_where(key)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: Dialect = Dialect::POSTGRES;
    const MY: Dialect = Dialect::MYSQL;

    fn req() -> BrowseRequest {
        BrowseRequest {
            schema: Some("public".into()),
            table: "orders".into(),
            filters: vec![],
            raw_where: None,
            search: None,
            search_columns: vec![],
            sort: vec![],
            tiebreak: vec!["id".into()],
            limit: 200,
            offset: 0,
        }
    }

    #[test]
    fn browse_with_filters_and_sort() {
        let mut r = req();
        r.filters = vec![
            Filter { column: "status".into(), op: FilterOp::Eq, value: "paid".into() },
            Filter { column: "note".into(), op: FilterOp::Contains, value: "50%_off".into() },
            Filter { column: "id".into(), op: FilterOp::In, value: "1, 2,,3".into() },
        ];
        r.raw_where = Some("total > 10".into());
        r.sort = vec![Sort { column: "total".into(), descending: true }];
        r.offset = 400;
        assert_eq!(
            browse_sql(PG, &r),
            r#"SELECT * FROM "public"."orders" WHERE "status" = 'paid' AND "note"::text ILIKE '%50\%\_off%' AND "id" IN ('1', '2', '3') AND (total > 10) ORDER BY "total" DESC, "id" DESC LIMIT 200 OFFSET 400"#
        );
        assert_eq!(
            count_sql(MY, &r),
            r#"SELECT COUNT(*) FROM `public`.`orders` WHERE `status` = 'paid' AND CAST(`note` AS CHAR) LIKE '%50\\%\\_off%' AND `id` IN ('1', '2', '3') AND (total > 10)"#
        );
    }

    #[test]
    fn conditions_must_be_a_single_expression() {
        assert!(validate_condition(PG, "total > 10 AND note ILIKE '%x%'").is_ok());
        assert!(validate_condition(MY, "placed_on >= CURDATE() - INTERVAL 7 DAY").is_ok());
        assert!(validate_condition(PG, "1=1); DROP TABLE orders; --").is_err());
        assert!(validate_condition(PG, "1=1; DROP TABLE orders").is_err());
        assert!(validate_condition(PG, "total > ").is_err());
        // Closing the WHERE clause early used to slip through.
        assert!(validate_condition(PG, "1=1) UNION SELECT 1 FROM pg_shadow WHERE (1=1").is_err());
        assert!(validate_condition(PG, "1=1) OR (1=1").is_err());
        assert!(validate_condition(PG, "set_config('default_transaction_read_only', 'off', false) IS NOT NULL").is_err());
        assert!(validate_condition(PG, "PG_SLEEP (10) IS NULL").is_err());
        assert!(validate_condition(MY, "SLEEP(5) = 0").is_err());
        assert!(validate_condition(PG, "pg_advisory_lock(1) IS NULL").is_err());
        // Ordinary functions and subqueries are fine.
        assert!(validate_condition(PG, "(status = 'paid' OR status = 'shipped') AND lower(note) LIKE '%x%'").is_ok());
        assert!(validate_condition(PG, "customer_id IN (SELECT id FROM customers WHERE is_active)").is_ok());
        assert!(validate_condition(PG, "placed_on >= current_date - interval '30 days'").is_ok());
    }

    #[test]
    fn checks_what_a_script_does() {
        let c = |sql: &str| check_script(PG, sql);
        assert_eq!(c("SELECT * FROM orders WHERE note = 'please delete me'; -- drop table x"), ScriptCheck { writes: false, destructive: 0 });
        assert_eq!(c("select \"update\" from t"), ScriptCheck { writes: false, destructive: 0 });
        assert_eq!(c("INSERT INTO t VALUES (1)"), ScriptCheck { writes: true, destructive: 0 });
        assert_eq!(c("delete from t; drop table u; update t set a = 1"), ScriptCheck { writes: true, destructive: 3 });
        assert_eq!(c("ALTER TABLE t DROP COLUMN a"), ScriptCheck { writes: true, destructive: 1 });
        assert_eq!(c("WITH gone AS (DELETE FROM t RETURNING *) SELECT count(*) FROM gone"), ScriptCheck { writes: true, destructive: 1 });
        assert!(c("SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE").writes);
        assert!(c("set default_transaction_read_only = off").writes);
        assert!(c("BEGIN READ WRITE").writes);
        assert!(check_script(MY, "SET SESSION TRANSACTION READ WRITE").writes);
        assert!(!c("SHOW search_path; EXPLAIN SELECT 1").writes);
        assert!(!c("SELECT comment, replace(name, 'a', 'b'), lock_timeout FROM posts").writes);
        assert!(c("EXPLAIN ANALYZE DELETE FROM t").writes);
        assert!(c("SELECT * INTO backup FROM t").writes);
        assert!(!c("EXPLAIN SELECT * FROM t").writes);
    }

    #[test]
    fn search_matches_any_column() {
        let mut r = req();
        r.search = Some(" o'b ".into());
        r.search_columns = vec!["name".into(), "email".into()];
        assert_eq!(
            count_sql(PG, &r),
            r#"SELECT COUNT(*) FROM "public"."orders" WHERE ("name"::text ILIKE '%o''b%' OR "email"::text ILIKE '%o''b%')"#
        );
    }

    #[test]
    fn export_and_bulk_insert() {
        let mut r = req();
        r.offset = 600;
        assert_eq!(export_sql(PG, &r), r#"SELECT * FROM "public"."orders" ORDER BY "id""#);
        let rows = vec![vec![Some("a".to_string()), None], vec![Some("it's".to_string()), Some("2".to_string())], vec![Some("c".into()), Some("3".into())]];
        let sql = plan_bulk_insert(MY, None, "t", &["name".into(), "n".into()], &rows, 2);
        assert_eq!(sql, vec!["INSERT INTO `t` (`name`, `n`) VALUES ('a', NULL), ('it''s', '2')", "INSERT INTO `t` (`name`, `n`) VALUES ('c', '3')"]);
    }

    #[test]
    fn sort_on_key_is_not_repeated() {
        let mut r = req();
        r.sort = vec![Sort { column: "id".into(), descending: true }];
        assert!(browse_sql(PG, &r).ends_with(r#"ORDER BY "id" DESC LIMIT 200"#));
    }

    #[test]
    fn row_changes() {
        let cv = |c: &str, v: Option<&str>| ColumnValue { column: c.into(), value: v.map(Into::into) };
        let set = ChangeSet {
            schema: None,
            table: "customers".into(),
            binary_columns: vec!["public_id".into()],
            bool_columns: vec![],
            changes: vec![
                RowChange::Update { key: vec![cv("id", Some("7"))], values: vec![cv("name", Some("O'Brien")), cv("bio", None)] },
                RowChange::Insert { values: vec![cv("name", Some("x")), cv("public_id", Some("0xabcd"))] },
                RowChange::Insert { values: vec![] },
                RowChange::Delete { key: vec![cv("id", Some("9")), cv("tenant", None)] },
            ],
        };
        assert_eq!(
            plan_changes(MY, &set),
            vec![
                "UPDATE `customers` SET `name` = 'O''Brien', `bio` = NULL WHERE `id` = '7' LIMIT 1",
                "INSERT INTO `customers` (`name`, `public_id`) VALUES ('x', X'abcd')",
                "INSERT INTO `customers` () VALUES ()",
                "DELETE FROM `customers` WHERE `id` = '9' AND `tenant` IS NULL LIMIT 1",
            ]
        );
        assert_eq!(plan_changes(PG, &set)[2], r#"INSERT INTO "customers" DEFAULT VALUES"#);
    }
}
