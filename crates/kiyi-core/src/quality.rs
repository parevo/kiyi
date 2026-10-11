//! A data quality check for one table: the kind of mess that builds up when people type data in
//! by hand or it arrives from other systems. Duplicates where there should be none, the same city
//! spelled three ways, stray spaces, blank text, addresses that aren't e-mail addresses, numbers
//! far outside the rest, dates in the future.
//!
//! The check reads a sample of the table (the first rows, up to a limit) and works it out in
//! memory, so it's quick on any size of table. Where a fix is safe and mechanical (trimming
//! spaces, one spelling for a value, blank text to empty) it comes as SQL for the whole table,
//! to be reviewed before it runs; otherwise the finding carries filters that show the rows.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::catalog::TypeCategory;
use crate::config::DbKind;
use crate::design::TableDetails;
use crate::dialect::Dialect;
use crate::dml::{BrowseRequest, Filter, FilterOp};
use crate::drivers::DbDriver;
use crate::error::Result;
use crate::types::Cell;

pub const SAMPLE: u32 = 50_000;
const EXAMPLES: usize = 5;
/// Columns with more distinct values than this aren't categories, so spelling variants aren't looked for.
const CATEGORY_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Duplicates,
    CopiedRows,
    Spellings,
    Spaces,
    BlankText,
    InvalidEmail,
    Outliers,
    Negative,
    FutureDates,
    AncientDates,
    NeverFilled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    pub value: Cell,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    /// What the button says: "Trim spaces", "Use “İstanbul” everywhere".
    pub label: String,
    pub statements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub kind: Kind,
    pub severity: Severity,
    pub column: Option<String>,
    pub title: String,
    pub detail: String,
    /// Rows affected among those checked.
    pub rows: u64,
    pub examples: Vec<Example>,
    pub fix: Option<Fix>,
    /// Filters that show the rows in the table view; empty when they can't be expressed.
    pub show: Vec<Filter>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityReport {
    pub schema: Option<String>,
    pub table: String,
    pub rows: u64,
    /// Rows looked at; less than `rows` on a big table.
    pub checked: u64,
    pub findings: Vec<Finding>,
}

/// Case, Turkish dotted and dotless i, common accents and spacing folded away, so spellings of
/// one value land together: "İstanbul", "Istanbul " and "istanbul" all become "istanbul".
pub(crate) fn fold(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| match c {
            'İ' | 'I' | 'ı' | 'í' | 'ì' | 'î' | 'ï' => 'i',
            'Ş' | 'ş' => 's',
            'Ç' | 'ç' => 'c',
            'Ğ' | 'ğ' => 'g',
            'Ö' | 'ö' | 'ó' | 'ò' | 'ô' | 'õ' => 'o',
            'Ü' | 'ü' | 'ú' | 'ù' | 'û' => 'u',
            'é' | 'è' | 'ê' | 'ë' | 'É' => 'e',
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'Á' | 'Ã' => 'a',
            'ñ' => 'n',
            c => c,
        })
        .flat_map(char::to_lowercase)
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn looks_like(name: &str, words: &[&str]) -> bool {
    let n = norm(name);
    words.iter().any(|w| n == *w || n.ends_with(w) || n.starts_with(w))
}

const IDENTIFYING: &[&str] = &["email", "eposta", "mail", "phone", "telefon", "gsm", "mobile", "tckn", "tcno", "ssn", "sku", "barcode", "barkod", "username", "slug", "iban", "vatnumber", "taxnumber", "vergino"];
const EMAIL: &[&str] = &["email", "eposta", "mail"];
/// Dates that are meant to be ahead.
const FUTURE_OK: &[&str] = &["due", "expire", "expires", "expiry", "end", "ends", "until", "deadline", "scheduled", "valid", "next", "renew", "planned", "bitis", "vade"];
/// Numbers that are naturally signed.
const SIGNED_OK: &[&str] = &["balance", "diff", "delta", "change", "adjust", "offset", "lat", "lng", "lon", "latitude", "longitude", "bakiye", "fark"];

