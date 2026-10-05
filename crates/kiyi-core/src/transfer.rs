//! Export a table view to CSV/JSON, and import CSV files into a table.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::dialect::Dialect;
use crate::dml;
use crate::drivers::DbDriver;
use crate::error::{Error, Result};
use crate::types::{Cell, ColumnMeta, QueryEvent, ValueKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
}

enum Out {
    Csv(Box<csv::Writer<BufWriter<File>>>),
    Json { file: BufWriter<File>, first: bool },
}

/// A JSON value for a cell: numbers, booleans and JSON columns keep their type.
fn json_value(kind: ValueKind, v: &Cell) -> serde_json::Value {
    let Some(v) = v else { return serde_json::Value::Null };
    match kind {
        // Keep big integers / exact decimals exact by emitting them verbatim when valid JSON numbers.
        ValueKind::Number => serde_json::from_str::<serde_json::Number>(v).map(serde_json::Value::Number).unwrap_or_else(|_| v.clone().into()),
        ValueKind::Bool => serde_json::Value::Bool(matches!(v.as_str(), "true" | "t" | "1")),
        ValueKind::Json => serde_json::from_str(v).unwrap_or_else(|_| v.clone().into()),
        _ => v.clone().into(),
    }
}

/// Streams the query's rows into `path`. Returns the number of rows written.
pub async fn export(driver: &dyn DbDriver, sql: &str, format: ExportFormat, path: &Path) -> Result<u64> {
    let file = BufWriter::new(File::create(path)?);
    let out = Mutex::new(match format {
        ExportFormat::Csv => Out::Csv(Box::new(csv::Writer::from_writer(file))),
        ExportFormat::Json => Out::Json { file, first: true },
    });
    let columns: Mutex<Vec<ColumnMeta>> = Mutex::new(Vec::new());
    let count = Mutex::new(0u64);
    let failure: Mutex<Option<String>> = Mutex::new(None);

    let sink = |event: QueryEvent| {
        let fail = |e: String| *failure.lock().unwrap() = Some(e);
        match event {
            QueryEvent::Columns { columns: cols } => {
                if let Out::Csv(w) = &mut *out.lock().unwrap() {
                    if let Err(e) = w.write_record(cols.iter().map(|c| c.name.as_str())) {
                        fail(e.to_string());
                    }
                }
                *columns.lock().unwrap() = cols;
            }
            QueryEvent::Rows { rows } => {
                let cols = columns.lock().unwrap();
                let mut out = out.lock().unwrap();
                for row in &rows {
                    let result = match &mut *out {
                        Out::Csv(w) => w.write_record(row.iter().map(|v| v.as_deref().unwrap_or(""))).map_err(|e| e.to_string()),
                        Out::Json { file, first } => {
                            let object: serde_json::Map<String, serde_json::Value> =
                                cols.iter().zip(row).map(|(c, v)| (c.name.clone(), json_value(c.kind, v))).collect();
                            let sep = if *first { "[\n  " } else { ",\n  " };
                            *first = false;
                            write!(file, "{sep}{}", serde_json::Value::Object(object)).map_err(|e| e.to_string())
                        }
                    };
                    if let Err(e) = result {
                        fail(e);
                        return;
                    }
                }
                *count.lock().unwrap() += rows.len() as u64;
            }
            _ => {}
        }
    };
    driver.execute(sql, &|_| {}, &sink).await?;
    if let Some(e) = failure.into_inner().unwrap() {
        return Err(Error::Invalid(format!("Couldn't write the file: {e}")));
    }
    match out.into_inner().unwrap() {
        Out::Csv(mut w) => w.flush()?,
        Out::Json { mut file, first } => {
            file.write_all(if first { b"[]\n" } else { b"\n]\n" })?;
            file.flush()?;
        }
    }
    Ok(count.into_inner().unwrap())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Data rows in the file (not counting the header).
    pub total: u64,
}

fn reader(path: &Path) -> Result<csv::Reader<File>> {
    // Sniff the delimiter from the first line: comma, semicolon (European Excel) or tab.
    let first = std::fs::read_to_string(path).map_err(|e| Error::Invalid(format!("Couldn't read the file: {e}")))?;
    let line = first.lines().next().unwrap_or("");
    let delimiter = b",;\t".iter().copied().max_by_key(|d| line.bytes().filter(|b| b == d).count()).unwrap_or(b',');
    csv::ReaderBuilder::new().delimiter(delimiter).has_headers(false).flexible(true).from_path(path).map_err(|e| Error::Invalid(e.to_string()))
}

pub fn preview(path: &Path) -> Result<CsvPreview> {
    let mut rows = reader(path)?.into_records();
    let headers: Vec<String> = match rows.next() {
        Some(r) => r.map_err(|e| Error::Invalid(e.to_string()))?.iter().map(|s| s.trim_start_matches('\u{feff}').to_string()).collect(),
        None => return Err(Error::Invalid("The file is empty.".into())),
    };
    let mut sample = Vec::new();
    let mut total = 0u64;
    for r in rows {
        let r = r.map_err(|e| Error::Invalid(e.to_string()))?;
        if sample.len() < 20 {
            sample.push(r.iter().map(str::to_string).collect());
        }
        total += 1;
    }
    Ok(CsvPreview { headers, rows: sample, total })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    pub schema: Option<String>,
    pub table: String,
    /// For each CSV column, the table column it goes into (or none to skip it).
    pub mapping: Vec<Option<String>>,
    pub has_header: bool,
    pub empty_as_null: bool,
}

/// Reads the whole file into INSERT statements, ready to run in one transaction.
pub fn plan_import(d: Dialect, path: &Path, plan: &ImportPlan) -> Result<(Vec<String>, u64)> {
    let targets: Vec<(usize, String)> = plan.mapping.iter().enumerate().filter_map(|(i, c)| c.clone().map(|c| (i, c))).collect();
    if targets.is_empty() {
        return Err(Error::Invalid("Choose at least one column to import.".into()));
    }
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for (n, record) in reader(path)?.into_records().enumerate() {
        let record = record.map_err(|e| Error::Invalid(format!("Line {}: {e}", n + 1)))?;
        if n == 0 && plan.has_header {
            continue;
        }
        rows.push(
            targets
                .iter()
                .map(|(i, _)| {
                    let v = record.get(*i).unwrap_or("");
                    if v.is_empty() && plan.empty_as_null { None } else { Some(v.to_string()) }
                })
                .collect(),
        );
    }
    let columns: Vec<String> = targets.into_iter().map(|(_, c)| c).collect();
    let count = rows.len() as u64;
    Ok((dml::plan_bulk_insert(d, plan.schema.as_deref(), &plan.table, &columns, &rows, 500), count))
}
