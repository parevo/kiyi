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
    /// Source rows left out because they repeat an earlier row's `match_on` values.
    pub repeated: u64,
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
    /// Columns in `writes` that link rows of this same table (a manager, a parent category).
    /// They're written empty first and filled once every row exists, so the order of the
    /// source rows doesn't matter.
    deferred: Vec<usize>,
    /// Where the target key is in `writes`, when Kiyi writes it.
    key_index: Option<usize>,
    /// Source columns that page through the table: its key, or a unique index without empty values.
    page_key: Vec<String>,
    /// Source columns a table without `page_key` is ordered by while paging.
    sortable: Vec<String>,
    /// Where the `match_on` columns are in `writes`.
    match_positions: Vec<usize>,
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

/// The columns a source table is paged by: its key, or else a unique index whose columns are never empty.
fn page_key(d: &TableDetails) -> Vec<String> {
    let pk: Vec<String> = d.design.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect();
    if !pk.is_empty() {
        return pk;
    }
    d.design
        .indexes
        .iter()
        .filter(|i| i.unique && !i.columns.is_empty())
        .find(|i| i.columns.iter().all(|n| d.design.columns.iter().any(|c| &c.name == n && !c.nullable)))
        .map(|i| i.columns.clone())
        .unwrap_or_default()
}

/// `int(11) unsigned` → `int`, `character varying(20)` → `character`.
pub(crate) fn base_type(t: &str) -> &str {
    t.trim().split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().unwrap_or("")
}

/// Columns a table without a key can be ordered by. Some types can't be sorted at all (SQL
/// Server's text, ntext, image and xml; Postgres json) and MySQL sorts text and blobs by a prefix.
fn sortable(kind: DbKind, d: &TableDetails) -> Vec<String> {
    let unsortable: &[&str] = match kind {
        DbKind::Sqlserver => &["text", "ntext", "image", "xml", "geography", "geometry"],
        DbKind::Postgres => &["json", "xml", "point", "line", "lseg", "box", "path", "polygon", "circle"],
        DbKind::Mysql => &["tinytext", "text", "mediumtext", "longtext", "tinyblob", "blob", "mediumblob", "longblob", "json", "geometry", "point", "linestring", "polygon"],
        DbKind::Sqlite => &[],
    };
    d.design.columns.iter().filter(|c| !unsortable.contains(&base_type(&c.data_type.to_ascii_lowercase()))).map(|c| c.name.clone()).collect()
}

/// Whether a foreign key points at the table a mapping fills (a key without a schema matches any).
fn refers_to(fk: &crate::design::ForeignKeyDesign, m: &TableMapping) -> bool {
    let schema = |s: Option<&str>| s.filter(|s| !s.is_empty()).map(str::to_string);
    fk.ref_table == m.target_table
        && match (schema(fk.ref_schema.as_deref()), schema(m.target_schema.as_deref())) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
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
        let page_key = page_key(&src);
        let sortable = sortable(source.dialect().kind, &src);
        if page_key.is_empty() && sortable.len() < src.design.columns.len() {
            issues.push(Issue::warning(
                name,
                None,
                format!("{} has no key or unique index, so it's read in pages ordered by its other columns; rows alike in all of them could be read twice or missed. Move it while nothing else writes to it.", m.source_table),
            ));
        }
        let mut writes: Vec<(TargetColumn, Option<ColumnMapping>)> = Vec::new();
        let mut reads: Vec<String> = page_key.clone();
        if let Some(k) = source_key.as_ref().filter(|k| !reads.contains(k)) {
            reads.push(k.clone());
        }
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
            let renumbered = enabled.iter().any(|o| refers_to(fk, o) && o.ids == IdMode::Renumber);
            let copied = m.columns.iter().any(|c| c.target == fk.columns[0] && matches!(c.source, ValueSource::Column { .. }));
            if renumbered && copied {
                issues.push(Issue::warning(name, Some(&fk.columns[0]), format!("{} links to {}, which gets new numbers; choose “New ID of a moved row” so links follow.", fk.columns[0], fk.ref_table)));
            }
        }
        let key_index = target_key.as_ref().and_then(|k| writes.iter().position(|(w, _)| w.name == k.name));
        let mut deferred = Vec::new();
        for (i, (w, mapping)) in writes.iter().enumerate() {
            let fk_to_self = dst.design.foreign_keys.iter().any(|f| f.columns.len() == 1 && f.columns[0] == w.name && refers_to(f, m));
            let ref_to_self = matches!(mapping, Some(ColumnMapping { source: ValueSource::Reference { schema, table, .. }, .. }) if table_key(schema.as_deref(), table) == m.source_key());
            if !(fk_to_self || ref_to_self) || m.write != WriteMode::Insert {
                continue;
            }
            match key_index {
                Some(_) if w.nullable => deferred.push(i),
                Some(_) => issues.push(Issue::warning(name, Some(&w.name), format!("{} links rows of {name} to each other and can't be empty, so each row has to come after the one it points to; if not, the move stops.", w.name))),
                None => issues.push(Issue::warning(name, Some(&w.name), format!("{name} gets its keys from the database, so {} can only point at rows that come earlier.", w.name))),
            }
        }
        let match_positions = m.match_on.iter().filter_map(|c| writes.iter().position(|(w, _)| &w.name == c)).collect();
        out.push(Prepared { mapping: m.clone(), target: dst, writes, reads, source_key, target_key, explicit_identity, deferred, key_index, page_key, sortable, match_positions });
    }
    order(&mut out, &mut issues);
    (out, issues)
}