fn is_email(v: &str) -> bool {
    let v = v.trim();
    let Some((local, domain)) = v.split_once('@') else { return false };
    !local.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !v.contains(char::is_whitespace)
}

fn examples(counts: &HashMap<Option<String>, u64>) -> Vec<Example> {
    let mut v: Vec<Example> = counts.iter().map(|(k, c)| Example { value: k.clone(), count: *c }).collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
    v.truncate(EXAMPLES);
    v
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", fmt(n), if n == 1 { one } else { many })
}

fn fmt(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The SQL that measures a text's length including trailing spaces: SQL Server and MariaDB
/// compare 'a ' = 'a', so a plain `c <> TRIM(c)` would never match there.
fn length_sql(d: Dialect, col: &str) -> String {
    match d.kind {
        DbKind::Sqlserver => format!("DATALENGTH({col})"),
        DbKind::Mysql => format!("CHAR_LENGTH({col})"),
        _ => format!("LENGTH({col})"),
    }
}

fn trim_sql(d: Dialect, col: &str) -> String {
    if d.is_sqlserver() { format!("LTRIM(RTRIM({col}))") } else { format!("TRIM({col})") }
}

struct Column<'a> {
    name: &'a str,
    index: usize,
    category: TypeCategory,
    nullable: bool,
    key: bool,
    unique: bool,
    link: bool,
}

/// Checks a table's first rows (up to `sample`) and says what looks wrong.
pub async fn check_table(driver: &dyn DbDriver, schema: Option<&str>, table: &str, sample: u32) -> Result<QualityReport> {
    let d = driver.dialect();
    let details = driver.table_details(schema, table).await?;
    let req = BrowseRequest {
        schema: schema.map(str::to_string),
        table: table.into(),
        filters: vec![],
        raw_where: None,
        search: None,
        search_columns: vec![],
        sort: vec![],
        tiebreak: details.design.primary_key(),
        limit: sample,
        offset: 0,
    };
    let (_, total) = driver.fetch(&crate::dml::count_sql(d, &req)).await?;
    let rows: u64 = total.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let (meta, data) = driver.fetch(&crate::dml::browse_sql(d, &req)).await?;
    let findings = analyse(d, &details, &meta.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), &data, now_utc());
    Ok(QualityReport { schema: schema.map(str::to_string), table: table.into(), rows, checked: data.len() as u64, findings })
}

fn now_utc() -> chrono::NaiveDateTime {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0).map(|t| t.naive_utc()).unwrap_or_default()
}

