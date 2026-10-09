//! Query plans in one shape for every database, so the UI can show a readable tree and point
//! out what's slow ("reads every row of orders") without the user decoding EXPLAIN output.
//! Plans are estimates: nothing is executed (no ANALYZE).

use serde::Serialize;
use serde_json::Value;

use crate::config::DbKind;
use crate::types::Cell;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanNode {
    /// What happens, e.g. "Seq Scan on orders" or "Index Scan using orders_pkey on orders".
    pub label: String,
    /// Conditions and keys, e.g. "Filter: (total > 100)".
    pub detail: Option<String>,
    /// Estimated rows this step produces.
    pub rows: Option<f64>,
    /// Estimated total cost, in the database's own units.
    pub cost: Option<f64>,
    /// Plain-language note when the step is likely slow.
    pub warning: Option<String>,
    pub children: Vec<PlanNode>,
}

/// Rows above which reading a whole table is worth pointing out.
const BIG_SCAN: f64 = 1_000.0;

fn scan_warning(table: &str) -> String {
    format!("Reads every row of {table}. If this query is slow, an index on the columns it filters or joins by may help.")
}

/// The EXPLAIN statement for `sql` (one statement, trailing `;` removed).
pub fn explain_sql(kind: DbKind, sql: &str) -> String {
    let sql = sql.trim().trim_end_matches(';').trim();
    match kind {
        DbKind::Postgres => format!("EXPLAIN (FORMAT JSON) {sql}"),
        DbKind::Mysql => format!("EXPLAIN FORMAT=TREE {sql}"),
        DbKind::Sqlite => format!("EXPLAIN QUERY PLAN {sql}"),
        // Run with SHOWPLAN_TEXT on (see the driver); the statement itself is sent as is.
        DbKind::Sqlserver => sql.to_string(),
    }
}

/// MariaDB and old MySQL have no tree format; their classic table is the fallback.
pub fn explain_classic_sql(sql: &str) -> String {
    format!("EXPLAIN {}", sql.trim().trim_end_matches(';').trim())
}

// ---- PostgreSQL

pub fn from_postgres(json: &str) -> Result<PlanNode, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Unexpected plan: {e}"))?;
    let plan = v.get(0).and_then(|p| p.get("Plan")).ok_or("Unexpected plan: no \"Plan\".")?;
    Ok(pg_node(plan))
}

