//! How a database's tables connect: primary keys and foreign keys for every table at once, read
//! from the catalog in one query, for the schema diagram and the database comparison.

use serde::Serialize;

use crate::config::DbKind;
use crate::types::Cell;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    /// Empty when the database leaves it implicit (SQLite referencing the primary key).
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryKey {
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaGraph {
    pub primary_keys: Vec<PrimaryKey>,
    pub relations: Vec<Relation>,
}

const POSTGRES: &str = r#"
SELECT n.nspname, c.relname, con.contype::text, con.conname,
  array_to_string(ARRAY(SELECT a.attname FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord)
    JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum ORDER BY k.ord), chr(31)),
  rn.nspname, rc.relname,
  array_to_string(ARRAY(SELECT a.attname FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord)
    JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum ORDER BY k.ord), chr(31))
FROM pg_constraint con
JOIN pg_class c ON c.oid = con.conrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_class rc ON rc.oid = con.confrelid
LEFT JOIN pg_namespace rn ON rn.oid = rc.relnamespace
WHERE con.contype IN ('p', 'f') AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%'
ORDER BY 1, 2, 4
"#;

/// One row per key column; grouped by constraint below. MySQL 8 returns catalog names as binary
/// strings, hence the casts.
const MYSQL: &str = r#"
SELECT CAST(k.TABLE_SCHEMA AS CHAR), CAST(k.TABLE_NAME AS CHAR), IF(k.CONSTRAINT_NAME = 'PRIMARY', 'p', 'f'),
  CAST(k.CONSTRAINT_NAME AS CHAR), CAST(k.COLUMN_NAME AS CHAR), CAST(k.REFERENCED_TABLE_SCHEMA AS CHAR),
  CAST(k.REFERENCED_TABLE_NAME AS CHAR), CAST(k.REFERENCED_COLUMN_NAME AS CHAR)
FROM information_schema.KEY_COLUMN_USAGE k
WHERE k.TABLE_SCHEMA NOT IN ('mysql', 'sys', 'information_schema', 'performance_schema')
  AND (k.CONSTRAINT_NAME = 'PRIMARY' OR k.REFERENCED_TABLE_NAME IS NOT NULL)
ORDER BY k.TABLE_SCHEMA, k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION
"#;

const SQLITE: &str = r#"
SELECT 'main', m.name, 'f', CAST(p.id AS TEXT), p."from", 'main', p."table", p."to", p.seq
FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) p WHERE m.type = 'table'
UNION ALL
SELECT 'main', m.name, 'p', 'pk', t.name, NULL, NULL, NULL, t.pk
FROM sqlite_master m JOIN pragma_table_info(m.name) t WHERE m.type = 'table' AND t.pk > 0
ORDER BY 2, 3, 4, 9
"#;

pub fn graph_sql(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => POSTGRES,
        DbKind::Mysql => MYSQL,
        DbKind::Sqlite => SQLITE,
    }
}

fn text(row: &[Cell], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

fn list(s: String) -> Vec<String> {
    s.split('\u{1f}').filter(|p| !p.is_empty()).map(str::to_string).collect()
}

pub fn from_rows(kind: DbKind, rows: &[Vec<Cell>]) -> SchemaGraph {
    let mut g = SchemaGraph::default();
    if kind == DbKind::Postgres {
        for r in rows {
            let (schema, table) = (text(r, 0), text(r, 1));
            match text(r, 2).as_str() {
                "p" => g.primary_keys.push(PrimaryKey { schema, table, columns: list(text(r, 4)) }),
                _ => g.relations.push(Relation { schema, table, columns: list(text(r, 4)), ref_schema: text(r, 5), ref_table: text(r, 6), ref_columns: list(text(r, 7)) }),
            }
        }
        return g;
    }
    // MySQL and SQLite: one row per column, consecutive rows of one constraint grouped.
    let mut i = 0;
    while i < rows.len() {
        let key = (text(&rows[i], 0), text(&rows[i], 1), text(&rows[i], 2), text(&rows[i], 3));
        let mut cols = Vec::new();
        let mut refs = Vec::new();
        let first = i;
        while i < rows.len() && (text(&rows[i], 0), text(&rows[i], 1), text(&rows[i], 2), text(&rows[i], 3)) == key {
            cols.push(text(&rows[i], 4));
            let to = text(&rows[i], 7);
            if !to.is_empty() {
                refs.push(to);
            }
            i += 1;
        }
        let (schema, table, kind, _) = key;
        if kind == "p" {
            g.primary_keys.push(PrimaryKey { schema, table, columns: cols });
        } else {
            g.relations.push(Relation { schema, table, columns: cols, ref_schema: text(&rows[first], 5), ref_table: text(&rows[first], 6), ref_columns: refs });
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<Cell> {
        cells.iter().map(|c| if c.is_empty() { None } else { Some(c.to_string()) }).collect()
    }

    #[test]
    fn groups_multi_column_keys() {
        let rows = vec![
            row(&["shop", "order_items", "f", "fk_order", "order_id", "shop", "orders", "id"]),
            row(&["shop", "order_items", "f", "fk_order", "tenant", "shop", "orders", "tenant"]),
            row(&["shop", "order_items", "p", "PRIMARY", "id", "", "", ""]),
            row(&["shop", "orders", "p", "PRIMARY", "id", "", "", ""]),
        ];
        let g = from_rows(DbKind::Mysql, &rows);
        assert_eq!(g.relations, [Relation { schema: "shop".into(), table: "order_items".into(), columns: vec!["order_id".into(), "tenant".into()], ref_schema: "shop".into(), ref_table: "orders".into(), ref_columns: vec!["id".into(), "tenant".into()] }]);
        assert_eq!(g.primary_keys.len(), 2);
    }
}