/// The checks themselves, on rows already read. Separate from the database for testing.
pub(crate) fn analyse(d: Dialect, details: &TableDetails, names: &[String], data: &[Vec<Cell>], now: chrono::NaiveDateTime) -> Vec<Finding> {
    let design = &details.design;
    let table = d.table(details.schema.as_deref(), &design.name);
    let writable = !details.is_view;
    let unique_cols: HashSet<&str> = design.indexes.iter().filter(|i| i.unique && i.columns.len() == 1).map(|i| i.columns[0].as_str()).collect();
    let link_cols: HashSet<&str> = design.foreign_keys.iter().flat_map(|f| f.columns.iter().map(String::as_str)).collect();
    let columns: Vec<Column> = names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let c = design.columns.iter().find(|c| &c.name == name)?;
            Some(Column {
                name: &c.name,
                index,
                category: crate::catalog::category(d.kind, &c.data_type),
                nullable: c.nullable,
                key: c.primary_key,
                unique: c.primary_key || unique_cols.contains(c.name.as_str()) || !c.enum_values.is_empty(),
                link: link_cols.contains(c.name.as_str()),
            })
        })
        .collect();
    let checked = data.len() as u64;
    let mut out = Vec::new();
    if checked == 0 {
        return out;
    }

    for c in &columns {
        let values: Vec<&Cell> = data.iter().map(|r| r.get(c.index).unwrap_or(&None)).collect();
        let present: Vec<&str> = values.iter().filter_map(|v| v.as_deref()).collect();
        let col = d.ident(c.name);
        let textual = matches!(c.category, TypeCategory::Text | TypeCategory::Other);

        if present.is_empty() {
            out.push(Finding {
                kind: Kind::NeverFilled,
                severity: Severity::Info,
                column: Some(c.name.into()),
                title: format!("{} is always empty", c.name),
                detail: format!("Every one of the {} rows checked has no {}. It may be unused.", fmt(checked), c.name),
                rows: checked,
                examples: vec![],
                fix: None,
                show: vec![],
            });
            continue;
        }

        if textual {
            // Text that is only spaces, or nothing: it looks filled in but isn't.
            let blank = present.iter().filter(|v| v.trim().is_empty()).count() as u64;
            if blank > 0 {
                out.push(Finding {
                    kind: Kind::BlankText,
                    severity: Severity::Warning,
                    column: Some(c.name.into()),
                    title: format!("{} has blank text", c.name),
                    detail: format!("{} only spaces or nothing, which looks filled in but isn't; filters for an empty {} miss them.", plural(blank, "value is", "values are"), c.name),
                    rows: blank,
                    examples: vec![],
                    fix: (c.nullable && writable).then(|| Fix {
                        label: "Make them empty (NULL)".into(),
                        statements: vec![format!("UPDATE {table} SET {col} = NULL WHERE {} = ''", trim_sql(d, &col))],
                    }),
                    show: vec![Filter { column: c.name.into(), op: FilterOp::Eq, value: String::new() }],
                });
            }

            // Spaces before or after the text.
            let mut spaced: HashMap<Option<String>, u64> = HashMap::new();
            for v in &present {
                if !v.trim().is_empty() && v.trim() != *v {
                    *spaced.entry(Some(format!("“{v}”"))).or_default() += 1;
                }
            }
            let n: u64 = spaced.values().sum();
            if n > 0 {
                out.push(Finding {
                    kind: Kind::Spaces,
                    severity: Severity::Warning,
                    column: Some(c.name.into()),
                    title: format!("{} has extra spaces", c.name),
                    detail: format!("{} start or end with spaces, so they don't match the same value typed cleanly.", plural(n, "value", "values")),
                    rows: n,
                    examples: examples(&spaced),
                    fix: writable.then(|| Fix {
                        label: "Trim the spaces".into(),
                        statements: vec![format!("UPDATE {table} SET {col} = {} WHERE {} <> {}", trim_sql(d, &col), length_sql(d, &col), length_sql(d, &trim_sql(d, &col)))],
                    }),
                    show: vec![],
                });
            }

            // One value, several spellings: only in category-like columns.
            let mut counts: HashMap<&str, u64> = HashMap::new();
            for v in &present {
                *counts.entry(v).or_default() += 1;
            }
            if counts.len() <= CATEGORY_LIMIT && !c.unique && !looks_like(c.name, IDENTIFYING) && present.iter().all(|v| v.chars().count() <= 60) {
                let mut groups: HashMap<String, Vec<(&str, u64)>> = HashMap::new();
                for (v, n) in &counts {
                    if !v.trim().is_empty() {
                        groups.entry(fold(v)).or_default().push((v, *n));
                    }
                }
                let mut clusters: Vec<Vec<(&str, u64)>> = groups.into_values().filter(|g| g.len() > 1).collect();
                for g in &mut clusters {
                    g.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
                }
                clusters.sort_by(|a, b| b.iter().map(|x| x.1).sum::<u64>().cmp(&a.iter().map(|x| x.1).sum::<u64>()));
                if !clusters.is_empty() {
                    let off: u64 = clusters.iter().map(|g| g[1..].iter().map(|x| x.1).sum::<u64>()).sum();
                    let first = &clusters[0];
                    let spellings = first.iter().map(|(v, n)| format!("“{v}” ({})", fmt(*n))).collect::<Vec<_>>().join(", ");
                    let statements: Vec<String> = clusters
                        .iter()
                        .map(|g| {
                            let others: Vec<String> = g[1..].iter().map(|(v, _)| d.string(v)).collect();
                            format!("UPDATE {table} SET {col} = {} WHERE {col} IN ({})", d.string(g[0].0), others.join(", "))
                        })
                        .collect();
                    out.push(Finding {
                        kind: Kind::Spellings,
                        severity: Severity::Warning,
                        column: Some(c.name.into()),
                        title: format!("{} spells the same value differently", c.name),
                        detail: format!("{} written more than one way, e.g. {spellings}. Reports and filters count them separately.", plural(clusters.len() as u64, "value is", "values are")),
                        rows: off,
                        examples: first.iter().map(|(v, n)| Example { value: Some((*v).to_string()), count: *n }).collect(),
                        fix: writable.then(|| Fix {
                            label: if clusters.len() == 1 { format!("Use “{}” everywhere", first[0].0) } else { format!("Use the most common spelling of each ({})", clusters.len()) },
                            statements,
                        }),
                        show: vec![Filter { column: c.name.into(), op: FilterOp::In, value: first[1..].iter().map(|(v, _)| v.to_string()).collect::<Vec<_>>().join(", ") }],
                    });
                }
            }
        }

        // Values that should identify one row but appear more than once.
        if !c.unique && !c.key && !c.link && looks_like(c.name, IDENTIFYING) {
            let mut seen: HashMap<String, (String, u64)> = HashMap::new();
            for v in present.iter().filter(|v| !v.trim().is_empty()) {
                let e = seen.entry(v.trim().to_lowercase()).or_insert_with(|| (v.to_string(), 0));
                e.1 += 1;
            }
            let dupes: HashMap<Option<String>, u64> = seen.into_values().filter(|(_, n)| *n > 1).map(|(v, n)| (Some(v), n)).collect();
            if !dupes.is_empty() {
                let rows: u64 = dupes.values().sum();
                let values: Vec<String> = dupes.keys().flatten().take(50).cloned().collect();
                out.push(Finding {
                    kind: Kind::Duplicates,
                    severity: Severity::Warning,
                    column: Some(c.name.into()),
                    title: format!("{} has duplicates", c.name),
                    detail: format!("{} appear more than once, in {}. Each {} usually belongs to one row.", plural(dupes.len() as u64, "value", "values"), plural(rows, "row", "rows"), c.name),
                    rows,
                    examples: examples(&dupes),
                    fix: None,
                    show: if values.iter().any(|v| v.contains(',')) { vec![] } else { vec![Filter { column: c.name.into(), op: FilterOp::In, value: values.join(", ") }] },
                });
            }
        }

        if looks_like(c.name, EMAIL) && textual {
            let mut bad: HashMap<Option<String>, u64> = HashMap::new();
            for v in present.iter().filter(|v| !v.trim().is_empty() && !is_email(v)) {
                *bad.entry(Some((*v).to_string())).or_default() += 1;
            }
            let n: u64 = bad.values().sum();
            if n > 0 {
                out.push(Finding {
                    kind: Kind::InvalidEmail,
                    severity: Severity::Warning,
                    column: Some(c.name.into()),
                    title: format!("{} has values that aren't e-mail addresses", c.name),
                    detail: format!("{} can't receive mail as written.", plural(n, "value", "values")),
                    rows: n,
                    examples: examples(&bad),
                    fix: None,
                    show: vec![],
                });
            }
        }

        if matches!(c.category, TypeCategory::Number | TypeCategory::Decimal) && !c.key && !c.link && !norm(c.name).ends_with("id") {
            let mut nums: Vec<f64> = present.iter().filter_map(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).collect();
            if nums.len() >= 30 {
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let q = |p: f64| nums[((nums.len() - 1) as f64 * p).round() as usize];
                let (q1, q3) = (q(0.25), q(0.75));
                let iqr = q3 - q1;
                let negatives = nums.iter().filter(|v| **v < 0.0).count();
                // A handful below zero (under 5%) among positives looks like a mistake; more is a signed column.
                if negatives > 0 && negatives * 20 < nums.len() && !looks_like(c.name, SIGNED_OK) {
                    let mut ex: HashMap<Option<String>, u64> = HashMap::new();
                    for v in nums.iter().filter(|v| **v < 0.0).take(EXAMPLES) {
                        *ex.entry(Some(trim_float(*v))).or_default() += 1;
                    }
                    out.push(Finding {
                        kind: Kind::Negative,
                        severity: Severity::Warning,
                        column: Some(c.name.into()),
                        title: format!("{} has a few negative values", c.name),
                        detail: format!("{} below zero, where almost every other row is zero or more.", plural(negatives as u64, "value is", "values are")),
                        rows: negatives as u64,
                        examples: examples(&ex),
                        fix: None,
                        show: vec![Filter { column: c.name.into(), op: FilterOp::Lt, value: "0".into() }],
                    });
                }
                if iqr > 0.0 {
                    let (low, high) = (q1 - 3.0 * iqr, q3 + 3.0 * iqr);
                    let above: Vec<f64> = nums.iter().copied().filter(|v| *v > high).collect();
                    let below = nums.iter().filter(|v| **v < low && **v >= 0.0).count();
                    let n = above.len() + below;
                    if n > 0 && (n as f64) < nums.len() as f64 * 0.02 {
                        let mut ex: HashMap<Option<String>, u64> = HashMap::new();
                        for v in above.iter().rev().take(EXAMPLES) {
                            *ex.entry(Some(trim_float(*v))).or_default() += 1;
                        }
                        out.push(Finding {
                            kind: Kind::Outliers,
                            severity: Severity::Warning,
                            column: Some(c.name.into()),
                            title: format!("{} has values far from the rest", c.name),
                            detail: format!("{} outside {} to {}, where most rows are between {} and {}. Typos or a different unit?", plural(n as u64, "value is", "values are"), trim_float(low.max(0.0)), trim_float(high), trim_float(q1), trim_float(q3)),
                            rows: n as u64,
                            examples: examples(&ex),
                            fix: None,
                            show: if above.is_empty() { vec![] } else { vec![Filter { column: c.name.into(), op: FilterOp::Gt, value: trim_float(high) }] },
                        });
                    }
                }
            }
        }

        if matches!(c.category, TypeCategory::Date | TypeCategory::DateTime) {
            let moments: Vec<chrono::NaiveDateTime> = present.iter().filter_map(|v| crate::migrate::convert::parse_datetime(v).map(|(t, off)| off.map(|o| t - chrono::TimeDelta::seconds(o as i64)).unwrap_or(t))).collect();
            let tomorrow = now + chrono::TimeDelta::days(1);
            if !looks_like(c.name, FUTURE_OK) {
                let future: Vec<&chrono::NaiveDateTime> = moments.iter().filter(|t| **t > tomorrow).collect();
                if !future.is_empty() {
                    let mut ex: HashMap<Option<String>, u64> = HashMap::new();
                    for t in future.iter().take(EXAMPLES) {
                        *ex.entry(Some(t.format("%Y-%m-%d").to_string())).or_default() += 1;
                    }
                    out.push(Finding {
                        kind: Kind::FutureDates,
                        severity: Severity::Warning,
                        column: Some(c.name.into()),
                        title: format!("{} has dates in the future", c.name),
                        detail: format!("{} after today, for something that should already have happened.", plural(future.len() as u64, "date is", "dates are")),
                        rows: future.len() as u64,
                        examples: examples(&ex),
                        fix: None,
                        show: vec![Filter { column: c.name.into(), op: FilterOp::Gt, value: tomorrow.format("%Y-%m-%d").to_string() }],
                    });
                }
            }
            let floor = chrono::NaiveDate::from_ymd_opt(1900, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap();
            let ancient = moments.iter().filter(|t| **t < floor).count() as u64;
            if ancient > 0 {
                out.push(Finding {
                    kind: Kind::AncientDates,
                    severity: Severity::Warning,
                    column: Some(c.name.into()),
                    title: format!("{} has dates before 1900", c.name),
                    detail: format!("{} a date before 1900: a placeholder like 0001-01-01, or a typo in the year?", plural(ancient, "row has", "rows have")),
                    rows: ancient,
                    examples: vec![],
                    fix: None,
                    show: vec![Filter { column: c.name.into(), op: FilterOp::Lt, value: "1900-01-01".into() }],
                });
            }
        }
    }

    // Whole rows that repeat another row in everything but the key.
    // Only meaningful when rows have enough to tell them apart: three columns or more, one of
    // them nearly always different (a name, an address, a time). Otherwise repeats are normal.
    let compare: Vec<&Column> = columns.iter().filter(|c| !c.key).collect();
    let telling = compare.iter().any(|c| {
        let distinct: HashSet<&Cell> = data.iter().map(|r| r.get(c.index).unwrap_or(&None)).collect();
        distinct.len() as f64 >= data.len() as f64 * 0.9
    });
    if compare.len() >= 3 && telling && columns.iter().any(|c| c.key) {
        let mut seen: HashMap<Vec<&Cell>, u64> = HashMap::new();
        for r in data {
            *seen.entry(compare.iter().map(|c| r.get(c.index).unwrap_or(&None)).collect()).or_default() += 1;
        }
        let extra: u64 = seen.values().filter(|n| **n > 1).map(|n| n - 1).sum();
        if extra > 0 {
            out.push(Finding {
                kind: Kind::CopiedRows,
                severity: Severity::Warning,
                column: None,
                title: "Some rows are copies of others".into(),
                detail: format!("{} the same as another row in every column except the key, often from an import that ran twice.", plural(extra, "row is", "rows are")),
                rows: extra,
                examples: vec![],
                fix: None,
                show: vec![],
            });
        }
    }

    out.sort_by_key(|f| (f.severity != Severity::Warning, std::cmp::Reverse(f.rows)));
    out
}

