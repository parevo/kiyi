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
    Xlsx,
}

enum Out {
    Csv(Box<csv::Writer<BufWriter<File>>>),
    Json { file: BufWriter<File>, first: bool },
    Xlsx { book: Box<rust_xlsxwriter::Workbook>, row: u32 },
}

/// Rows in an Excel sheet, header included.
const EXCEL_MAX_ROWS: u32 = 1_048_576;
/// Excel keeps 15 significant digits; longer numbers (IDs, exact decimals) stay text so nothing is rounded.
const EXCEL_DIGITS: usize = 15;
const EXCEL_MAX_TEXT: usize = 32_767;

fn excel_cell(sheet: &mut rust_xlsxwriter::Worksheet, row: u32, col: u16, kind: ValueKind, v: &Cell) -> std::result::Result<(), rust_xlsxwriter::XlsxError> {
    let Some(v) = v else { return Ok(()) };
    match kind {
        ValueKind::Number if v.chars().filter(char::is_ascii_digit).count() <= EXCEL_DIGITS => match v.parse::<f64>() {
            Ok(n) if n.is_finite() => sheet.write_number(row, col, n).map(|_| ()),
            _ => sheet.write_string(row, col, v).map(|_| ()),
        },
        ValueKind::Bool => sheet.write_boolean(row, col, matches!(v.as_str(), "true" | "t" | "1")).map(|_| ()),
        _ => {
            let text: String = if v.len() > EXCEL_MAX_TEXT { v.chars().take(EXCEL_MAX_TEXT).collect() } else { v.clone() };
            sheet.write_string(row, col, text).map(|_| ())
        }
    }
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
    let out = Mutex::new(match format {
        ExportFormat::Csv => Out::Csv(Box::new(csv::Writer::from_writer(BufWriter::new(File::create(path)?)))),
        ExportFormat::Json => Out::Json { file: BufWriter::new(File::create(path)?), first: true },
        ExportFormat::Xlsx => {
            let mut book = rust_xlsxwriter::Workbook::new();
            // Rows go to a temporary file as they arrive, so big exports don't fill memory.
            book.add_worksheet_with_constant_memory();
            Out::Xlsx { book: Box::new(book), row: 0 }
        }
    });
    let columns: Mutex<Vec<ColumnMeta>> = Mutex::new(Vec::new());
    let count = Mutex::new(0u64);
    let failure: Mutex<Option<String>> = Mutex::new(None);

    let sink = |event: QueryEvent| {
        let fail = |e: String| *failure.lock().unwrap() = Some(e);
        match event {
            QueryEvent::Columns { columns: cols } => {
                match &mut *out.lock().unwrap() {
                    Out::Csv(w) => {
                        if let Err(e) = w.write_record(cols.iter().map(|c| c.name.as_str())) {
                            fail(e.to_string());
                        }
                    }
                    Out::Xlsx { book, row } => {
                        let bold = rust_xlsxwriter::Format::new().set_bold();
                        let sheet = book.worksheet_from_index(0).expect("one sheet");
                        let header = cols.iter().enumerate().try_for_each(|(i, c)| sheet.write_string_with_format(0, i as u16, &c.name, &bold).map(|_| ()));
                        let freeze = sheet.set_freeze_panes(1, 0).map(|_| ());
                        if let Err(e) = header.and(freeze) {
                            fail(e.to_string());
                        }
                        *row = 1;
                    }
                    Out::Json { .. } => {}
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
                        Out::Xlsx { book, row: at } => {
                            if *at >= EXCEL_MAX_ROWS {
                                Err(format!("Excel sheets hold at most {} rows. Filter the table first, or export as CSV.", EXCEL_MAX_ROWS - 1))
                            } else {
                                let sheet = book.worksheet_from_index(0).expect("one sheet");
                                let written = cols.iter().zip(row).enumerate().try_for_each(|(i, (c, v))| excel_cell(sheet, *at, i as u16, c.kind, v)).map_err(|e| e.to_string());
                                *at += 1;
                                written
                            }
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
        Out::Xlsx { mut book, .. } => book.save(path).map_err(|e| Error::Invalid(format!("Couldn't write the Excel file: {e}")))?,
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
    /// The text encoding it was read with, e.g. "UTF-8" or "windows-1254"; "xlsx" etc. for spreadsheets.
    pub encoding: String,
    /// A spreadsheet's sheets (empty for CSV), and the one previewed.
    pub sheets: Vec<String>,
    pub sheet: Option<String>,
}

/// Excel, LibreOffice and older Excel files, read with calamine.
pub fn is_spreadsheet(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "xlsx" | "xlsm" | "xlsb" | "xls" | "ods"))
}

/// A spreadsheet cell as the text Kiyi imports: whole numbers without ".0", dates as ISO.
fn sheet_text(v: &calamine::Data) -> String {
    use calamine::Data;
    match v {
        Data::Empty | Data::Error(_) => String::new(),
        Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        Data::Float(f) => f.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(dt) => {
            let (y, mo, d, h, mi, s, ms) = dt.to_ymd_hms_milli();
            if dt.is_duration() {
                format!("{:02}:{mi:02}:{s:02}", dt.as_f64().trunc() as i64 * 24 + h as i64)
            } else if (h, mi, s, ms) == (0, 0, 0, 0) {
                format!("{y:04}-{mo:02}-{d:02}")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            }
        }
    }
}

/// The rows of one sheet (the first when `sheet` is None), and the sheet names.
fn read_sheet(path: &Path, sheet: Option<&str>) -> Result<(Vec<String>, String, Vec<Vec<String>>)> {
    use calamine::Reader;
    let mut book = calamine::open_workbook_auto(path).map_err(|e| Error::Invalid(format!("Couldn't open the spreadsheet: {e}")))?;
    let names = book.sheet_names();
    let name = match sheet {
        Some(s) if names.iter().any(|n| n == s) => s.to_string(),
        Some(s) => return Err(Error::Invalid(format!("The file has no sheet named {s}."))),
        None => names.first().cloned().ok_or_else(|| Error::Invalid("The spreadsheet has no sheets.".into()))?,
    };
    let range = book.worksheet_range(&name).map_err(|e| Error::Invalid(format!("Couldn't read sheet {name}: {e}")))?;
    let rows = range.rows().map(|r| r.iter().map(sheet_text).collect()).collect();
    Ok((names, name, rows))
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

/// The first rows of the file. `encoding` overrides detection (CSV); `sheet` picks a spreadsheet's sheet.
pub fn preview(path: &Path, encoding_label: Option<&str>, sheet: Option<&str>) -> Result<CsvPreview> {
    if is_spreadsheet(path) {
        let (sheets, name, mut rows) = read_sheet(path, sheet)?;
        if rows.is_empty() {
            return Err(Error::Invalid(format!("Sheet {name} is empty.")));
        }
        let headers = rows.remove(0);
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("xlsx").to_ascii_lowercase();
        return Ok(CsvPreview { headers, total: rows.len() as u64, rows: rows.into_iter().take(20).collect(), encoding: ext, sheets, sheet: Some(name) });
    }
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
    Ok(CsvPreview { headers, rows: sample, total, encoding: enc.name().to_string(), sheets: vec![], sheet: None })
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
    /// Text encoding label ("UTF-8", "windows-1254"…); detected when missing. Ignored for spreadsheets.
    #[serde(default)]
    pub encoding: Option<String>,
    /// The spreadsheet sheet to import; the first when missing.
    #[serde(default)]
    pub sheet: Option<String>,
}

fn is_bool_type(data_type: &str) -> bool {
    let t = data_type.trim().to_ascii_lowercase();
    t.starts_with("bool") || t == "tinyint(1)" || t == "bit(1)"
}

/// A boolean as the database wants it, or None to leave an unrecognized value for the database to judge.
fn bool_text(v: &str, numeric: bool) -> Option<&'static str> {
    let truthy = matches!(v.trim().to_lowercase().as_str(), "true" | "t" | "yes" | "y" | "1" | "on" | "evet" | "doğru" | "dogru" | "e");
    let falsy = matches!(v.trim().to_lowercase().as_str(), "false" | "f" | "no" | "n" | "0" | "off" | "hayır" | "hayir" | "yanlış" | "yanlis" | "h");
    match (truthy, falsy, numeric) {
        (true, _, true) => Some("1"),
        (true, _, false) => Some("true"),
        (_, true, true) => Some("0"),
        (_, true, false) => Some("false"),
        _ => None,
    }
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
    // Booleans arrive as TRUE/FALSE from spreadsheets and as yes/no, 1/0 or words from people;
    // MySQL would quietly turn the text 'true' into 0, so they're normalized per database.
    let details = driver.table_details(plan.schema.as_deref(), &plan.table).await?;
    let bools: Vec<bool> = targets
        .iter()
        .map(|(_, c)| details.design.columns.iter().find(|x| &x.name == c).is_some_and(|x| is_bool_type(&x.data_type)))
        .collect();
    let mysql_like = d.is_mysql() || d.is_sqlite();
    // Both sources yield the file's rows as text, numbered from 1 for error messages.
    let mut records: Box<dyn Iterator<Item = (usize, Result<Vec<String>>)> + Send> = if is_spreadsheet(path) {
        let (_, _, rows) = read_sheet(path, plan.sheet.as_deref())?;
        Box::new(rows.into_iter().map(Ok).enumerate())
    } else {
        let csv = reader(path, encoding(plan.encoding.as_deref(), path)?)?;
        Box::new(csv.into_records().map(|r| r.map(|r| r.iter().map(str::to_string).collect()).map_err(|e| Error::Invalid(e.to_string()))).enumerate())
    };
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
            let record: Vec<&str> = record.iter().map(String::as_str).collect();
            rows.push(
                targets
                    .iter()
                    .zip(&bools)
                    .map(|((i, _), is_bool)| {
                        let v = record.get(*i).copied().unwrap_or("");
                        if v.is_empty() && plan.empty_as_null {
                            None
                        } else if *is_bool {
                            Some(bool_text(v, mysql_like).unwrap_or(v).to_string())
                        } else {
                            Some(v.to_string())
                        }
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
