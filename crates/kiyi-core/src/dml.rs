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
/// so it can't smuggle in a second statement such as `1=1; DROP TABLE t`.
pub fn validate_condition(d: Dialect, condition: &str) -> Result<(), String> {
    use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect, SQLiteDialect};
    use sqlparser::parser::Parser;
    let sql = format!("SELECT 1 FROM t WHERE ({condition})");
    let parsed = match d.kind {
        crate::config::DbKind::Mysql => Parser::parse_sql(&MySqlDialect {}, &sql),
        crate::config::DbKind::Sqlite => Parser::parse_sql(&SQLiteDialect {}, &sql),
        crate::config::DbKind::Postgres => Parser::parse_sql(&PostgreSqlDialect {}, &sql),
    };
    match parsed {
        Ok(statements) if statements.len() == 1 && matches!(statements[0], sqlparser::ast::Statement::Query(_)) => Ok(()),
        Ok(_) => Err("The condition must be a single expression.".into()),
        Err(e) => Err(format!("The condition is not valid SQL: {e}")),
    }
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
