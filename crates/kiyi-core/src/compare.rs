//! Comparing two databases (say staging and production): which tables and columns differ, rough
//! row counts, and, when both are the same kind of database, the SQL that would make the second
//! match the first. The SQL is only proposed; nothing runs from here.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::design::{self, TableDesign};
use crate::drivers::DbDriver;
use crate::error::{is_missing_table, Result};
use crate::types::{SchemaSnapshot, TableKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffStatus {
    Same,
    Different,
    OnlyLeft,
    OnlyRight,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDiff {
    pub name: String,
    /// "integer, required" on each side; None where the column is missing.
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDiff {
    pub name: String,
    pub status: DiffStatus,
    pub columns: Vec<ColumnDiff>,
    /// Planner estimates, not exact counts.
    pub left_rows: Option<i64>,
    pub right_rows: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub left_schema: Option<String>,
    pub right_schema: Option<String>,
    pub tables: Vec<TableDiff>,
    /// Statements that would make the right side's structure match the left (same database kind only).
    pub migration: Option<Vec<String>>,
    /// Why there's no migration, or anything the SQL can't cover.
    pub note: Option<String>,
}

fn describe(data_type: &str, nullable: bool) -> String {
    format!("{}{}", data_type, if nullable { "" } else { ", required" })
}

/// Tables (not views) of the snapshot's default schema, by name.
fn tables(s: &SchemaSnapshot) -> (Option<String>, BTreeMap<String, &crate::types::TableInfo>) {
    let schema = s.schemas.iter().find(|x| Some(&x.name) == s.default_schema.as_ref()).or_else(|| s.schemas.first());
    let map = schema.map(|sc| sc.tables.iter().filter(|t| matches!(t.kind, TableKind::Table)).map(|t| (t.name.clone(), t)).collect()).unwrap_or_default();
    (schema.map(|sc| sc.name.clone()), map)
}

pub async fn compare(left: &dyn DbDriver, right: &dyn DbDriver) -> Result<Comparison> {
    let (left_snapshot, right_snapshot) = (left.schema().await?, right.schema().await?);
    let (ls, lt) = tables(&left_snapshot);
    let (rs, rt) = tables(&right_snapshot);
    let mut out = Vec::new();
    let names: std::collections::BTreeSet<&String> = lt.keys().chain(rt.keys()).collect();
    for name in names {
        let (l, r) = (lt.get(name), rt.get(name));
        let mut columns = Vec::new();
        let status = match (l, r) {
            (Some(_), None) => DiffStatus::OnlyLeft,
            (None, Some(_)) => DiffStatus::OnlyRight,
            (Some(l), Some(r)) => {
                let lc: BTreeMap<_, _> = l.columns.iter().map(|c| (c.name.as_str(), describe(&c.data_type, c.nullable))).collect();
                let rc: BTreeMap<_, _> = r.columns.iter().map(|c| (c.name.as_str(), describe(&c.data_type, c.nullable))).collect();
                for col in lc.keys().chain(rc.keys()).collect::<std::collections::BTreeSet<_>>() {
                    let (a, b) = (lc.get(col).cloned(), rc.get(col).cloned());
                    if a != b {
                        columns.push(ColumnDiff { name: col.to_string(), left: a, right: b });
                    }
                }
                if columns.is_empty() { DiffStatus::Same } else { DiffStatus::Different }
            }
            (None, None) => continue,
        };
        out.push(TableDiff { name: name.clone(), status, columns, left_rows: l.and_then(|t| t.row_estimate), right_rows: r.and_then(|t| t.row_estimate) });
    }

    let (migration, note) = if left.dialect().kind != right.dialect().kind {
        (None, Some("These are different kinds of database, so Kiyi can't write SQL to align them.".to_string()))
    } else {
        let d = right.dialect();
        let schema_of = |s: &Option<String>| if d.is_sqlite() || d.is_mysql() { None } else { s.clone() };
        let (lsch, rsch) = (schema_of(&ls), schema_of(&rs));
        let mut sql = Vec::new();
        let mut problems = Vec::new();
        // Postgres enum types the right side lacks are created before the tables that use them.
        let mut right_enums: Vec<String> = if d.kind == crate::config::DbKind::Postgres {
            right.fetch("SELECT typname FROM pg_type WHERE typtype = 'e'").await?.1.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect()
        } else {
            vec![]
        };
        let mut ensure_enums = |des: &TableDesign, sql: &mut Vec<String>| {
            for c in des.columns.iter().filter(|c| !c.enum_values.is_empty() && d.kind == crate::config::DbKind::Postgres) {
                let bare = c.data_type.rsplit('.').next().unwrap_or(&c.data_type).trim_matches('"').to_string();
                if !right_enums.contains(&bare) {
                    right_enums.push(bare);
                    sql.push(format!("CREATE TYPE {} AS ENUM ({})", c.data_type, c.enum_values.iter().map(|v| d.string(v)).collect::<Vec<_>>().join(", ")));
                }
            }
        };
        for t in &out {
            match t.status {
                DiffStatus::OnlyLeft => {
                    // A table dropped since the snapshot was read just drops out of the proposal.
                    let mut des = match left.table_details(lsch.as_deref(), &t.name).await {
                        Ok(x) => x.design,
                        Err(e) if is_missing_table(&e) => continue,
                        Err(e) => return Err(e),
                    };
                    design::portable_serials(d, &mut des);
                    ensure_enums(&des, &mut sql);
                    match design::plan_create(d, rsch.as_deref(), &des) {
                        Ok(s) => sql.extend(s),
                        Err(e) => problems.push(format!("{}: {e}", t.name)),
                    }
                }
                DiffStatus::Different => {
                    let (old, left_des) = match (right.table_details(rsch.as_deref(), &t.name).await, left.table_details(lsch.as_deref(), &t.name).await) {
                        (Ok(r), Ok(l)) => (r.design, l.design),
                        (Err(e), _) | (_, Err(e)) if is_missing_table(&e) => continue,
                        (Err(e), _) | (_, Err(e)) => return Err(e),
                    };
                    let (mut old, mut left_des) = (old, left_des);
                    design::portable_serials(d, &mut old);
                    design::portable_serials(d, &mut left_des);
                    let new = aligned(&left_des, &old);
                    ensure_enums(&new, &mut sql);
                    match design::plan_alter(d, rsch.as_deref(), &old, &new) {
                        Ok(s) => sql.extend(s),
                        Err(e) => problems.push(format!("{}: {e}", t.name)),
                    }
                }
                _ => {}
            }
        }
        let only_right = out.iter().filter(|t| t.status == DiffStatus::OnlyRight).count();
        let mut notes = Vec::new();
        if only_right > 0 {
            notes.push(if only_right == 1 {
                "1 table exists only on the right; the SQL leaves it alone rather than delete anything.".to_string()
            } else {
                format!("{only_right} tables exist only on the right; the SQL leaves them alone rather than delete anything.")
            });
        }
        if !problems.is_empty() {
            notes.push(format!("Some changes need doing by hand: {}", problems.join("; ")));
        }
        (Some(sql), (!notes.is_empty()).then(|| notes.join(" ")))
    };
    Ok(Comparison { left_schema: ls, right_schema: rs, tables: out, migration, note })
}

/// The left design, with every column, key and index that also exists on the right marked as
/// that existing one, so the planner alters instead of dropping and re-adding.
fn aligned(left: &TableDesign, right: &TableDesign) -> TableDesign {
    let mut new = left.clone();
    new.primary_key_name = right.primary_key_name.clone();
    for c in &mut new.columns {
        c.original = right.columns.iter().any(|r| r.name == c.name).then(|| c.name.clone());
    }
    for f in &mut new.foreign_keys {
        f.original = right.foreign_keys.iter().any(|r| r.name == f.name).then(|| f.name.clone());
    }
    for i in &mut new.indexes {
        i.original = right.indexes.iter().any(|r| r.name == i.name).then(|| i.name.clone());
    }
    new
}