/// Referenced tables first, so their rows exist (and their new numbers are known) when links arrive.
fn order(tables: &mut Vec<Prepared>, issues: &mut Vec<Issue>) {
    let deps: Vec<HashSet<usize>> = tables
        .iter()
        .enumerate()
        .map(|(me, p)| {
            let mut d: HashSet<usize> = p
                .mapping
                .columns
                .iter()
                .filter_map(|c| match &c.source {
                    ValueSource::Reference { schema, table, .. } => tables.iter().position(|o| o.mapping.source_key() == table_key(schema.as_deref(), table)),
                    _ => None,
                })
                .collect();
            for fk in &p.target.design.foreign_keys {
                d.extend(tables.iter().enumerate().filter(|(_, o)| refers_to(fk, &o.mapping)).map(|(i, _)| i));
            }
            d.remove(&me);
            d
        })
        .collect();
    let mut sorted: Vec<usize> = Vec::new();
    while sorted.len() < tables.len() {
        let next = (0..tables.len()).find(|i| !sorted.contains(i) && deps[*i].iter().all(|d| sorted.contains(d)));
        match next {
            Some(i) => sorted.push(i),
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
    /// The key columns, quoted, and where they are among the columns read.
    key: Vec<String>,
    key_at: Vec<usize>,
    after: Option<Vec<String>>,
    offset: u64,
    done: bool,
}

/// `(a, b) > (x, y)` spelled out so every database understands it: `a > x OR (a = x AND b > y)`.
fn after_sql(d: Dialect, key: &[String], values: &[String]) -> String {
    let terms: Vec<String> = (0..key.len())
        .map(|i| {
            let mut parts: Vec<String> = (0..i).map(|j| format!("{} = {}", key[j], d.string(&values[j]))).collect();
            parts.push(format!("{} > {}", key[i], d.string(&values[i])));
            format!("({})", parts.join(" AND "))
        })
        .collect();
    format!("({})", terms.join(" OR "))
}

impl<'a> Pager<'a> {
    /// `key` must be among `columns`; without it, rows are paged by offset.
    fn new(driver: &'a dyn DbDriver, p: &Prepared, columns: &[String], key: &[String]) -> Self {
        let d = driver.dialect();
        let table = d.table(p.mapping.source_schema.as_deref(), &p.mapping.source_table);
        let key_at: Option<Vec<usize>> = key.iter().map(|k| columns.iter().position(|c| c == k)).collect();
        let (key, key_at, order) = match key_at.filter(|k| !k.is_empty()) {
            Some(at) => (key.iter().map(|k| d.ident(k)).collect(), at, d.ident_list(key)),
            None => (vec![], vec![], d.ident_list(&p.sortable)),
        };
        Self { driver, sql_head: format!("SELECT {} FROM {table}", d.ident_list(columns)), order, key, key_at, after: None, offset: 0, done: false }
    }

    async fn next(&mut self, limit: u32) -> Result<Option<(Vec<ColumnMeta>, Vec<Vec<Cell>>)>> {
        if self.done {
            return Ok(None);
        }
        let d = self.driver.dialect();
        let mut sql = self.sql_head.clone();
        if let Some(after) = &self.after {
            sql.push_str(&format!(" WHERE {}", after_sql(d, &self.key, after)));
        }
        if !self.order.is_empty() {
            sql.push_str(&format!(" ORDER BY {}", self.order));
        } else if d.is_sqlserver() {
            // OFFSET needs an ORDER BY, even when nothing can be sorted.
            sql.push_str(" ORDER BY (SELECT NULL)");
        }
        let keyed = !self.key.is_empty();
        let offset = if keyed { 0 } else { self.offset };
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
        if keyed {
            self.after = rows.last().and_then(|r| self.key_at.iter().map(|i| r[*i].clone()).collect());
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
    // SQL Server's COUNT(*) is an int and overflows past two billion rows.
    let count = if d.is_sqlserver() { "COUNT_BIG(*)" } else { "COUNT(*)" };
    let (_, rows) = driver.fetch(&format!("SELECT {count} FROM {}", d.table(schema, table))).await?;
    Ok(rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0))
}

async fn max_key(driver: &dyn DbDriver, p: &Prepared, key: &str) -> Result<i64> {
    let d = driver.dialect();
    let (_, rows) = driver.fetch(&format!("SELECT MAX({}) FROM {}", d.ident(key), d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table))).await?;
    Ok(rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.trim().parse().ok()).unwrap_or(0))
}

/// The key of the existing target row each set of `match_on` values finds, None when there's none
/// or a value is empty. The target is asked rather than the values compared here, so its own
/// collation decides whether case, accents and trailing spaces matter, as its unique index will.
async fn existing_keys(target: &dyn DbDriver, p: &Prepared, key: &str, lookups: &[Vec<Option<String>>]) -> Result<Vec<Option<i64>>> {
    let d = target.dialect();
    let table = d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table);
    let mut out = vec![None; lookups.len()];
    let asked: Vec<usize> = (0..lookups.len()).filter(|i| lookups[*i].iter().all(Option::is_some)).collect();
    for chunk in asked.chunks(200) {
        let selects: Vec<String> = chunk
            .iter()
            .map(|&i| {
                let on = p.match_positions.iter().zip(&lookups[i]).map(|(&at, v)| {
                    let col = &p.writes[at].0;
                    format!("t.{} = {}", d.ident(&col.name), literal(d, col, v.as_deref()))
                });
                format!("SELECT {i} AS n, t.{} AS k FROM {table} t WHERE {}", d.ident(key), on.collect::<Vec<_>>().join(" AND "))
            })
            .collect();
        let (_, rows) = target.fetch(&selects.join(" UNION ALL ")).await?;
        for r in rows {
            let n = r.first().cloned().flatten().and_then(|v| v.trim().parse::<usize>().ok());
            let k = r.get(1).cloned().flatten().and_then(|v| v.trim().parse::<i64>().ok());
            if let (Some(n), Some(k)) = (n, k) {
                out[n] = Some(k);
            }
        }
    }
    Ok(out)
}

