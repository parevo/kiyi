//! Export a table view to CSV/JSON, and import CSV files into a table.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::dml;
use crate::drivers::DbDriver;
use crate::error::{Error, ErrorInfo, Result};
use encoding_rs::Encoding;
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
    /// The text encoding it was read with, e.g. "UTF-8" or "windows-1254".
    pub encoding: String,
}

/// Rows per INSERT statement during an import.
const IMPORT_BATCH: usize = 500;

/// Bytes that are Turkish letters in Windows-1254 (Ğ İ Ş ğ ı ş) but rare Icelandic ones in
/// Windows-1252 (Ð Ý Þ ð ý þ), which tells the two apart.
const TURKISH_BYTES: [u8; 6] = [0xD0, 0xDD, 0xDE, 0xF0, 0xFD, 0xFE];

/// The file's text encoding. Excel saves "CSV" in the system's legacy code page, so a file
/// that isn't valid UTF-8 from start to end is read as Windows-1254 (Turkish) or -1252 (Western).
pub fn detect_encoding(path: &Path) -> Result<&'static Encoding> {
    use std::io::Read;
    let mut file = File::open(path).map_err(|e| Error::Invalid(format!("Couldn't read the file: {e}")))?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    let mut first = true;
    let mut utf8 = true;
    let mut turkish = false;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        if first {
            first = false;
            if let Some((enc, _)) = Encoding::for_bom(chunk) {
                return Ok(enc);
            }
        }
        turkish |= chunk.iter().any(|b| TURKISH_BYTES.contains(b));
        if utf8 {
            carry.extend_from_slice(chunk);
            match std::str::from_utf8(&carry) {
                Ok(_) => carry.clear(),
                // A character split across reads: keep its first bytes for the next round.
                Err(e) if e.error_len().is_none() => carry = carry[e.valid_up_to()..].to_vec(),
                Err(_) => utf8 = false,
            }
        }
    }
    Ok(if utf8 && carry.is_empty() {
        encoding_rs::UTF_8
    } else if turkish {
        encoding_rs::WINDOWS_1254
    } else {
        encoding_rs::WINDOWS_1252
    })
}

fn encoding(label: Option<&str>, path: &Path) -> Result<&'static Encoding> {
    match label.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => Encoding::for_label(l.as_bytes()).ok_or_else(|| Error::Invalid(format!("Unknown text encoding: {l}"))),
        None => detect_encoding(path),
    }
}

/// The file as UTF-8 text, whatever it was saved as.
fn decoded(path: &Path, enc: &'static Encoding) -> Result<impl std::io::Read + Send> {
    let file = File::open(path).map_err(|e| Error::Invalid(format!("Couldn't read the file: {e}")))?;
    Ok(encoding_rs_io::DecodeReaderBytesBuilder::new().encoding(Some(enc)).bom_override(true).build(file))
}

fn reader(path: &Path, enc: &'static Encoding) -> Result<csv::Reader<impl std::io::Read + Send>> {
    use std::io::BufRead;
    // Sniff the delimiter from the first line: comma, semicolon (European Excel) or tab.
    let mut line = String::new();
    std::io::BufReader::new(decoded(path, enc)?).read_line(&mut line).map_err(|e| Error::Invalid(format!("Couldn't read the file: {e}")))?;
    let delimiter = b",;\t".iter().copied().max_by_key(|d| line.bytes().filter(|b| b == d).count()).unwrap_or(b',');
    Ok(csv::ReaderBuilder::new().delimiter(delimiter).has_headers(false).flexible(true).from_reader(decoded(path, enc)?))
}

/// The first rows of the file. `encoding` overrides detection.
pub fn preview(path: &Path, encoding_label: Option<&str>) -> Result<CsvPreview> {
    let enc = encoding(encoding_label, path)?;
    let mut rows = reader(path, enc)?.into_records();
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
    Ok(CsvPreview { headers, rows: sample, total, encoding: enc.name().to_string() })
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
    /// Text encoding label ("UTF-8", "windows-1254"…); detected when missing.
    #[serde(default)]
    pub encoding: Option<String>,
}

/// Imports the file in one transaction, a batch of rows at a time, so a large file never sits in
/// memory whole. Nothing is kept if any row fails. Returns the number of rows imported.
pub async fn import(driver: &dyn DbDriver, path: &Path, plan: &ImportPlan) -> Result<u64> {
    let d = driver.dialect();
    let targets: Vec<(usize, String)> = plan.mapping.iter().enumerate().filter_map(|(i, c)| c.clone().map(|c| (i, c))).collect();
    if targets.is_empty() {
        return Err(Error::Invalid("Choose at least one column to import.".into()));
    }
    let columns: Vec<String> = targets.iter().map(|(_, c)| c.clone()).collect();
    let mut records = reader(path, encoding(plan.encoding.as_deref(), path)?)?.into_records().enumerate();
    if plan.has_header {
        records.next();
    }
    let mut count = 0u64;
    let mut next = || -> Option<Result<String>> {
        let mut rows: Vec<Vec<Cell>> = Vec::with_capacity(IMPORT_BATCH);
        for (n, record) in records.by_ref() {
            let record = match record {
                Ok(r) => r,
                Err(e) => return Some(Err(Error::Invalid(format!("Line {}: {e}", n + 1)))),
            };
            rows.push(
                targets
                    .iter()
                    .map(|(i, _)| {
                        let v = record.get(*i).unwrap_or("");
                        if v.is_empty() && plan.empty_as_null { None } else { Some(v.to_string()) }
                    })
                    .collect(),
            );
            if rows.len() == IMPORT_BATCH {
                break;
            }
        }
        if rows.is_empty() {
            return None;
        }
        count += rows.len() as u64;
        dml::plan_bulk_insert(d, plan.schema.as_deref(), &plan.table, &columns, &rows, IMPORT_BATCH).into_iter().next().map(Ok)
    };
    match driver.execute_stream(&mut next).await {
        Ok(_) => Ok(count),
        Err(Error::Script { index, source }) => {
            let first = index * IMPORT_BATCH + 1;
            Err(Error::Invalid(format!("Nothing was imported. A row between {} and {} was rejected: {}", first, first + IMPORT_BATCH - 1, ErrorInfo::from(&*source).message)))
        }
        Err(e) => Err(e),
    }
}
