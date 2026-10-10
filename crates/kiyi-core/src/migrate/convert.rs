//! Turning a source value into one the target column accepts: the plan's steps first, then the
//! type rules every database disagrees on (booleans, time zones, binary, JSON, lengths, enums).

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

use super::{Otherwise, Step};
use crate::catalog::TypeCategory;
use crate::config::DbKind;
use crate::design::ColumnDesign;
use crate::dialect::Dialect;
use crate::types::ValueKind;

pub(crate) fn apply_steps(mut v: Option<String>, steps: &[Step]) -> Option<String> {
    for step in steps {
        v = match step {
            Step::Trim => v.map(|s| s.trim().to_string()),
            Step::Lower => v.map(|s| s.to_lowercase()),
            Step::Upper => v.map(|s| s.to_uppercase()),
            Step::Replace { find, with } if !find.is_empty() => v.map(|s| s.replace(find.as_str(), with)),
            Step::Replace { .. } => v,
            Step::Split { separator, part, rest } => v.map(|s| {
                let parts: Vec<&str> = if separator.is_empty() { s.split_whitespace().collect() } else { s.split(separator.as_str()).map(str::trim).filter(|p| !p.is_empty()).collect() };
                let i = part.saturating_sub(1);
                let glue = if separator.is_empty() { " " } else { separator.as_str() };
                if *rest { parts.get(i..).map(|p| p.join(glue)).unwrap_or_default() } else { parts.get(i).map(|p| p.to_string()).unwrap_or_default() }
            }),
            Step::Map { pairs, otherwise } => match &v {
                Some(s) => match pairs.iter().find(|p| p.from == *s).or_else(|| pairs.iter().find(|p| p.from.eq_ignore_ascii_case(s.trim()))) {
                    Some(p) => p.to.clone(),
                    None => match otherwise {
                        Otherwise::Keep => v,
                        Otherwise::Null => None,
                        Otherwise::Value { value } => Some(value.clone()),
                    },
                },
                None => match otherwise {
                    Otherwise::Value { value } => Some(value.clone()),
                    _ => None,
                },
            },
            Step::IfEmpty { value } => match &v {
                Some(s) if !s.trim().is_empty() => v,
                _ => value.clone(),
            },
        };
    }
    v
}

/// What the converter needs to know about a target column.
#[derive(Debug, Clone)]
pub(crate) struct TargetColumn {
    pub name: String,
    pub data_type: String,
    pub category: TypeCategory,
    pub nullable: bool,
    /// The database fills it in when no value is given (default, identity, computed).
    pub fills_itself: bool,
    pub enum_values: Vec<String>,
    pub max_len: Option<usize>,
}

impl TargetColumn {
    pub fn new(kind: DbKind, c: &ColumnDesign) -> Self {
        Self {
            name: c.name.clone(),
            data_type: c.data_type.clone(),
            category: crate::catalog::category(kind, &c.data_type),
            nullable: c.nullable,
            fills_itself: c.default.is_some() || c.auto_increment || c.generated,
            enum_values: c.enum_values.clone(),
            max_len: text_length(&c.data_type),
        }
    }

    pub fn required(&self) -> bool {
        !self.nullable && !self.fills_itself
    }
}

/// `varchar(255)`, `nvarchar(100)`, `character varying(50)`, `char(2)` → the character limit.
fn text_length(data_type: &str) -> Option<usize> {
    let t = data_type.to_ascii_lowercase();
    let textual = ["varchar", "char", "character", "nvarchar", "nchar", "varying"].iter().any(|w| t.contains(w));
    if !textual {
        return None;
    }
    let inside = t.split_once('(')?.1.split_once(')')?.0;
    inside.trim().parse().ok()
}

pub(crate) fn bool_value(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "true" | "t" | "yes" | "y" | "1" | "on" | "evet" | "doğru" | "dogru" | "e" | "-1" => Some(true),
        "false" | "f" | "no" | "n" | "0" | "off" | "hayır" | "hayir" | "yanlış" | "yanlis" | "h" => Some(false),
        _ => None,
    }
}

