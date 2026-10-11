//! Whether a query's result can be edited in place: it reads one table, every row of the result
//! is one row of that table, and the table's key is in the result. Then a changed cell becomes an
//! UPDATE of that row by its key, exactly like an edit in the table view.

use serde::Serialize;
use sqlparser::ast::{Expr, GroupByExpr, ObjectNamePart, SelectItem, SetExpr, Statement, TableFactor};
use sqlparser::parser::Parser;

use crate::dialect::Dialect;

/// What a query reads from, as written in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub schema: Option<String>,
    pub table: String,
    /// Result column name → table column. `*` stands for every column of the table.
    pub columns: Vec<(String, String)>,
    pub star: bool,
}

/// The table behind a result and, for each result column, the table column it shows (None for
/// computed columns, which stay read-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableResult {
    pub schema: Option<String>,
    pub table: String,
    pub key: Vec<String>,
    pub columns: Vec<Option<String>>,
}

fn ident_value(i: &sqlparser::ast::Ident) -> String {
    i.value.clone()
}

/// The single table a plain SELECT reads, if that's all it does.
pub(crate) fn source(d: Dialect, sql: &str) -> Option<Source> {
    let mut statements = Parser::parse_sql(crate::dml::parser_dialect(d), sql).ok()?;
    if statements.len() != 1 {
        return None;
    }
    let Statement::Query(query) = statements.remove(0) else { return None };
    if query.with.is_some() {
        return None;
    }
    let SetExpr::Select(select) = *query.body else { return None };
    let grouped = match &select.group_by {
        GroupByExpr::All(_) => true,
        GroupByExpr::Expressions(e, _) => !e.is_empty(),
    };
    if select.distinct.is_some() || grouped || select.having.is_some() || select.from.len() != 1 || !select.from[0].joins.is_empty() || select.into.is_some() {
        return None;
    }
    let TableFactor::Table { name, args: None, .. } = &select.from[0].relation else { return None };
    let parts: Vec<String> = name
        .0
        .iter()
        .filter_map(|p| match p {
            ObjectNamePart::Identifier(i) => Some(ident_value(i)),
            #[allow(unreachable_patterns)]
            _ => None,
        })
        .collect();
    let (schema, table) = match parts.as_slice() {
        [t] => (None, t.clone()),
        [s, t] => (Some(s.clone()), t.clone()),
        [_, s, t] => (Some(s.clone()), t.clone()),
        _ => return None,
    };
    let mut columns = Vec::new();
    let mut star = false;
    for item in &select.projection {
        match item {
            SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(..) => star = true,
            SelectItem::UnnamedExpr(e) => match e {
                Expr::Identifier(i) => columns.push((ident_value(i), ident_value(i))),
                Expr::CompoundIdentifier(parts) => {
                    let c = ident_value(parts.last()?);
                    columns.push((c.clone(), c));
                }
                // Computed: shown, not editable. Its result name isn't known here, so nothing is recorded.
                _ => {}
            },
            SelectItem::ExprWithAlias { expr, alias } => match expr {
                Expr::Identifier(i) => columns.push((ident_value(alias), ident_value(i))),
                Expr::CompoundIdentifier(parts) => columns.push((ident_value(alias), ident_value(parts.last()?))),
                _ => columns.push((ident_value(alias), String::new())),
            },
            #[allow(unreachable_patterns)]
            _ => return None,
        }
    }
    Some(Source { schema, table, columns, star })
}

/// Maps the result's columns onto the table, given the table's columns and key. None when the
/// result can't be edited (no key, the key isn't shown, or a name means two things).
pub(crate) fn map_result(src: &Source, table_columns: &[String], key: &[String], result_columns: &[String]) -> Option<Vec<Option<String>>> {
    if key.is_empty() {
        return None;
    }
    let mapped: Vec<Option<String>> = result_columns
        .iter()
        .map(|name| {
            let explicit: Vec<&String> = src.columns.iter().filter(|(r, _)| r == name).map(|(_, c)| c).collect();
            match explicit.as_slice() {
                [c] if !c.is_empty() && table_columns.contains(c) => Some((*c).clone()),
                [] if src.star && table_columns.contains(name) => Some(name.clone()),
                _ => None,
            }
        })
        .collect();
    // A name shown twice can't be told apart when saving.
    let mut seen = std::collections::HashSet::new();
    if result_columns.iter().any(|n| !seen.insert(n)) {
        return None;
    }
    key.iter().all(|k| mapped.iter().any(|m| m.as_deref() == Some(k.as_str()))).then_some(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_selects_of_one_table_are_editable() {
        let s = source(Dialect::POSTGRES, "SELECT id, name AS customer, email FROM public.customers WHERE plan = 'pro' ORDER BY name LIMIT 50").unwrap();
        assert_eq!((s.schema.as_deref(), s.table.as_str(), s.star), (Some("public"), "customers", false));
        let m = map_result(&s, &cols(&["id", "name", "email", "plan"]), &cols(&["id"]), &cols(&["id", "customer", "email"])).unwrap();
        assert_eq!(m, vec![Some("id".into()), Some("name".into()), Some("email".into())]);

        let s = source(Dialect::MYSQL, "SELECT *, lifetime_value * 2 AS doubled FROM `customers`").unwrap();
        let m = map_result(&s, &cols(&["id", "name"]), &cols(&["id"]), &cols(&["id", "name", "doubled"])).unwrap();
        assert_eq!(m, vec![Some("id".into()), Some("name".into()), None], "the computed column stays read-only");

        assert!(source(Dialect::SQLSERVER, "SELECT TOP 10 [id], [name] FROM [dbo].[customers]").is_some());
    }

    #[test]
    fn anything_that_isnt_one_row_per_table_row_is_not() {
        for sql in [
            "SELECT c.id, o.total FROM customers c JOIN orders o ON o.customer_id = c.id",
            "SELECT country, count(*) FROM customers GROUP BY country",
            "SELECT DISTINCT country FROM customers",
            "SELECT id FROM customers UNION SELECT id FROM orders",
            "WITH x AS (SELECT * FROM customers) SELECT * FROM x",
            "SELECT * FROM (SELECT * FROM customers) t",
            "SELECT 1",
            "UPDATE customers SET name = 'x'",
            "SELECT * FROM customers; SELECT * FROM orders",
        ] {
            assert!(source(Dialect::POSTGRES, sql).is_none(), "{sql}");
        }
    }

    #[test]
    fn the_key_must_be_in_the_result() {
        let s = source(Dialect::POSTGRES, "SELECT name, email FROM customers").unwrap();
        assert!(map_result(&s, &cols(&["id", "name", "email"]), &cols(&["id"]), &cols(&["name", "email"])).is_none());
        assert!(map_result(&s, &cols(&["id", "name", "email"]), &[], &cols(&["name", "email"])).is_none(), "no key, no edits");
        let s = source(Dialect::POSTGRES, "SELECT id, name, name FROM customers").unwrap();
        assert!(map_result(&s, &cols(&["id", "name"]), &cols(&["id"]), &cols(&["id", "name", "name"])).is_none());
    }
}