/// New numbers for renumbered tables, after the highest key already there. Rows that will be
/// skipped or updated because they already exist (by `match_on`) keep the existing row's key, so
/// links to them land on the right row; rows that repeat an earlier source row exactly share its key.
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
            let mut pager = Pager::new(source, p, std::slice::from_ref(sk), std::slice::from_ref(sk));
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
        let index: HashMap<&str, usize> = p.reads.iter().enumerate().map(|(i, c)| (c.as_str(), i)).collect();
        let key_at = index[sk.as_str()];
        let mut pager = Pager::new(source, p, &p.reads, &p.page_key);
        let mut scratch = 0i64;
        let mut seen: HashMap<Vec<Option<String>>, i64> = HashMap::new();
        while let Some((meta, rows)) = pager.next(PAGE).await? {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let kinds: Vec<ValueKind> = meta.iter().map(|c| c.kind).collect();
            let mut keyed: Vec<(String, Vec<Option<String>>)> = Vec::with_capacity(rows.len());
            for row in &rows {
                let Some(source_key) = row[key_at].clone() else { continue };
                // A row that won't convert gets a new number; the move reports it later.
                let values = convert_row(d.kind, p, row, &index, &kinds, &maps, &mut scratch)
                    .map(|v| p.match_positions.iter().map(|i| v[*i].clone()).collect())
                    .unwrap_or_default();
                keyed.push((source_key.trim().to_string(), values));
            }
            let lookups: Vec<Vec<Option<String>>> = keyed.iter().map(|(_, v)| v.clone()).collect();
            let found = existing_keys(target, p, &tk.name, &lookups).await?;
            for ((source_key, values), existing) in keyed.into_iter().zip(found) {
                let complete = !values.is_empty() && values.iter().all(Option::is_some);
                let key = match seen.get(&values).filter(|_| complete) {
                    Some(k) => *k,
                    None => existing.unwrap_or_else(|| {
                        next += 1;
                        next - 1
                    }),
                };
                if complete {
                    seen.entry(values).or_insert(key);
                }
                map.insert(source_key, key);
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
        let mut pager = Pager::new(source, p, &p.reads, &p.page_key);
        let mut preview = Vec::new();
        // column → (rows that failed, first example)
        let mut failures: HashMap<String, (u32, String)> = HashMap::new();
        let mut missing_required: HashMap<String, u32> = HashMap::new();
        let mut sampled = 0u32;
        let matching = p.mapping.write != WriteMode::Insert && !p.match_positions.is_empty();
        let mut exact: HashSet<Vec<String>> = HashSet::new();
        let mut loose: HashMap<Vec<String>, Vec<String>> = HashMap::new();
        let mut repeated = 0u32;
        let mut alike: Option<(Vec<String>, Vec<String>)> = None;
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
                        let found: Option<Vec<String>> = p.match_positions.iter().map(|i| values[*i].clone()).collect();
                        if let Some(found) = found.filter(|_| matching) {
                            if !exact.insert(found.clone()) {
                                repeated += 1;
                            } else {
                                let folded: Vec<String> = found.iter().map(|v| v.trim().to_lowercase()).collect();
                                match loose.get(&folded) {
                                    Some(first) if alike.is_none() => alike = Some((first.clone(), found)),
                                    Some(_) => {}
                                    None => {
                                        loose.insert(folded, found);
                                    }
                                }
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
        let on = p.mapping.match_on.join(" + ");
        if repeated > 0 {
            table_issues.push(Issue::warning(&p.name(), None, format!("{} repeat the {on} of an earlier row; only the first of each is moved.", of(repeated))));
        }
        if let Some((a, b)) = alike {
            table_issues.push(Issue::warning(
                &p.name(),
                None,
                format!(
                    "“{}” and “{}” differ only in case or spaces. If {} treats them as the same {on}, the second lands on the first and links to it can break; make them identical with Trim or Lower steps.",
                    a.join(" + "),
                    b.join(" + "),
                    p.name()
                ),
            ));
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

/// The spool holds every moved value, so only its owner may read it.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Spool files of moves that never finished because the app was closed or crashed.
fn remove_stale_spools(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 24 * 3600);
        if name.starts_with("kiyi-move-") && name.ends_with(".sql.jsonl") && stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Which table and rows a spooled statement holds, for progress and error messages.
struct Batch {
    table: usize,
    /// The source rows the statement covers, counting from 1; rows left out as repeats included.
    first_row: u64,
    last_row: u64,
    /// Rows the statement writes.
    rows: u64,
    /// Source rows it accounts for, for progress.
    read: u64,
}

/// Fills in links between rows of the same table once they all exist:
/// `UPDATE t SET parent = CASE id WHEN 4 THEN 9 … END WHERE id IN (4, …)`.
fn link_updates(d: Dialect, p: &Prepared, links: &[(String, usize, String)]) -> Vec<String> {
    let Some(k) = p.key_index else { return vec![] };
    let key = &p.writes[k].0;
    let table = d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table);
    let mut out = Vec::new();
    for &i in &p.deferred {
        let col = &p.writes[i].0;
        let mine: Vec<&(String, usize, String)> = links.iter().filter(|(_, c, _)| *c == i).collect();
        for chunk in mine.chunks(500) {
            // Postgres reads a CASE of quoted values as text; say what type it is.
            let value = |v: &str| if d.kind == DbKind::Postgres { format!("CAST({} AS {})", literal(d, col, Some(v)), col.data_type) } else { literal(d, col, Some(v)) };
            let cases: Vec<String> = chunk.iter().map(|(id, _, v)| format!("WHEN {} THEN {}", literal(d, key, Some(id)), value(v))).collect();
            let ids: Vec<String> = chunk.iter().map(|(id, _, _)| literal(d, key, Some(id))).collect();
            out.push(format!("UPDATE {table} SET {} = CASE {} {} END WHERE {} IN ({})", d.ident(&col.name), d.ident(&key.name), cases.join(" "), d.ident(&key.name), ids.join(", ")));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn spool_batch(file: &mut BufWriter<std::fs::File>, batches: &mut Vec<Batch>, written: &mut [u64], table: usize, sql: String, pending: &mut Vec<Vec<Option<String>>>, first_row: u64, last_row: u64) -> Result<()> {
    let n = pending.len() as u64;
    writeln!(file, "{}", serde_json::to_string(&sql)?)?;
    batches.push(Batch { table, first_row, last_row, rows: n, read: last_row + 1 - first_row });
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
    remove_stale_spools(spool_dir);
    let spool = Spool(spool_dir.join(format!("kiyi-move-{}.sql.jsonl", uuid::Uuid::new_v4())));
    let mut file = BufWriter::new(create_private(&spool.0)?);
    let mut batches: Vec<Batch> = Vec::new();
    let mut read_so_far = 0u64;
    let mut written_per_table = vec![0u64; tables.len()];
    let mut repeated_per_table = vec![0u64; tables.len()];
    for (ti, p) in tables.iter().enumerate() {
        let identity_on = d.is_sqlserver() && p.explicit_identity;
        let qualified = d.table(p.mapping.target_schema.as_deref(), &p.mapping.target_table);
        if identity_on {
            writeln!(file, "{}", serde_json::to_string(&format!("SET IDENTITY_INSERT {qualified} ON"))?)?;
            batches.push(Batch { table: ti, first_row: 0, last_row: 0, rows: 0, read: 0 });
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
        let mut pager = Pager::new(source, p, &p.reads, &p.page_key);
        let mut links: Vec<(String, usize, String)> = Vec::new();
        let mut pending: Vec<Vec<Option<String>>> = Vec::new();
        let mut pending_bytes = 0usize;
        let mut row_no = 0u64;
        let mut flushed = 0u64;
        // A row repeating an earlier one's match values is left out: the database would refuse
        // to update one row twice in a statement, and it already shares that row's key.
        let matching = p.mapping.write != WriteMode::Insert && !p.match_positions.is_empty();
        let mut seen: HashSet<Vec<String>> = HashSet::new();
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
                if matching {
                    let found: Option<Vec<String>> = p.match_positions.iter().map(|i| values[*i].clone()).collect();
                    if found.is_some_and(|f| !seen.insert(f)) {
                        repeated_per_table[ti] += 1;
                        continue;
                    }
                }
                let mut values = values;
                if let Some(key) = p.key_index.and_then(|k| values[k].clone()) {
                    for &i in &p.deferred {
                        if let Some(v) = values[i].take() {
                            links.push((key.clone(), i, v));
                        }
                    }
                }
                pending_bytes += values.iter().map(|v| v.as_ref().map_or(4, String::len) + 4).sum::<usize>();
                pending.push(values);
                if pending.len() >= ROWS_PER_STATEMENT || pending_bytes >= BYTES_PER_STATEMENT {
                    spool_batch(&mut file, &mut batches, &mut written_per_table, ti, statement(d, p, &pending, overriding), &mut pending, flushed + 1, row_no)?;
                    flushed = row_no;
                    pending_bytes = 0;
                }
            }
            read_so_far += rows.len() as u64;
            progress(Progress { stage: Stage::Reading, table: p.name(), done: read_so_far, total });
        }
        if !pending.is_empty() {
            spool_batch(&mut file, &mut batches, &mut written_per_table, ti, statement(d, p, &pending, overriding), &mut pending, flushed + 1, row_no)?;
        } else if let Some(b) = batches.last_mut().filter(|b| b.table == ti && b.rows > 0) {
            // Repeats after the last statement still count as read.
            b.read += row_no - flushed;
        }
        for sql in link_updates(d, p, &links) {
            writeln!(file, "{}", serde_json::to_string(&sql)?)?;
            batches.push(Batch { table: ti, first_row: 0, last_row: 0, rows: 0, read: 0 });
        }
        if identity_on {
            writeln!(file, "{}", serde_json::to_string(&format!("SET IDENTITY_INSERT {qualified} OFF"))?)?;
            batches.push(Batch { table: ti, first_row: 0, last_row: 0, rows: 0, read: 0 });
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
                done += b.read;
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
                if b.rows == 0 { t.name() } else { format!("{} → {}, rows {}–{}", t.mapping.source_table, t.name(), b.first_row, b.last_row) }
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
        tables: tables.iter().zip(&written_per_table).zip(&repeated_per_table).map(|((p, n), r)| TableResult { target: p.name(), rows: *n, repeated: *r }).collect(),
        rows: written_per_table.iter().sum(),
        test_run,
        seconds: started.elapsed().as_secs_f64(),
    })
}