/// A moment in time from any database's text, with its UTC offset in seconds when it had one.
pub(crate) fn parse_datetime(s: &str) -> Option<(NaiveDateTime, Option<i32>)> {
    let s = s.trim();
    let date = NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()?;
    let rest = s[10..].trim_start_matches(['T', ' ']);
    if rest.is_empty() {
        return Some((date.and_hms_opt(0, 0, 0)?, None));
    }
    // Time: HH:MM[:SS[.fraction]]
    let time_end = rest.find(|c: char| !(c.is_ascii_digit() || c == ':' || c == '.')).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(time_end);
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    let mut parts = hms.split(':').map(|p| p.parse::<u32>().ok());
    let h = parts.next()??;
    let m = parts.next()??;
    let sec = parts.next().flatten().unwrap_or(0);
    let digits: String = frac.chars().take(9).collect();
    let nanos = if digits.is_empty() { 0 } else { digits.parse::<u32>().ok()? * 10u32.pow(9 - digits.len() as u32) };
    let at = date.and_hms_nano_opt(h, m, sec, nanos)?;
    let zone = zone.trim();
    let offset = match zone {
        "" => None,
        "Z" | "z" | "UTC" => Some(0),
        z if z.starts_with('+') || z.starts_with('-') => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let d: String = z[1..].chars().filter(char::is_ascii_digit).collect();
            let (hh, mm) = match d.len() {
                2 => (d.parse::<i32>().ok()?, 0),
                4 => (d[..2].parse::<i32>().ok()?, d[2..].parse::<i32>().ok()?),
                _ => return None,
            };
            Some(sign * (hh * 3600 + mm * 60))
        }
        _ => return None,
    };
    Some((at, offset))
}

fn format_datetime(at: NaiveDateTime, max_fraction: usize) -> String {
    let mut out = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", at.year(), at.month(), at.day(), at.hour(), at.minute(), at.second());
    let nanos = at.nanosecond();
    if nanos > 0 && max_fraction > 0 {
        let digits = format!("{nanos:09}");
        let kept = digits[..max_fraction.min(9)].trim_end_matches('0');
        if !kept.is_empty() {
            out.push('.');
            out.push_str(kept);
        }
    }
    out
}

fn format_offset(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let s = seconds.abs();
    format!("{sign}{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// Text of a moment as the target column wants it: with its offset where the type keeps one,
/// otherwise converted to UTC.
fn datetime_for(kind: DbKind, data_type: &str, at: NaiveDateTime, offset: Option<i32>) -> String {
    let t = data_type.to_ascii_lowercase();
    let keeps_zone = t.contains("with time zone") || t.starts_with("timestamptz") || t.starts_with("datetimeoffset");
    let utc = |at: NaiveDateTime| offset.map(|o| at - chrono::TimeDelta::seconds(o as i64)).unwrap_or(at);
    // Fractions of a second are cut to what the column keeps, rather than left for the database
    // to round (MySQL turns 30.5 seconds into 31).
    let declared = t.split_once('(').and_then(|(_, r)| r.split(')').next()).and_then(|n| n.trim().parse::<usize>().ok());
    let fraction = declared.unwrap_or(match kind {
        DbKind::Mysql => 0,
        DbKind::Sqlserver if t == "datetime" => 3,
        DbKind::Sqlserver if t == "smalldatetime" => 0,
        DbKind::Sqlserver => 7,
        _ => 6,
    });
    match (keeps_zone, offset) {
        (true, Some(o)) => {
            let sep = if kind == DbKind::Sqlserver { " " } else { "" };
            format!("{}{sep}{}", format_datetime(at, fraction), format_offset(o))
        }
        _ => format_datetime(utc(at), fraction),
    }
}

/// `{a,"b c",NULL}` (a Postgres array) as a JSON array.
fn pg_array_to_json(s: &str) -> Option<String> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut was_quoted, mut escape) = (false, false, false);
    for c in inner.chars() {
        match c {
            _ if escape => {
                cur.push(c);
                escape = false;
            }
            '\\' if quoted => escape = true,
            '"' => {
                quoted = !quoted;
                was_quoted = true;
            }
            ',' if !quoted => {
                items.push(if !was_quoted && cur == "NULL" { serde_json::Value::Null } else { serde_json::Value::String(std::mem::take(&mut cur)) });
                cur.clear();
                was_quoted = false;
            }
            '{' | '}' if !quoted => return None,
            _ => cur.push(c),
        }
    }
    if !inner.is_empty() {
        items.push(if !was_quoted && cur == "NULL" { serde_json::Value::Null } else { serde_json::Value::String(cur) });
    }
    Some(serde_json::Value::Array(items).to_string())
}

