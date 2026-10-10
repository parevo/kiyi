//! Checking and running a plan.
//!
//! The move reads every source table page by page, converts each row and writes the INSERTs to a
//! spool file; then it replays the file against the target in one transaction. Reading first
//! means a bad value stops the move before anything is written; one transaction means the target
//! ends up with all of it or none of it; the spool keeps memory flat however large the tables.
//! A test run does all of that and rolls back at the end, so the database itself vouches for
//! every row (types, constraints, foreign keys) without keeping any of them.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde::Serialize;

use super::convert::{apply_steps, convert, literal, TargetColumn};
use super::{table_key, ColumnMapping, IdMode, Issue, MigrationPlan, Severity, TableMapping, ValueSource, WriteMode};
use crate::catalog::TypeCategory;
use crate::config::DbKind;
use crate::design::TableDetails;
use crate::dialect::Dialect;
use crate::drivers::DbDriver;
use crate::error::{Error, ErrorInfo, Result};
use crate::types::{Cell, ColumnMeta, ValueKind};

const PAGE: u32 = 2_000;
const ROWS_PER_STATEMENT: usize = 200;
const BYTES_PER_STATEMENT: usize = 2 * 1024 * 1024;
const SAMPLE: u32 = 200;
const PREVIEW: usize = 12;
/// Ends a test run's transaction on purpose; never shown to anyone.
const TEST_RUN_DONE: &str = "\u{1}kiyi-test-run-done";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Preparing,
    Reading,
    Writing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: Stage,
    pub table: String,
    /// Rows done and expected across the whole move, for one progress bar.
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableCheck {
    pub source: String,
    pub target: String,
    pub rows: u64,
    /// The target columns written, then a few converted rows as they'd be stored.
    pub columns: Vec<String>,
    pub preview: Vec<Vec<Cell>>,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationCheck {
    pub tables: Vec<TableCheck>,
    /// Problems with the plan as a whole, and every table's problems again for a summary.
    pub issues: Vec<Issue>,
    /// Target tables in the order they'll be filled.
    pub order: Vec<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableResult {
    pub target: String,
    pub rows: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub tables: Vec<TableResult>,
    pub rows: u64,
    /// Everything was written and then rolled back.
    pub test_run: bool,
    pub seconds: f64,
}

/// One table of the plan, with everything looked up that checking and running need.
struct Prepared {
    mapping: TableMapping,
    target: TableDetails,
    /// Target columns written, in statement order, each with its mapping (None for the renumbered key).
    writes: Vec<(TargetColumn, Option<ColumnMapping>)>,
    /// Source columns read, in SELECT order.
    reads: Vec<String>,
    source_key: Option<String>,
    target_key: Option<TargetColumn>,
    /// Explicit values go into an identity / auto-increment key.
    explicit_identity: bool,
}

impl Prepared {
    fn name(&self) -> String {
        self.mapping.target_table.clone()
    }
    fn renumbered(&self) -> bool {
        self.mapping.ids == IdMode::Renumber
    }
}

fn single_key(d: &TableDetails) -> Option<&crate::design::ColumnDesign> {
    let keys: Vec<_> = d.design.columns.iter().filter(|c| c.primary_key).collect();
    (keys.len() == 1).then(|| keys[0])
}

/// Looks the plan's tables up and finds everything wrong with it that can be known without reading rows.
async fn prepare(source: &dyn DbDriver, target: &dyn DbDriver, plan: &MigrationPlan) -> (Vec<Prepared>, Vec<Issue>) {
    let t_kind = target.dialect().kind;
    let mut issues = Vec::new();
    let mut out = Vec::new();
    let enabled: Vec<&TableMapping> = plan.tables.iter().filter(|m| m.enabled).collect();
    if enabled.is_empty() {
        issues.push(Issue::error("", None, "Choose at least one table to move."));
    }
    let mut seen_targets = HashSet::new();
    // Every table's details first, so links can be checked against the real keys.
    let mut looked_up: Vec<(&TableMapping, Option<TableDetails>, Option<TableDetails>)> = Vec::new();
    for m in &enabled {
        let name = m.target_table.as_str();
        if !seen_targets.insert(table_key(m.target_schema.as_deref(), &m.target_table)) {
            issues.push(Issue::error(name, None, format!("{name} is filled twice; turn one of them off.")));
        }
        let src = match source.table_details(m.source_schema.as_deref(), &m.source_table).await {
            Ok(d) => Some(d),
            Err(e) => {
                issues.push(Issue::error(name, None, format!("Can't read {} from the source: {e}", m.source_table)));
                None
            }
        };
        let dst = match target.table_details(m.target_schema.as_deref(), &m.target_table).await {
            Ok(d) => Some(d),
            Err(e) => {
                issues.push(Issue::error(name, None, format!("Can't find {name} in the target: {e}")));
                None
            }
        };
        looked_up.push((m, src, dst));
    }
    // Source tables whose rows can be linked to: renumbered, or with their key copied over.
    let linkable: HashMap<String, Option<String>> = looked_up
        .iter()
        .map(|(m, _, dst)| {
            let key = dst.as_ref().and_then(single_key).map(|k| k.name.clone());
            let ok = m.ids == IdMode::Renumber || key.as_ref().is_some_and(|k| m.columns.iter().any(|c| &c.target == k && !matches!(c.source, ValueSource::Default)));
            (m.source_key(), (!ok).then(|| m.target_table.clone()))
        })
        .collect();
    for (m, src, dst) in looked_up {
        let name = m.target_table.as_str();
        let (Some(src), Some(dst)) = (src, dst) else { continue };
        if dst.is_view {
            issues.push(Issue::error(name, None, format!("{name} is a view; move data into tables.")));
            continue;
        }
        let source_cols: HashSet<&str> = src.design.columns.iter().map(|c| c.name.as_str()).collect();
        let target_key = single_key(&dst).map(|k| TargetColumn::new(t_kind, k));
        let source_key = single_key(&src).map(|k| k.name.clone());
        let mut writes: Vec<(TargetColumn, Option<ColumnMapping>)> = Vec::new();
        let mut reads: Vec<String> = source_key.iter().cloned().collect();
        let mut read = |c: &str| {
            if !reads.iter().any(|r| r == c) {
                reads.push(c.to_string());
            }
        };

        if m.ids == IdMode::Renumber {
            match &target_key {
                Some(k) if k.category == TypeCategory::Number => writes.push((k.clone(), None)),
                _ => issues.push(Issue::error(name, None, format!("{name} doesn't have a single whole-number key, so it can't be given new numbers. Keep its IDs instead."))),
            }
        }
        for cm in &m.columns {
            let Some(tc) = dst.design.columns.iter().find(|c| c.name == cm.target) else {
                issues.push(Issue::error(name, Some(&cm.target), format!("{name} has no column {}.", cm.target)));
                continue;
            };
            if m.ids == IdMode::Renumber && target_key.as_ref().is_some_and(|k| k.name == tc.name) {
                continue;
            }
            let missing = |c: &str| (!source_cols.contains(c)).then(|| format!("{} has no column {c}.", m.source_table));
            match &cm.source {
                ValueSource::Default => continue,
                ValueSource::Fixed { .. } => {}
                ValueSource::Column { column } => match missing(column) {
                    Some(e) => issues.push(Issue::error(name, Some(&tc.name), e)),
                    None => read(column),
                },
                ValueSource::Combine { columns, .. } => {
                    for c in columns {
                        match missing(c) {
                            Some(e) => issues.push(Issue::error(name, Some(&tc.name), e)),
                            None => read(c),
                        }
                    }
                }
                ValueSource::Reference { column, schema, table } => {
                    if let Some(e) = missing(column) {
                        issues.push(Issue::error(name, Some(&tc.name), e));
                    } else {
                        read(column);
                    }
                    match linkable.get(&table_key(schema.as_deref(), table)) {
                        None => issues.push(Issue::error(name, Some(&tc.name), format!("{} links to {table}, which isn't being moved. Move {table} too, or map {} another way.", tc.name, tc.name))),
                        Some(Some(other)) => issues.push(Issue::error(
                            name,
                            Some(&tc.name),
                            format!("{other} gets its IDs from the database, so links to it can't be followed. Give {other} new numbers, or map its key."),
                        )),
                        Some(None) => {}
                    }
                }
            }
            if tc.generated {
                issues.push(Issue::error(name, Some(&tc.name), format!("{} is computed by the database and can't be written.", tc.name)));
                continue;
            }
            writes.push((TargetColumn::new(t_kind, tc), Some(cm.clone())));
        }
        for tc in &dst.design.columns {
            let tcol = TargetColumn::new(t_kind, tc);
            if tcol.required() && !writes.iter().any(|(w, _)| w.name == tc.name) {
                issues.push(Issue::error(name, Some(&tc.name), format!("{} is required but gets no value. Map it, or give it a fixed value.", tc.name)));
            }
        }
        if matches!(m.write, WriteMode::Skip | WriteMode::Update) {
            let wanted: HashSet<&str> = m.match_on.iter().map(String::as_str).collect();
            let pk: HashSet<&str> = dst.design.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
            let unique = !wanted.is_empty() && (wanted == pk || dst.design.indexes.iter().any(|i| i.unique && i.columns.iter().map(String::as_str).collect::<HashSet<_>>() == wanted));
            if m.match_on.is_empty() {
                issues.push(Issue::error(name, None, "Choose the columns that identify an existing row."));
            } else if !unique {
                issues.push(Issue::error(name, None, format!("{} must be the key or have a unique index to find existing rows.", m.match_on.join(" + "))));
            } else if m.ids == IdMode::Renumber && wanted == pk {
                issues.push(Issue::error(name, None, "New numbers never match existing rows; match on another unique column, or keep the IDs."));
            }
            for c in &m.match_on {
                if !writes.iter().any(|(w, _)| &w.name == c) {
                    issues.push(Issue::error(name, Some(c), format!("{c} identifies existing rows, so it needs a value.")));
                }
            }
            if t_kind == DbKind::Sqlite && m.write == WriteMode::Update && writes.len() == m.match_on.len() {
                issues.push(Issue::warning(name, None, "Only the matching columns are mapped, so there's nothing to update."));
            }
        }
        let explicit_identity = target_key.as_ref().is_some_and(|k| dst.design.columns.iter().any(|c| c.name == k.name && c.auto_increment) && writes.iter().any(|(w, _)| w.name == k.name));
        // A copied link into a renumbered table would point at the wrong rows.
        for fk in &dst.design.foreign_keys {
            if fk.columns.len() != 1 {
                continue;
            }
            let renumbered = enabled.iter().any(|o| o.target_table == fk.ref_table && o.ids == IdMode::Renumber);
            let copied = m.columns.iter().any(|c| c.target == fk.columns[0] && matches!(c.source, ValueSource::Column { .. }));
            if renumbered && copied {
                issues.push(Issue::warning(name, Some(&fk.columns[0]), format!("{} links to {}, which gets new numbers; choose “New ID of a moved row” so links follow.", fk.columns[0], fk.ref_table)));
            }
        }
        out.push(Prepared { mapping: m.clone(), target: dst, writes, reads, source_key, target_key, explicit_identity });
    }
    order(&mut out, &mut issues);
    (out, issues)
}

/// Referenced tables first, so their rows exist (and their new numbers are known) when links arrive.
fn order(tables: &mut Vec<Prepared>, issues: &mut Vec<Issue>) {
    let deps: Vec<HashSet<String>> = tables
        .iter()
        .map(|p| {
            let mut d: HashSet<String> = p
                .mapping
                .columns
                .iter()
                .filter_map(|c| match &c.source {
                    ValueSource::Reference { schema, table, .. } => tables.iter().find(|o| o.mapping.source_key() == table_key(schema.as_deref(), table)).map(|o| o.name()),
                    _ => None,
                })
                .collect();
            for fk in &p.target.design.foreign_keys {
                if tables.iter().any(|o| o.mapping.target_table == fk.ref_table) {
                    d.insert(fk.ref_table.clone());
                }
            }
            d.remove(&p.name());
            d
        })
        .collect();
    let mut done: Vec<String> = Vec::new();
    let mut sorted: Vec<usize> = Vec::new();
    while sorted.len() < tables.len() {
        let next = (0..tables.len()).find(|i| !sorted.contains(i) && deps[*i].iter().all(|d| done.contains(d)));
        match next {
            Some(i) => {
                sorted.push(i);
                done.push(tables[i].name());
            }
            None => {
                // A cycle: fill the rest in plan order and say so.
                let rest: Vec<usize> = (0..tables.len()).filter(|i| !sorted.contains(i)).collect();
                issues.push(Issue::warning("", None, format!("{} link to each other in a circle; if the move fails on a link, move them in two passes.", rest.iter().map(|i| tables[*i].name()).collect::<Vec<_>>().join(", "))));
                sorted.extend(rest);
            }
        }
    }
    let mut slots: Vec<Option<Prepared>> = std::mem::take(tables).into_iter().map(Some).collect();
    *tables = sorted.into_iter().map(|i| slots[i].take().unwrap()).collect();
}

/// For each source table: the target key its rows get, by source key.
enum KeyMap {
    /// Links point at the same key values in the target.
    Same,
    Renumbered(HashMap<String, i64>),
}

/// Pages through a source table, by key when it has one (fast at any depth) and by offset otherwise.
struct Pager<'a> {
    driver: &'a dyn DbDriver,
    sql_head: String,
    order: String,
    key_index: Option<usize>,
    after: Option<String>,
    offset: u64,
    done: bool,
}

impl<'a> Pager<'a> {
    fn new(driver: &'a dyn DbDriver, p: &Prepared, columns: &[String], key: Option<&str>) -> Self {
        let d = driver.dialect();
        let table = d.table(p.mapping.source_schema.as_deref(), &p.mapping.source_table);
        let order = match key {
            Some(k) => d.ident(k),
            None => d.ident_list(columns),
        };
        Self { driver, sql_head: format!("SELECT {} FROM {table}", d.ident_list(columns)), order, key_index: key.and_then(|k| columns.iter().position(|c| c == k)), after: None, offset: 0, done: false }
    }

    async fn next(&mut self, limit: u32) -> Result<Option<(Vec<ColumnMeta>, Vec<Vec<Cell>>)>> {
        if self.done {
            return Ok(None);
        }
        let d = self.driver.dialect();
        let mut sql = self.sql_head.clone();
        if let (Some(_), Some(after)) = (self.key_index, &self.after) {
            sql.push_str(&format!(" WHERE {} > {}", self.order, d.string(after)));
        }
        sql.push_str(&format!(" ORDER BY {}", self.order));
        let offset = if self.key_index.is_some() { 0 } else { self.offset };
        if d.is_sqlserver() {
            sql.push_str(&format!(" OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"));
        } else {
            sql.push_str(&format!(" LIMIT {limit}"));
            if offset > 0 {
                sql.push_str(&format!(" OFFSET {offset}"));
            }
        }
        let (columns, rows) = self.driver.fetch(&sql).await?;
        if (rows.len() as u32) < limit {
            self.done = true;
        }
        if rows.is_empty() {
            return Ok(None);
        }
        if let Some(i) = self.key_index {
            self.after = rows.last().and_then(|r| r[i].clone());
            if self.after.is_none() {
                // A NULL key can't be paged past; fall back to the end.
                self.done = true;
            }
        }
        self.offset += rows.len() as u64;
        Ok(Some((columns, rows)))
    }
}

async fn count(driver: &dyn DbDriver, schema: Option<&str>, table: &str) -> Result<u64> {
    let d = driver.dialect();
    let (_, rows) = driver.fetch(&format!("SELECT COUNT(*) FROM {}", d.table(schema, table))).await?;
    Ok(rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0))
}

