//! Quoting and literal escaping. Every SQL string Kıyı generates goes through here,
//! so what the user reviews is exactly what runs — no hidden bind parameters.

use serde::{Deserialize, Serialize};

use crate::config::DbKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dialect {
    pub kind: DbKind,
    /// MySQL treats `\` as an escape in string literals unless NO_BACKSLASH_ESCAPES is set.
    /// Postgres sessions always run with `standard_conforming_strings = on`.
    pub backslash_escapes: bool,
}

impl Dialect {
    pub const POSTGRES: Dialect = Dialect { kind: DbKind::Postgres, backslash_escapes: false };
    pub const MYSQL: Dialect = Dialect { kind: DbKind::Mysql, backslash_escapes: true };

    pub fn is_mysql(self) -> bool {
        self.kind == DbKind::Mysql
    }

    pub fn ident(self, name: &str) -> String {
        match self.kind {
            DbKind::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
            DbKind::Mysql => format!("`{}`", name.replace('`', "``")),
        }
    }

    /// `schema.table`, or just `table` when no schema is given.
    pub fn table(self, schema: Option<&str>, name: &str) -> String {
        match schema {
            Some(s) if !s.is_empty() => format!("{}.{}", self.ident(s), self.ident(name)),
            _ => self.ident(name),
        }
    }

    pub fn string(self, value: &str) -> String {
        let mut out = String::with_capacity(value.len() + 2);
        out.push('\'');
        for c in value.chars() {
            match c {
                '\'' => out.push_str("''"),
                '\\' if self.backslash_escapes => out.push_str("\\\\"),
                '\0' if self.backslash_escapes => out.push_str("\\0"),
                c => out.push(c),
            }
        }
        out.push('\'');
        out
    }

    /// A cell value as SQL. Values travel as text and both servers coerce an untyped
    /// string literal to the column's type, so `'42'`, `'true'` and `'{a,b}'` all work.
    /// MySQL binary values (shown as `0x…`) become hex literals.
    pub fn value(self, value: Option<&str>, binary: bool) -> String {
        match value {
            None => "NULL".into(),
            Some(v) if binary && self.is_mysql() && is_hex_literal(v) => {
                if v.len() == 2 { "''".into() } else { format!("X'{}'", &v[2..]) }
            }
            Some(v) => self.string(v),
        }
    }

    pub fn ident_list(self, names: &[String]) -> String {
        names.iter().map(|n| self.ident(n)).collect::<Vec<_>>().join(", ")
    }
}

fn is_hex_literal(v: &str) -> bool {
    v.len().is_multiple_of(2) && (v.starts_with("0x") || v.starts_with("0X")) && v[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_identifiers() {
        assert_eq!(Dialect::POSTGRES.ident(r#"we"ird"#), r#""we""ird""#);
        assert_eq!(Dialect::MYSQL.ident("we`ird"), "`we``ird`");
        assert_eq!(Dialect::POSTGRES.table(Some("public"), "t"), r#""public"."t""#);
        assert_eq!(Dialect::MYSQL.table(None, "t"), "`t`");
    }

    #[test]
    fn escapes_literals() {
        assert_eq!(Dialect::POSTGRES.string(r"it's \n"), r"'it''s \n'");
        assert_eq!(Dialect::MYSQL.string(r"it's \'; DROP"), r"'it''s \\''; DROP'");
        let no_bs = Dialect { backslash_escapes: false, ..Dialect::MYSQL };
        assert_eq!(no_bs.string(r"a\b"), r"'a\b'");
        assert_eq!(Dialect::MYSQL.string("a\0b"), r"'a\0b'");
    }

    #[test]
    fn renders_values() {
        assert_eq!(Dialect::POSTGRES.value(None, false), "NULL");
        assert_eq!(Dialect::MYSQL.value(Some("0xdeadBEEF"), true), "X'deadBEEF'");
        assert_eq!(Dialect::MYSQL.value(Some("0xzz"), true), "'0xzz'");
        assert_eq!(Dialect::POSTGRES.value(Some(r"\xdead"), true), r"'\xdead'");
    }
}