fn hex_of(v: &str) -> String {
    let t = v.trim();
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).or_else(|| t.strip_prefix("\\x")) {
        if h.len().is_multiple_of(2) && h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return h.to_ascii_lowercase();
        }
    }
    v.bytes().map(|b| format!("{b:02x}")).collect()
}

fn short(v: &str) -> String {
    if v.chars().count() > 40 { format!("“{}…”", v.chars().take(40).collect::<String>()) } else { format!("“{v}”") }
}

/// The value the target column should get, or why it can't take this one.
pub(crate) fn convert(kind: DbKind, col: &TargetColumn, v: Option<String>, source_kind: Option<ValueKind>) -> Result<Option<String>, String> {
    use TypeCategory::*;
    let Some(v) = v else { return Ok(None) };
    let textual = matches!(col.category, Text | Other | List);
    if !textual && v.trim().is_empty() {
        return Ok(None);
    }
    if !col.enum_values.is_empty() {
        return match col.enum_values.iter().find(|e| **e == v).or_else(|| col.enum_values.iter().find(|e| e.eq_ignore_ascii_case(v.trim()))) {
            Some(e) => Ok(Some(e.clone())),
            None => Err(format!("{} isn't one of the allowed values ({})", short(&v), col.enum_values.join(", "))),
        };
    }
    let out = match col.category {
        Boolean => {
            let b = bool_value(&v).ok_or_else(|| format!("{} isn't a yes/no value", short(&v)))?;
            let numeric = !matches!(kind, DbKind::Postgres);
            (if numeric { if b { "1" } else { "0" } } else if b { "true" } else { "false" }).to_string()
        }
        Number => {
            let t = v.trim();
            if let Some(b) = bool_value(t).filter(|_| source_kind == Some(ValueKind::Bool)) {
                (if b { "1" } else { "0" }).to_string()
            } else if t.strip_prefix('-').unwrap_or(t).bytes().all(|b| b.is_ascii_digit()) && !t.is_empty() && t != "-" {
                t.to_string()
            } else if let Some((whole, frac)) = t.split_once('.').filter(|(w, f)| f.bytes().all(|b| b == b'0') && w.strip_prefix('-').unwrap_or(w).bytes().all(|b| b.is_ascii_digit()) && !w.is_empty()) {
                let _ = frac;
                whole.to_string()
            } else {
                return Err(format!("{} isn't a whole number", short(&v)));
            }
        }
        Decimal => {
            let t = v.trim();
            // 1.234,56 and 1,5 are how much of the world writes numbers.
            let t = match (t.rfind(','), t.rfind('.')) {
                // 1,5 or 1.234,56: the comma is the decimal point.
                (Some(_), None) if t.matches(',').count() == 1 => t.replace(',', "."),
                (Some(c), Some(p)) if c > p => t.replace('.', "").replace(',', "."),
                // 1,234.50: commas group thousands.
                _ => t.replace(',', ""),
            };
            let ok = t.parse::<f64>().is_ok_and(f64::is_finite) && !t.to_ascii_lowercase().contains("inf") && !t.to_ascii_lowercase().contains("nan");
            if !ok {
                return Err(format!("{} isn't a number", short(&v)));
            }
            t
        }
        Date => {
            let (at, _) = parse_datetime(&v).ok_or_else(|| format!("{} isn't a date", short(&v)))?;
            at.date().format("%Y-%m-%d").to_string()
        }
        DateTime => {
            let (at, offset) = parse_datetime(&v).ok_or_else(|| format!("{} isn't a date and time", short(&v)))?;
            datetime_for(kind, &col.data_type, at, offset)
        }
        Json => {
            if source_kind == Some(ValueKind::Array) {
                pg_array_to_json(&v).unwrap_or(v)
            } else if serde_json::from_str::<serde_json::Value>(&v).is_ok() {
                v
            } else {
                return Err(format!("{} isn't valid JSON", short(&v)));
            }
        }
        Binary => format!("0x{}", hex_of(&v)),
        Identifier | Time => v.trim().to_string(),
        Text | Other | List => v,
    };
    if let Some(max) = col.max_len {
        let n = out.chars().count();
        if n > max {
            return Err(format!("{} is {n} characters long; {} holds {max}", short(&out), col.name));
        }
    }
    Ok(Some(out))
}