async fn max_key(driver: &dyn DbDriver, p: &Prepared, key: &str) -> Result<i64> {
    let d = driver.dialect();
    let (_, rows) = driver.fetch(&format!("SELECT MAX({}) FROM {}", d.ident(key), d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table))).await?;
    Ok(rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.trim().parse().ok()).unwrap_or(0))
}

/// Matching is forgiving about case and spaces, like most databases' unique checks.
fn match_text(v: &Option<String>) -> String {
    v.as_deref().map(|s| s.trim().to_lowercase()).unwrap_or_default()
}

/// New numbers for renumbered tables, after the highest key already there. Rows that will be
/// skipped or updated because they already exist (by `match_on`) keep the existing row's key, so
/// links to them land on the right row.
async fn key_maps(source: &dyn DbDriver, target: &dyn DbDriver, tables: &[Prepared], cancel: &AtomicBool) -> Result<HashMap<String, KeyMap>> {
    let d = target.dialect();
    let mut maps = HashMap::new();
    for p in tables {
        if !p.renumbered() {
            maps.insert(p.mapping.source_key(), KeyMap::Same);
            continue;
        }
        let (Some(tk), Some(sk)) = (&p.target_key, &p.source_key) else {
            maps.insert(p.mapping.source_key(), KeyMap::Renumbered(HashMap::new()));
            continue;
        };
        let mut next = max_key(target, p, &tk.name).await? + 1;
        let mut map = HashMap::new();
        let matching = p.mapping.write != WriteMode::Insert && !p.mapping.match_on.is_empty();
        if !matching {
            let mut pager = Pager::new(source, p, std::slice::from_ref(sk), Some(sk));
            while let Some((_, rows)) = pager.next(PAGE * 5).await? {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                for r in rows {
                    if let Some(k) = r.into_iter().next().flatten() {
                        map.insert(k.trim().to_string(), next);
                        next += 1;
                    }
                }
            }
            maps.insert(p.mapping.source_key(), KeyMap::Renumbered(map));
            continue;
        }
        // Existing rows by their match columns.
        let sql = format!(
            "SELECT {}, {} FROM {}",
            d.ident_list(&p.mapping.match_on),
            d.ident(&tk.name),
            d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table)
        );
        let (_, existing_rows) = target.fetch(&sql).await?;
        let existing: HashMap<Vec<String>, i64> = existing_rows
            .into_iter()
            .filter_map(|r| {
                let key = r.last().cloned().flatten()?.trim().parse().ok()?;
                Some((r[..r.len() - 1].iter().map(match_text).collect(), key))
            })
            .collect();
        let positions: Vec<usize> = p.mapping.match_on.iter().filter_map(|m| p.writes.iter().position(|(c, _)| &c.name == m)).collect();
        let index: HashMap<&str, usize> = p.reads.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
        let key_at = index[sk.as_str()];
        let mut pager = Pager::new(source, p, &p.reads, Some(sk));
        let mut scratch = 0i64;
        while let Some((meta, rows)) = pager.next(PAGE).await? {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let kinds: Vec<ValueKind> = meta.iter().map(|c| c.kind).collect();
            for row in &rows {
                let Some(source_key) = row[key_at].clone() else { continue };
                // A row that won't convert gets a new number; the move reports it later.
                let found = convert_row(d.kind, p, row, &index, &kinds, &maps, &mut scratch)
                    .ok()
                    .and_then(|values| existing.get(&positions.iter().map(|i| match_text(&values[*i])).collect::<Vec<_>>()).copied());
                let key = found.unwrap_or_else(|| {
                    next += 1;
                    next - 1
                });
                map.insert(source_key.trim().to_string(), key);
            }
        }
        maps.insert(p.mapping.source_key(), KeyMap::Renumbered(map));
    }
    Ok(maps)
}