fn pg_node(p: &Value) -> PlanNode {
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let node = s("Node Type").unwrap_or("Step");
    let relation = s("Relation Name");
    let mut label = match (node, s("Join Type")) {
        (n, Some(j)) if n.contains("Join") || n == "Nested Loop" => format!("{n} ({j})"),
        (n, _) => n.to_string(),
    };
    if let Some(index) = s("Index Name") {
        label.push_str(&format!(" using {index}"));
    }
    if let Some(rel) = relation {
        label.push_str(&format!(" on {rel}"));
        if let Some(alias) = s("Alias").filter(|a| *a != rel) {
            label.push_str(&format!(" {alias}"));
        }
    }
    let mut details: Vec<String> = ["Index Cond", "Hash Cond", "Merge Cond", "Join Filter", "Filter", "Recheck Cond"]
        .iter()
        .filter_map(|k| s(k).map(|v| format!("{k}: {v}")))
        .collect();
    if let Some(keys) = p.get("Sort Key").and_then(Value::as_array) {
        details.push(format!("Sort Key: {}", keys.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
    }
    if let Some(keys) = p.get("Group Key").and_then(Value::as_array) {
        details.push(format!("Group Key: {}", keys.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
    }
    let rows = p.get("Plan Rows").and_then(Value::as_f64);
    let warning = match (node, relation) {
        ("Seq Scan", Some(rel)) if rows.unwrap_or(0.0) >= BIG_SCAN || details.iter().any(|d| d.starts_with("Filter")) => Some(scan_warning(rel)),
        _ => None,
    };
    PlanNode {
        label,
        detail: (!details.is_empty()).then(|| details.join("\n")),
        rows,
        cost: p.get("Total Cost").and_then(Value::as_f64),
        warning,
        children: p.get("Plans").and_then(Value::as_array).map(|c| c.iter().map(pg_node).collect()).unwrap_or_default(),
    }
}

// ---- MySQL

/// `EXPLAIN FORMAT=TREE` text: one step per `->` line, nesting by indentation.
pub fn from_mysql_tree(text: &str) -> Result<PlanNode, String> {
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    let mut roots: Vec<PlanNode> = Vec::new();
    for line in text.lines() {
        let Some(arrow) = line.find("->") else { continue };
        let depth = line[..arrow].chars().count();
        let body = line[arrow + 2..].trim();
        let (step, figures) = match body.rfind("  (") {
            Some(i) => (body[..i].trim(), &body[i..]),
            None => (body, ""),
        };
        let num = |key: &str| {
            figures.find(&format!("{key}=")).and_then(|i| {
                let rest = &figures[i + key.len() + 1..];
                let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == 'e' || c == '+' || c == '-')).unwrap_or(rest.len());
                rest[..end].parse::<f64>().ok()
            })
        };
        let (label, detail) = match step.split_once(": ") {
            Some((l, d)) if !l.contains(' ') || l.starts_with("Filter") || l.starts_with("Sort") => (l.to_string(), Some(d.to_string())),
            _ => (step.to_string(), None),
        };
        let rows = num("rows");
        let warning = step
            .strip_prefix("Table scan on ")
            .map(|t| t.split_whitespace().next().unwrap_or(t))
            .filter(|_| rows.unwrap_or(0.0) >= BIG_SCAN)
            .map(scan_warning);
        let node = PlanNode { label, detail, rows, cost: num("cost"), warning, children: vec![] };
        while let Some((d, _)) = stack.last() {
            if *d >= depth {
                let (_, done) = stack.pop().unwrap();
                attach(&mut stack, &mut roots, done);
            } else {
                break;
            }
        }
        stack.push((depth, node));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    single_root(roots)
}

fn attach(stack: &mut [(usize, PlanNode)], roots: &mut Vec<PlanNode>, node: PlanNode) {
    match stack.last_mut() {
        Some((_, parent)) => parent.children.push(node),
        None => roots.push(node),
    }
}

fn single_root(mut roots: Vec<PlanNode>) -> Result<PlanNode, String> {
    match roots.len() {
        0 => Err("The database returned no plan.".into()),
        1 => Ok(roots.remove(0)),
        _ => Ok(PlanNode { label: "Query".into(), children: roots, ..Default::default() }),
    }
}

/// Classic `EXPLAIN` table (MariaDB, older MySQL): one step per table, in join order.
pub fn from_mysql_classic(columns: &[String], rows: &[Vec<Cell>]) -> Result<PlanNode, String> {
    let col = |name: &str| columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let get = |row: &Vec<Cell>, name: &str| col(name).and_then(|i| row.get(i).cloned().flatten());
    let steps = rows
        .iter()
        .map(|r| {
            let table = get(r, "table").unwrap_or_else(|| "?".into());
            let access = get(r, "type").unwrap_or_default();
            let key = get(r, "key");
            let estimate = get(r, "rows").and_then(|v| v.parse::<f64>().ok());
            let label = match (&key, access.as_str()) {
                (Some(k), _) => format!("Read {table} using index {k}"),
                (None, "ALL") => format!("Table scan on {table}"),
                (None, a) => format!("Read {table} ({a})"),
            };
            PlanNode {
                warning: (access == "ALL" && estimate.unwrap_or(0.0) >= BIG_SCAN).then(|| scan_warning(&table)),
                label,
                detail: get(r, "Extra").filter(|e| !e.is_empty()),
                rows: estimate,
                cost: None,
                children: vec![],
            }
        })
        .collect();
    single_root(steps).map(|mut root| {
        if root.label != "Query" {
            root = PlanNode { label: "Query".into(), children: vec![root], ..Default::default() };
        }
        root
    })
}

// ---- SQL Server

/// SHOWPLAN_TEXT rows: `|--Operator(arguments)`, nested by how far `|--` is indented.
pub fn from_sqlserver_text(rows: &[Vec<Cell>]) -> Result<PlanNode, String> {
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    let mut roots: Vec<PlanNode> = Vec::new();
    for line in rows.iter().filter_map(|r| r.first().cloned().flatten()) {
        let Some(at) = line.find("|--") else { continue };
        let depth = line[..at].chars().count();
        let body = line[at + 3..].trim();
        let (op, args) = match body.find('(') {
            Some(i) => (body[..i].trim(), Some(body[i + 1..].trim_end_matches(')').to_string())),
            None => (body, None),
        };
        // OBJECT:([db].[schema].[table].[index]) names the table a scan reads.
        let table = args.as_deref().and_then(|a| a.split("OBJECT:(").nth(1)).and_then(|o| o.split('.').nth(2)).map(|t| t.trim_matches(|c| c == '[' || c == ']').to_string());
        let scan = matches!(op, "Table Scan" | "Clustered Index Scan") && args.as_deref().is_some_and(|a| !a.contains("SEEK:"));
        let label = match &table {
            Some(t) => format!("{op} on {t}"),
            None => op.to_string(),
        };
        let node = PlanNode { label, detail: args, warning: if scan { table.map(|t| scan_warning(&t)) } else { None }, ..Default::default() };
        while let Some((d, _)) = stack.last() {
            if *d >= depth {
                let (_, done) = stack.pop().unwrap();
                attach(&mut stack, &mut roots, done);
            } else {
                break;
            }
        }
        stack.push((depth, node));
    }
    while let Some((_, done)) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    single_root(roots)
}

// ---- SQLite

/// `EXPLAIN QUERY PLAN` rows: id, parent, notused, detail.
pub fn from_sqlite(rows: &[Vec<Cell>]) -> Result<PlanNode, String> {
    let parsed: Vec<(i64, i64, String)> = rows
        .iter()
        .filter_map(|r| {
            let n = |i: usize| r.get(i).cloned().flatten().and_then(|v| v.parse::<i64>().ok());
            Some((n(0)?, n(1)?, r.get(3).cloned().flatten()?))
        })
        .collect();
    fn build(parent: i64, all: &[(i64, i64, String)]) -> Vec<PlanNode> {
        all.iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, detail)| {
                let warning = detail
                    .strip_prefix("SCAN ")
                    .filter(|rest| !rest.contains("USING") && !rest.starts_with("CONSTANT"))
                    .map(|rest| scan_warning(rest.split_whitespace().next().unwrap_or(rest)));
                PlanNode { label: detail.clone(), warning, children: build(*id, all), ..Default::default() }
            })
            .collect()
    }
    single_root(build(0, &parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_plans_read_as_steps() {
        let json = r#"[{"Plan": {"Node Type": "Sort", "Total Cost": 120.5, "Plan Rows": 4000, "Sort Key": ["total DESC"],
            "Plans": [{"Node Type": "Seq Scan", "Relation Name": "orders", "Alias": "o", "Total Cost": 90.0, "Plan Rows": 4000, "Filter": "(total > 100)"},
                      {"Node Type": "Index Scan", "Index Name": "customers_pkey", "Relation Name": "customers", "Alias": "customers", "Plan Rows": 1}]}}]"#;
        let plan = from_postgres(json).unwrap();
        assert_eq!(plan.label, "Sort");
        assert_eq!(plan.detail.as_deref(), Some("Sort Key: total DESC"));
        assert_eq!(plan.children[0].label, "Seq Scan on orders o");
        assert!(plan.children[0].warning.as_deref().unwrap().contains("every row of orders"));
        assert_eq!(plan.children[1].label, "Index Scan using customers_pkey on customers");
        assert!(plan.children[1].warning.is_none());
    }

    #[test]
    fn mysql_trees_nest_by_indentation() {
        let text = "-> Sort: orders.total DESC  (cost=1012 rows=3300)\n    -> Filter: (orders.total > 100)  (cost=1012 rows=3300)\n        -> Table scan on orders  (cost=1012 rows=10000)\n";
        let plan = from_mysql_tree(text).unwrap();
        assert_eq!(plan.label, "Sort");
        assert_eq!(plan.detail.as_deref(), Some("orders.total DESC"));
        let scan = &plan.children[0].children[0];
        assert_eq!(scan.label, "Table scan on orders");
        assert_eq!(scan.rows, Some(10000.0));
        assert!(scan.warning.is_some());
    }

    #[test]
    fn sql_server_plans_nest_by_indentation() {
        let row = |s: &str| vec![Some(s.to_string())];
        let plan = from_sqlserver_text(&[
            row("  |--Sort(ORDER BY:([o].[total] DESC))"),
            row("       |--Clustered Index Scan(OBJECT:([shop].[dbo].[orders].[PK_orders] AS [o]), WHERE:([o].[total]>(100)))"),
        ])
        .unwrap();
        assert_eq!(plan.label, "Sort");
        assert_eq!(plan.children[0].label, "Clustered Index Scan on orders");
        assert!(plan.children[0].warning.is_some());
    }

    #[test]
    fn sqlite_plans_follow_parent_ids() {
        let row = |id: &str, parent: &str, detail: &str| vec![Some(id.to_string()), Some(parent.to_string()), Some("0".to_string()), Some(detail.to_string())];
        let plan = from_sqlite(&[row("3", "0", "SCAN orders"), row("5", "0", "SEARCH customers USING INTEGER PRIMARY KEY (rowid=?)")]).unwrap();
        assert_eq!(plan.label, "Query");
        assert!(plan.children[0].warning.is_some());
        assert!(plan.children[1].warning.is_none());
    }
}