/// The converted value as a SQL literal for the target.
pub(crate) fn literal(d: Dialect, col: &TargetColumn, v: Option<&str>) -> String {
    match (v, col.category) {
        (None, _) => "NULL".into(),
        (Some(h), TypeCategory::Binary) => {
            let hex = h.trim_start_matches("0x");
            match d.kind {
                DbKind::Postgres => d.string(&format!("\\x{hex}")),
                DbKind::Sqlserver => format!("0x{hex}"),
                _ => format!("X'{hex}'"),
            }
        }
        (Some(v), _) => d.string(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::MapPair;

    fn col(kind: DbKind, data_type: &str) -> TargetColumn {
        TargetColumn::new(
            kind,
            &ColumnDesign {
                original: None,
                name: "c".into(),
                data_type: data_type.into(),
                nullable: true,
                default: None,
                primary_key: false,
                auto_increment: false,
                comment: None,
                generated: false,
                extra: None,
                enum_values: vec![],
            },
        )
    }

    fn conv(kind: DbKind, data_type: &str, v: &str) -> Result<Option<String>, String> {
        convert(kind, &col(kind, data_type), Some(v.into()), None)
    }

    #[test]
    fn steps_clean_split_and_map() {
        let s = |v: &str, steps: &[Step]| apply_steps(Some(v.into()), steps);
        assert_eq!(s("  Ada  ", &[Step::Trim, Step::Upper]).as_deref(), Some("ADA"));
        let first = Step::Split { separator: " ".into(), part: 1, rest: false };
        let last = Step::Split { separator: " ".into(), part: 2, rest: true };
        assert_eq!(s("Ada María Lovelace", std::slice::from_ref(&first)).as_deref(), Some("Ada"));
        assert_eq!(s("Ada María Lovelace", std::slice::from_ref(&last)).as_deref(), Some("María Lovelace"));
        assert_eq!(s("Cher", &[last]).as_deref(), Some(""));
        let map = Step::Map { pairs: vec![MapPair { from: "A".into(), to: Some("active".into()) }, MapPair { from: "X".into(), to: None }], otherwise: Otherwise::Value { value: "other".into() } };
        assert_eq!(s("a", std::slice::from_ref(&map)).as_deref(), Some("active"));
        assert_eq!(s("X", std::slice::from_ref(&map)), None);
        assert_eq!(s("zzz", std::slice::from_ref(&map)).as_deref(), Some("other"));
        assert_eq!(apply_steps(None, &[Step::IfEmpty { value: Some("n/a".into()) }]).as_deref(), Some("n/a"));
        assert_eq!(s("  ", &[Step::IfEmpty { value: None }]), None);
        assert_eq!(s("a-b", &[Step::Replace { find: "-".into(), with: "+".into() }]).as_deref(), Some("a+b"));
    }

    #[test]
    fn booleans_follow_the_target() {
        assert_eq!(conv(DbKind::Mysql, "tinyint(1)", "true").unwrap().as_deref(), Some("1"));
        assert_eq!(conv(DbKind::Postgres, "boolean", "Evet").unwrap().as_deref(), Some("true"));
        assert_eq!(conv(DbKind::Sqlserver, "bit", "f").unwrap().as_deref(), Some("0"));
        assert!(conv(DbKind::Postgres, "boolean", "maybe").unwrap_err().contains("yes/no"));
    }

    #[test]
    fn numbers_are_checked() {
        assert_eq!(conv(DbKind::Mysql, "int", " 42 ").unwrap().as_deref(), Some("42"));
        assert_eq!(conv(DbKind::Mysql, "int", "12.00").unwrap().as_deref(), Some("12"));
        assert!(conv(DbKind::Mysql, "int", "12.5").is_err());
        assert_eq!(conv(DbKind::Postgres, "numeric(12,2)", "1,5").unwrap().as_deref(), Some("1.5"));
        assert_eq!(conv(DbKind::Postgres, "numeric(12,2)", "1,234.50").unwrap().as_deref(), Some("1234.50"));
        assert_eq!(conv(DbKind::Postgres, "numeric(12,2)", "1.234,56").unwrap().as_deref(), Some("1234.56"));
        assert!(conv(DbKind::Postgres, "numeric(12,2)", "n/a").unwrap_err().contains("isn't a number"));
        assert_eq!(conv(DbKind::Postgres, "integer", "").unwrap(), None, "blank numbers become NULL");
    }

    #[test]
    fn moments_move_between_time_zone_rules() {
        // Postgres timestamptz text into MySQL DATETIME: converted to UTC.
        assert_eq!(conv(DbKind::Mysql, "datetime(6)", "2026-10-04 23:30:04.063391+03").unwrap().as_deref(), Some("2026-10-04 20:30:04.063391"));
        assert_eq!(conv(DbKind::Mysql, "datetime", "2026-10-04 23:30:04.9+03").unwrap().as_deref(), Some("2026-10-04 20:30:04"), "cut, not rounded");
        assert_eq!(conv(DbKind::Mysql, "datetime(3)", "2026-10-04 23:30:04.98765").unwrap().as_deref(), Some("2026-10-04 23:30:04.987"));
        // …and kept with its offset where the type has one.
        assert_eq!(conv(DbKind::Postgres, "timestamp with time zone", "2026-10-04T23:30:04+03:00").unwrap().as_deref(), Some("2026-10-04 23:30:04+03:00"));
        assert_eq!(conv(DbKind::Sqlserver, "datetimeoffset", "2026-10-04 23:30:04 +0300").unwrap().as_deref(), Some("2026-10-04 23:30:04 +03:00"));
        assert_eq!(conv(DbKind::Sqlserver, "datetime", "2026-10-04 20:00:04.0633900").unwrap().as_deref(), Some("2026-10-04 20:00:04.063"));
        assert_eq!(conv(DbKind::Sqlite, "DATETIME", "2026-10-04").unwrap().as_deref(), Some("2026-10-04 00:00:00"));
        assert_eq!(conv(DbKind::Mysql, "date", "2026-10-04 20:00:00").unwrap().as_deref(), Some("2026-10-04"));
        assert!(conv(DbKind::Mysql, "datetime", "yesterday").is_err());
        assert_eq!(conv(DbKind::Mysql, "datetime", "2026-01-01 00:30:00Z").unwrap().as_deref(), Some("2026-01-01 00:30:00"));
        assert_eq!(conv(DbKind::Mysql, "datetime", "2026-01-01 00:30:00+01").unwrap().as_deref(), Some("2025-12-31 23:30:00"));
    }

    #[test]
    fn json_binary_text_and_enums() {
        let arr = convert(DbKind::Mysql, &col(DbKind::Mysql, "json"), Some(r#"{a,"b c",NULL}"#.into()), Some(ValueKind::Array)).unwrap();
        assert_eq!(arr.as_deref(), Some(r#"["a","b c",null]"#));
        assert!(conv(DbKind::Mysql, "json", "not json").is_err());
        let bin = col(DbKind::Postgres, "bytea");
        let v = convert(DbKind::Postgres, &bin, Some("0xDEADbeef".into()), None).unwrap();
        assert_eq!(literal(Dialect::POSTGRES, &bin, v.as_deref()), "'\\xdeadbeef'");
        let mbin = col(DbKind::Mysql, "blob");
        assert_eq!(literal(Dialect::MYSQL, &mbin, Some("0x00ff")), "X'00ff'");
        assert!(conv(DbKind::Mysql, "varchar(3)", "abcd").unwrap_err().contains("holds 3"));
        assert_eq!(conv(DbKind::Mysql, "varchar(3)", "çok").unwrap().as_deref(), Some("çok"), "characters, not bytes");
        let mut e = col(DbKind::Postgres, "plan");
        e.enum_values = vec!["free".into(), "pro".into()];
        assert_eq!(convert(DbKind::Postgres, &e, Some("PRO".into()), None).unwrap().as_deref(), Some("pro"));
        assert!(convert(DbKind::Postgres, &e, Some("gold".into()), None).unwrap_err().contains("free, pro"));
    }
}