/// Converts one source row into the target's values (None = NULL), or says which column failed.
fn convert_row(
    t_kind: DbKind,
    p: &Prepared,
    row: &[Cell],
    index: &HashMap<&str, usize>,
    kinds: &[ValueKind],
    maps: &HashMap<String, KeyMap>,
    counter: &mut i64,
) -> std::result::Result<Vec<Option<String>>, (String, String)> {
    let get = |c: &str| index.get(c).and_then(|i| row.get(*i).cloned().flatten());
    let kind_of = |c: &str| index.get(c).and_then(|i| kinds.get(*i).copied());
    let mut out = Vec::with_capacity(p.writes.len());
    for (col, mapping) in &p.writes {
        let value = match mapping {
            None => {
                // The renumbered key: the number reserved for this row, or the next one.
                let reserved = p.source_key.as_deref().and_then(get).and_then(|k| match maps.get(&p.mapping.source_key()) {
                    Some(KeyMap::Renumbered(m)) => m.get(k.trim()).copied(),
                    _ => None,
                });
                let n = reserved.unwrap_or_else(|| {
                    *counter += 1;
                    *counter
                });
                Some(n.to_string())
            }
            Some(m) => {
                let (raw, kind) = match &m.source {
                    ValueSource::Column { column } => (get(column), kind_of(column)),
                    ValueSource::Combine { columns, separator } => {
                        let parts: Vec<String> = columns.iter().filter_map(|c| get(c)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect();
                        ((!parts.is_empty()).then(|| parts.join(separator)), None)
                    }
                    ValueSource::Fixed { value } => (value.clone(), None),
                    ValueSource::Reference { column, schema, table } => {
                        let raw = get(column);
                        let resolved = match (raw, maps.get(&table_key(schema.as_deref(), table))) {
                            (None, _) => None,
                            (Some(v), Some(KeyMap::Renumbered(map))) => match map.get(v.trim()) {
                                Some(n) => Some(n.to_string()),
                                None => return Err((col.name.clone(), format!("links to {table} “{}”, which isn't among the moved rows", v.trim()))),
                            },
                            (Some(v), _) => Some(v),
                        };
                        (resolved, None)
                    }
                    ValueSource::Default => continue,
                };
                let stepped = apply_steps(raw, &m.steps);
                convert(t_kind, col, stepped, kind).map_err(|e| (col.name.clone(), e))?
            }
        };
        out.push(value);
    }
    Ok(out)
}

/// The SQL for a missing value: NULL, or the column's default when it can't be NULL.
fn missing_literal(d: Dialect, col: &TargetColumn, default: Option<&str>, in_merge: bool) -> String {
    if col.nullable {
        return "NULL".into();
    }
    match (d.kind, default) {
        (DbKind::Postgres | DbKind::Mysql, _) => "DEFAULT".into(),
        (DbKind::Sqlserver, _) if !in_merge => "DEFAULT".into(),
        (_, Some(expr)) => expr.to_string(),
        _ => "NULL".into(),
    }
}

/// Multi-row INSERT (or MERGE on SQL Server) for one batch, following the table's write mode.
fn statement(d: Dialect, p: &Prepared, rows: &[Vec<Option<String>>], pg_identity: bool) -> String {
    let m = &p.mapping;
    let table = d.table(m.target_schema.as_deref(), &m.target_table);
    let names: Vec<String> = p.writes.iter().map(|(c, _)| c.name.clone()).collect();
    let cols = d.ident_list(&names);
    let merge = d.is_sqlserver() && m.write != WriteMode::Insert;
    let defaults: Vec<Option<&str>> = p.writes.iter().map(|(c, _)| p.target.design.columns.iter().find(|x| x.name == c.name).and_then(|x| x.default.as_deref())).collect();
    let values: Vec<String> = rows
        .iter()
        .map(|r| {
            let cells: Vec<String> = r.iter().zip(&p.writes).zip(&defaults).map(|((v, (col, _)), def)| if v.is_none() { missing_literal(d, col, *def, merge) } else { literal(d, col, v.as_deref()) }).collect();
            format!("({})", cells.join(", "))
        })
        .collect();
    let values = values.join(",\n");
    // An update never touches the key: other rows may already point at it.
    let rest: Vec<&String> = names.iter().filter(|n| !m.match_on.contains(n) && !p.target.design.columns.iter().any(|c| &c.name == *n && c.primary_key)).collect();
    let keys = d.ident_list(&m.match_on);
    if merge {
        let on: Vec<String> = m.match_on.iter().map(|c| format!("t.{0} = s.{0}", d.ident(c))).collect();
        let insert_vals: Vec<String> = names.iter().map(|c| format!("s.{}", d.ident(c))).collect();
        let mut sql = format!("MERGE INTO {table} AS t USING (VALUES {values}) AS s ({cols}) ON {} WHEN NOT MATCHED THEN INSERT ({cols}) VALUES ({})", on.join(" AND "), insert_vals.join(", "));
        if m.write == WriteMode::Update && !rest.is_empty() {
            let set: Vec<String> = rest.iter().map(|c| format!("t.{0} = s.{0}", d.ident(c))).collect();
            sql.push_str(&format!(" WHEN MATCHED THEN UPDATE SET {}", set.join(", ")));
        }
        return sql;
    }
    let overriding = if pg_identity { " OVERRIDING SYSTEM VALUE" } else { "" };
    let mut sql = format!("INSERT INTO {table} ({cols}){overriding} VALUES {values}");
    match (m.write, d.kind) {
        (WriteMode::Insert, _) => {}
        (WriteMode::Skip, DbKind::Mysql) => sql.push_str(&format!(" ON DUPLICATE KEY UPDATE {0} = {0}", d.ident(&m.match_on[0]))),
        (WriteMode::Update, DbKind::Mysql) if !rest.is_empty() => {
            let set: Vec<String> = rest.iter().map(|c| format!("{0} = VALUES({0})", d.ident(c))).collect();
            sql.push_str(&format!(" ON DUPLICATE KEY UPDATE {}", set.join(", ")));
        }
        (WriteMode::Update, DbKind::Mysql) => sql.push_str(&format!(" ON DUPLICATE KEY UPDATE {0} = {0}", d.ident(&m.match_on[0]))),
        (WriteMode::Update, _) if !rest.is_empty() => {
            let set: Vec<String> = rest.iter().map(|c| format!("{0} = EXCLUDED.{0}", d.ident(c))).collect();
            sql.push_str(&format!(" ON CONFLICT ({keys}) DO UPDATE SET {}", set.join(", ")));
        }
        _ => sql.push_str(&format!(" ON CONFLICT ({keys}) DO NOTHING")),
    }
    sql
}

/// Postgres identity columns refuse explicit values unless asked.
async fn pg_identity(target: &dyn DbDriver, p: &Prepared) -> bool {
    let d = target.dialect();
    if d.kind != DbKind::Postgres || !p.explicit_identity {
        return false;
    }
    let Some(k) = &p.target_key else { return false };
    let reg = d.string(&d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table));
    let sql = format!("SELECT a.attidentity::text FROM pg_attribute a WHERE a.attrelid = {reg}::regclass AND a.attname = {}", d.string(&k.name));
    target.fetch(&sql).await.ok().and_then(|(_, rows)| rows.first().and_then(|r| r[0].clone())).is_some_and(|v| !v.is_empty())
}