fn trim_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v:.2}") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::{ColumnDesign, IndexDesign, TableDesign};

    fn col(name: &str, data_type: &str, pk: bool) -> ColumnDesign {
        ColumnDesign { original: None, name: name.into(), data_type: data_type.into(), nullable: !pk, default: None, primary_key: pk, auto_increment: pk, comment: None, generated: false, extra: None, enum_values: vec![] }
    }

    fn details(cols: Vec<ColumnDesign>, indexes: Vec<IndexDesign>) -> TableDetails {
        TableDetails { schema: None, design: TableDesign { name: "people".into(), columns: cols, indexes, foreign_keys: vec![], primary_key_name: None }, is_view: false, row_estimate: None }
    }

    fn row(v: &[&str]) -> Vec<Cell> {
        v.iter().map(|s| if *s == "∅" { None } else { Some(s.to_string()) }).collect()
    }

    fn now() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 11).unwrap().and_hms_opt(12, 0, 0).unwrap()
    }

    #[test]
    fn folding_brings_spellings_together() {
        assert_eq!(fold("  İstanbul "), "istanbul");
        assert_eq!(fold("ISTANBUL"), "istanbul");
        assert_eq!(fold("São  Paulo"), "sao paulo");
        assert_ne!(fold("Izmir"), fold("Ankara"));
    }

    #[test]
    fn finds_the_usual_mess_and_offers_safe_fixes() {
        let d = details(
            vec![col("id", "integer", true), col("name", "text", false), col("email", "text", false), col("city", "text", false), col("total", "numeric", false), col("signed_up_at", "timestamp", false), col("note", "text", false)],
            vec![],
        );
        let names: Vec<String> = ["id", "name", "email", "city", "total", "signed_up_at", "note"].iter().map(|s| s.to_string()).collect();
        let mut data = Vec::new();
        for i in 0..60 {
            let city = match i % 10 {
                0 => "Istanbul",
                1 => "istanbul ",
                2..=5 => "İstanbul",
                _ => "Ankara",
            };
            let email = match i {
                0 | 1 => "dup@x.io".to_string(),
                2 => "not-an-email".to_string(),
                _ => format!("p{i}@x.io"),
            };
            let total = match i {
                3 => "90000".to_string(),
                4 => "-5".to_string(),
                _ => (100 + i).to_string(),
            };
            let when = if i == 5 { "2030-01-01 00:00:00" } else { "2026-01-01 00:00:00" };
            let name = if i == 6 { "   " } else { "Ada" };
            data.push(row(&[&i.to_string(), name, &email, city, &total, when, "∅"]));
        }
        data.push(row(&["99", "Ada", "p59@x.io", "Ankara", "159", "2026-01-01 00:00:00", "∅"])); // a copy of row 59, but for the key
        let f = analyse(Dialect::POSTGRES, &d, &names, &data, now());
        let by = |k: Kind| f.iter().find(|x| x.kind == k).unwrap_or_else(|| panic!("no {k:?} in {f:#?}"));

        let s = by(Kind::Spellings);
        assert_eq!(s.column.as_deref(), Some("city"));
        assert_eq!(s.examples[0].value.as_deref(), Some("İstanbul"), "the most common spelling leads");
        assert_eq!(s.fix.as_ref().unwrap().statements[0].split(" WHERE").next().unwrap(), r#"UPDATE "people" SET "city" = 'İstanbul'"#);
        assert!(s.fix.as_ref().unwrap().statements[0].contains("'Istanbul'") && s.fix.as_ref().unwrap().statements[0].contains("'istanbul '"));

        let sp = by(Kind::Spaces);
        assert_eq!(sp.rows, 6);
        assert_eq!(sp.fix.as_ref().unwrap().statements[0], r#"UPDATE "people" SET "city" = TRIM("city") WHERE LENGTH("city") <> LENGTH(TRIM("city"))"#);

        assert_eq!(by(Kind::BlankText).column.as_deref(), Some("name"));
        assert_eq!(by(Kind::Duplicates).examples[0].value.as_deref(), Some("dup@x.io"));
        assert_eq!(by(Kind::InvalidEmail).examples[0].value.as_deref(), Some("not-an-email"));
        assert_eq!(by(Kind::Outliers).examples[0].value.as_deref(), Some("90000"));
        assert_eq!(by(Kind::Negative).rows, 1);
        assert_eq!(by(Kind::FutureDates).examples[0].value.as_deref(), Some("2030-01-01"));
        assert_eq!(by(Kind::NeverFilled).column.as_deref(), Some("note"));
        assert_eq!(by(Kind::CopiedRows).rows, 1);
    }

    #[test]
    fn clean_columns_and_unique_ones_stay_quiet() {
        let d = details(vec![col("id", "integer", true), col("email", "text", false), col("city", "text", false)], vec![IndexDesign { original: None, name: "u".into(), columns: vec!["email".into()], unique: true, is_constraint: false }]);
        let names: Vec<String> = ["id", "email", "city"].iter().map(|s| s.to_string()).collect();
        let data: Vec<Vec<Cell>> = (0..40).map(|i| row(&[&i.to_string(), &format!("p{}@x.io", i % 39), if i % 2 == 0 { "Paris" } else { "Lyon" }])).collect();
        let f = analyse(Dialect::POSTGRES, &d, &names, &data, now());
        assert!(f.iter().all(|x| x.kind != Kind::Duplicates), "a unique column's duplicates are the database's job: {f:#?}");
        assert!(f.iter().all(|x| x.kind != Kind::Spellings && x.kind != Kind::Spaces), "{f:#?}");
    }

    #[test]
    fn repeats_in_a_narrow_table_are_not_copies() {
        let d = details(vec![col("id", "integer", true), col("city", "text", false)], vec![]);
        let names: Vec<String> = ["id", "city"].iter().map(|s| s.to_string()).collect();
        let data: Vec<Vec<Cell>> = (0..40).map(|i| row(&[&i.to_string(), if i % 2 == 0 { "Paris" } else { "Lyon" }])).collect();
        assert!(analyse(Dialect::POSTGRES, &d, &names, &data, now()).iter().all(|f| f.kind != Kind::CopiedRows));
    }

    #[test]
    fn trailing_spaces_are_found_where_the_database_ignores_them() {
        assert_eq!(length_sql(Dialect::SQLSERVER, "[c]"), "DATALENGTH([c])");
        assert_eq!(trim_sql(Dialect::SQLSERVER, "[c]"), "LTRIM(RTRIM([c]))");
        assert_eq!(length_sql(Dialect::MYSQL, "`c`"), "CHAR_LENGTH(`c`)");
    }
}