fn display(v: &Option<String>) -> Cell {
    v.clone()
}

/// Reads a sample of every table and reports what the move would do and what would go wrong.
pub async fn check(source: &dyn DbDriver, target: &dyn DbDriver, plan: &MigrationPlan) -> Result<MigrationCheck> {
    let cancel = AtomicBool::new(false);
    let (tables, mut issues) = prepare(source, target, plan).await;
    let maps = key_maps(source, target, &tables, &cancel).await?;
    let t_kind = target.dialect().kind;
    let mut checks = Vec::new();
    for p in &tables {
        let mut table_issues: Vec<Issue> = issues.iter().filter(|i| i.table == p.name()).cloned().collect();
        let rows = count(source, p.mapping.source_schema.as_deref(), &p.mapping.source_table).await?;
        let mut pager = Pager::new(source, p, &p.reads, p.source_key.as_deref());
        let mut preview = Vec::new();
        // column → (rows that failed, first example)
        let mut failures: HashMap<String, (u32, String)> = HashMap::new();
        let mut missing_required: HashMap<String, u32> = HashMap::new();
        let mut sampled = 0u32;
        if let Some((meta, sample)) = pager.next(SAMPLE).await? {
            let index: HashMap<&str, usize> = p.reads.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
            let kinds: Vec<ValueKind> = meta.iter().map(|c| c.kind).collect();
            let mut counter = match &p.target_key {
                Some(k) if p.renumbered() => max_key(target, p, &k.name).await.unwrap_or(0),
                _ => 0,
            };
            if let Some(KeyMap::Renumbered(m)) = maps.get(&p.mapping.source_key()) {
                counter = counter.max(m.values().copied().max().unwrap_or(0));
            }
            for row in &sample {
                sampled += 1;
                match convert_row(t_kind, p, row, &index, &kinds, &maps, &mut counter) {
                    Ok(values) => {
                        for ((col, _), v) in p.writes.iter().zip(&values) {
                            if v.is_none() && col.required() {
                                *missing_required.entry(col.name.clone()).or_default() += 1;
                            }
                        }
                        if preview.len() < PREVIEW {
                            preview.push(values.iter().map(display).collect());
                        }
                    }
                    Err((column, message)) => {
                        let e = failures.entry(column).or_insert((0, message));
                        e.0 += 1;
                    }
                }
            }
        }
        let of = |n: u32| if n == sampled { format!("all {sampled} rows checked") } else { format!("{n} of {sampled} rows checked") };
        for (column, (n, example)) in failures {
            table_issues.push(Issue::error(&p.name(), Some(&column), format!("{column}: {}; e.g. {example}.", of(n))));
        }
        for (column, n) in missing_required {
            table_issues.push(Issue::error(&p.name(), Some(&column), format!("{column} is required but is empty in {}. Add an “If empty” step or map it differently.", of(n))));
        }
        if p.mapping.write == WriteMode::Insert {
            let existing = count(target, p.mapping.target_schema.as_deref(), &p.mapping.target_table).await.unwrap_or(0);
            if existing > 0 && !p.renumbered() && p.writes.iter().any(|(c, _)| p.target_key.as_ref().is_some_and(|k| k.name == c.name)) {
                table_issues.push(Issue::warning(
                    &p.name(),
                    None,
                    format!("{} already has {existing} rows; a moved row with the same key stops the move. Give it new numbers, or skip or update existing rows.", p.name()),
                ));
            }
        }
        table_issues.sort_by_key(|i| (i.severity != Severity::Error, i.column.clone()));
        for i in &table_issues {
            if !issues.contains(i) {
                issues.push(i.clone());
            }
        }
        checks.push(TableCheck {
            source: p.mapping.source_table.clone(),
            target: p.name(),
            rows,
            columns: p.writes.iter().map(|(c, _)| c.name.clone()).collect(),
            preview,
            issues: table_issues,
        });
    }
    let ready = !issues.iter().any(|i| i.severity == Severity::Error);
    Ok(MigrationCheck { order: tables.iter().map(Prepared::name).collect(), tables: checks, issues, ready })
}

/// Removes the spool file however the move ends.
struct Spool(PathBuf);

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Which table and rows a spooled statement holds, for progress and error messages.
struct Batch {
    table: usize,
    first_row: u64,
    rows: u64,
}

fn spool_batch(file: &mut BufWriter<std::fs::File>, batches: &mut Vec<Batch>, written: &mut [u64], table: usize, sql: String, pending: &mut Vec<Vec<Option<String>>>, row_no: u64) -> Result<()> {
    let n = pending.len() as u64;
    writeln!(file, "{}", serde_json::to_string(&sql)?)?;
    batches.push(Batch { table, first_row: row_no - n + 1, rows: n });
    written[table] += n;
    pending.clear();
    Ok(())
}

/// Moves the data. With `test_run`, everything is written and then rolled back.
pub async fn run(
    source: &dyn DbDriver,
    target: &dyn DbDriver,
    plan: &MigrationPlan,
    test_run: bool,
    spool_dir: &Path,
    progress: &(dyn Fn(Progress) + Send + Sync),
    cancel: &AtomicBool,
) -> Result<MigrationReport> {
    let started = Instant::now();
    let d = target.dialect();
    let (tables, issues) = prepare(source, target, plan).await;
    let errors: Vec<String> = issues.iter().filter(|i| i.severity == Severity::Error).map(|i| if i.table.is_empty() { i.message.clone() } else { format!("{}: {}", i.table, i.message) }).collect();
    if !errors.is_empty() {
        return Err(Error::Invalid(format!("The plan needs fixing first: {}", errors.join(" "))));
    }
    let mut totals = Vec::new();
    for p in &tables {
        totals.push(count(source, p.mapping.source_schema.as_deref(), &p.mapping.source_table).await?);
    }
    let total: u64 = totals.iter().sum();
    progress(Progress { stage: Stage::Preparing, table: String::new(), done: 0, total });
    let maps = key_maps(source, target, &tables, cancel).await?;

    // Read and convert everything into the spool before touching the target.
    std::fs::create_dir_all(spool_dir)?;
    let spool = Spool(spool_dir.join(format!("kiyi-move-{}.sql.jsonl", uuid::Uuid::new_v4())));
    let mut file = BufWriter::new(std::fs::File::create(&spool.0)?);
    let mut batches: Vec<Batch> = Vec::new();
    let mut read_so_far = 0u64;
    let mut written_per_table = vec![0u64; tables.len()];
    for (ti, p) in tables.iter().enumerate() {
        let identity_on = d.is_sqlserver() && p.explicit_identity;
        let qualified = d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table);
        if identity_on {
            writeln!(file, "{}", serde_json::to_string(&format!("SET IDENTITY_INSERT {qualified} ON"))?)?;
            batches.push(Batch { table: ti, first_row: 0, rows: 0 });
        }
        let overriding = pg_identity(target, p).await;
        let index: HashMap<&str, usize> = p.reads.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
        let mut counter = match &p.target_key {
            Some(k) if p.renumbered() => max_key(target, p, &k.name).await?,
            _ => 0,
        };
        // Rows without a source key are numbered after the reserved ones.
        if let Some(KeyMap::Renumbered(m)) = maps.get(&p.mapping.source_key()) {
            counter = counter.max(m.values().copied().max().unwrap_or(0));
        }
        let mut pager = Pager::new(source, p, &p.reads, p.source_key.as_deref());
        let mut pending: Vec<Vec<Option<String>>> = Vec::new();
        let mut pending_bytes = 0usize;
        let mut row_no = 0u64;
        while let Some((meta, rows)) = pager.next(PAGE).await? {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let kinds: Vec<ValueKind> = meta.iter().map(|c| c.kind).collect();
            for row in &rows {
                row_no += 1;
                let values = convert_row(d.kind, p, row, &index, &kinds, &maps, &mut counter).map_err(|(column, message)| {
                    let which = p.source_key.as_deref().and_then(|k| index.get(k)).and_then(|i| row[*i].clone()).map(|k| format!("the row with key {k}")).unwrap_or_else(|| format!("row {row_no}"));
                    Error::Invalid(format!("Nothing was moved. {} → {}, {which}: {column} {message}.", p.mapping.source_table, p.name()))
                })?;
                pending_bytes += values.iter().map(|v| v.as_ref().map_or(4, String::len) + 4).sum::<usize>();
                pending.push(values);
                if pending.len() >= ROWS_PER_STATEMENT || pending_bytes >= BYTES_PER_STATEMENT {
                    spool_batch(&mut file, &mut batches, &mut written_per_table, ti, statement(d, p, &pending, overriding), &mut pending, row_no)?;
                    pending_bytes = 0;
                }
            }
            read_so_far += rows.len() as u64;
            progress(Progress { stage: Stage::Reading, table: p.name(), done: read_so_far, total });
        }
        if !pending.is_empty() {
            spool_batch(&mut file, &mut batches, &mut written_per_table, ti, statement(d, p, &pending, overriding), &mut pending, row_no)?;
        }
        if identity_on {
            writeln!(file, "{}", serde_json::to_string(&format!("SET IDENTITY_INSERT {qualified} OFF"))?)?;
            batches.push(Batch { table: ti, first_row: 0, rows: 0 });
        }
    }
    file.flush()?;
    drop(file);

    // Replay the spool in one transaction.
    let mut lines = BufReader::new(std::fs::File::open(&spool.0)?).lines();
    let mut index = 0usize;
    let mut done = 0u64;
    let mut finished = false;
    let mut next = || -> Option<Result<String>> {
        if cancel.load(Ordering::Relaxed) {
            return Some(Err(Error::Cancelled));
        }
        if index > 0 {
            if let Some(b) = batches.get(index - 1) {
                done += b.rows;
                progress(Progress { stage: Stage::Writing, table: tables[b.table].name(), done, total });
            }
        }
        match lines.next() {
            Some(Ok(line)) => {
                index += 1;
                Some(serde_json::from_str::<String>(&line).map_err(Error::from))
            }
            Some(Err(e)) => Some(Err(e.into())),
            None if test_run && !finished => {
                finished = true;
                Some(Err(Error::Invalid(TEST_RUN_DONE.into())))
            }
            None => None,
        }
    };
    match target.execute_stream(&mut next).await {
        Ok(_) => {}
        Err(Error::Invalid(m)) if m == TEST_RUN_DONE => {}
        Err(Error::Cancelled) => return Err(Error::Invalid("Stopped. Nothing was moved.".into())),
        Err(Error::Script { index, source }) => {
            let where_ = batches.get(index).map(|b| {
                let t = &tables[b.table];
                if b.rows == 0 { t.name() } else { format!("{} → {}, rows {}–{}", t.mapping.source_table, t.name(), b.first_row, b.first_row + b.rows - 1) }
            });
            return Err(Error::Invalid(format!("Nothing was moved. {}: {}", where_.unwrap_or_default(), ErrorInfo::from(&*source).message)));
        }
        Err(e) => return Err(Error::Invalid(format!("Nothing was moved. {}", ErrorInfo::from(&e).message))),
    }

    // Postgres sequences don't follow explicit keys; move them past the new rows.
    if !test_run && d.kind == DbKind::Postgres {
        for p in tables.iter().filter(|p| p.explicit_identity) {
            let Some(k) = &p.target_key else { continue };
            let qualified = d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table);
            let sql = format!("SELECT setval(pg_get_serial_sequence({}, {}), COALESCE((SELECT MAX({}) FROM {qualified}), 1))", d.string(&qualified), d.string(&k.name), d.ident(&k.name));
            if let Err(e) = target.fetch(&sql).await {
                tracing::warn!("couldn't move the sequence of {qualified}: {e}");
            }
        }
    }

    Ok(MigrationReport {
        tables: tables.iter().zip(&written_per_table).map(|(p, n)| TableResult { target: p.name(), rows: *n }).collect(),
        rows: written_per_table.iter().sum(),
        test_run,
        seconds: started.elapsed().as_secs_f64(),
    })
}
