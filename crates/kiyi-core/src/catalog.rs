//! What the app knows about each database engine: how to present it, how to connect,
//! and friendly column types. The UI is built from this list, so adding an engine
//! means adding a driver and an entry here — no UI changes.

use serde::Serialize;

use crate::config::DbKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeCategory {
    Text,
    Number,
    Decimal,
    Boolean,
    Date,
    DateTime,
    Time,
    Identifier,
    Json,
    Binary,
    List,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeOption {
    /// What a non-developer sees: "Text", "Integer"…
    pub label: &'static str,
    pub category: TypeCategory,
    /// The SQL type written to the database.
    pub sql: &'static str,
    pub hint: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverInfo {
    /// Stable id, e.g. "postgres", "mariadb".
    pub id: &'static str,
    pub name: &'static str,
    /// Which driver implementation connects to it; `None` while not yet supported.
    pub kind: Option<DbKind>,
    pub default_port: u16,
    pub url_example: &'static str,
    /// Types offered when creating or changing a column.
    pub types: Vec<TypeOption>,
    /// Native type prefix → category, for recognising columns created elsewhere.
    pub recognise: Vec<(&'static str, TypeCategory)>,
}

macro_rules! ty {
    ($label:expr, $cat:ident, $sql:expr, $hint:expr) => {
        TypeOption { label: $label, category: TypeCategory::$cat, sql: $sql, hint: $hint }
    };
}

fn postgres_types() -> Vec<TypeOption> {
    vec![
        ty!("Text", Text, "text", "Names, descriptions, addresses: any length of text"),
        ty!("Short text", Text, "varchar(255)", "Up to 255 characters"),
        ty!("Integer", Number, "integer", "Whole numbers up to about ±2 billion"),
        ty!("Big integer", Number, "bigint", "IDs and very large whole numbers"),
        ty!("Decimal", Decimal, "double precision", "Approximate values such as measurements"),
        ty!("Money / exact number", Decimal, "numeric(12,2)", "Exact amounts, to the cent"),
        ty!("True / False", Boolean, "boolean", "A yes-or-no value"),
        ty!("Date", Date, "date", "A calendar day"),
        ty!("Date & time", DateTime, "timestamptz", "A moment in time, with time zone"),
        ty!("Time", Time, "time", "A time of day"),
        ty!("UUID", Identifier, "uuid", "A unique, unguessable identifier"),
        ty!("JSON", Json, "jsonb", "Flexible, nested data"),
        ty!("Binary", Binary, "bytea", "Raw bytes, such as files"),
        ty!("List of text", List, "text[]", "Several text values"),
    ]
}

fn mysql_types() -> Vec<TypeOption> {
    vec![
        ty!("Text", Text, "text", "Names, descriptions, addresses: any length of text"),
        ty!("Short text", Text, "varchar(255)", "Up to 255 characters"),
        ty!("Integer", Number, "int", "Whole numbers up to about ±2 billion"),
        ty!("Big integer", Number, "bigint", "IDs and very large whole numbers"),
        ty!("Decimal", Decimal, "double", "Approximate values such as measurements"),
        ty!("Money / exact number", Decimal, "decimal(12,2)", "Exact amounts, to the cent"),
        ty!("True / False", Boolean, "tinyint(1)", "A yes-or-no value"),
        ty!("Date", Date, "date", "A calendar day"),
        ty!("Date & time", DateTime, "datetime", "A date with a time of day"),
        ty!("Time", Time, "time", "A time of day"),
        ty!("UUID", Identifier, "char(36)", "A unique, unguessable identifier"),
        ty!("JSON", Json, "json", "Flexible, nested data"),
        ty!("Binary", Binary, "blob", "Raw bytes, such as files"),
    ]
}

fn postgres_recognise() -> Vec<(&'static str, TypeCategory)> {
    use TypeCategory::*;
    vec![
        ("boolean", Boolean),
        ("smallint", Number),
        ("integer", Number),
        ("bigint", Number),
        ("numeric", Decimal),
        ("real", Decimal),
        ("double precision", Decimal),
        ("money", Decimal),
        ("timestamp", DateTime),
        ("date", Date),
        ("time", Time),
        ("interval", Time),
        ("uuid", Identifier),
        ("json", Json),
        ("bytea", Binary),
        ("text[]", List),
        ("character varying[]", List),
        ("integer[]", List),
        ("text", Text),
        ("character", Text),
        ("citext", Text),
    ]
}

fn mysql_recognise() -> Vec<(&'static str, TypeCategory)> {
    use TypeCategory::*;
    vec![
        ("tinyint(1)", Boolean),
        ("tinyint", Number),
        ("smallint", Number),
        ("mediumint", Number),
        ("int", Number),
        ("bigint", Number),
        ("decimal", Decimal),
        ("float", Decimal),
        ("double", Decimal),
        ("datetime", DateTime),
        ("timestamp", DateTime),
        ("date", Date),
        ("time", Time),
        ("year", Number),
        ("json", Json),
        ("char(36)", Identifier),
        ("binary(16)", Identifier),
        ("varchar", Text),
        ("char", Text),
        ("text", Text),
        ("tinytext", Text),
        ("mediumtext", Text),
        ("longtext", Text),
        ("enum", Text),
        ("set", Text),
        ("blob", Binary),
        ("binary", Binary),
        ("varbinary", Binary),
    ]
}

fn sqlite_types() -> Vec<TypeOption> {
    vec![
        ty!("Text", Text, "TEXT", "Names, descriptions, addresses: any length of text"),
        ty!("Integer", Number, "INTEGER", "Whole numbers"),
        ty!("Decimal", Decimal, "REAL", "Approximate values such as measurements"),
        ty!("Money / exact number", Decimal, "NUMERIC", "Exact amounts"),
        ty!("True / False", Boolean, "BOOLEAN", "A yes-or-no value (stored as 1 or 0)"),
        ty!("Date", Date, "DATE", "A calendar day, as YYYY-MM-DD"),
        ty!("Date & time", DateTime, "DATETIME", "A moment, as YYYY-MM-DD HH:MM:SS"),
        ty!("JSON", Json, "JSON", "Flexible, nested data"),
        ty!("Binary", Binary, "BLOB", "Raw bytes, such as files"),
    ]
}

fn sqlserver_types() -> Vec<TypeOption> {
    vec![
        ty!("Text", Text, "nvarchar(max)", "Names, descriptions, addresses: any length of text"),
        ty!("Short text", Text, "nvarchar(255)", "Up to 255 characters"),
        ty!("Integer", Number, "int", "Whole numbers up to about ±2 billion"),
        ty!("Big integer", Number, "bigint", "IDs and very large whole numbers"),
        ty!("Decimal", Decimal, "float", "Approximate values such as measurements"),
        ty!("Money / exact number", Decimal, "decimal(12,2)", "Exact amounts, to the cent"),
        ty!("True / False", Boolean, "bit", "A yes-or-no value"),
        ty!("Date", Date, "date", "A calendar day"),
        ty!("Date & time", DateTime, "datetime2", "A date with a time of day"),
        ty!("Date & time with zone", DateTime, "datetimeoffset", "A moment in time, with its UTC offset"),
        ty!("Time", Time, "time", "A time of day"),
        ty!("UUID", Identifier, "uniqueidentifier", "A unique, unguessable identifier"),
        ty!("JSON", Json, "nvarchar(max)", "Flexible, nested data (stored as text)"),
        ty!("Binary", Binary, "varbinary(max)", "Raw bytes, such as files"),
    ]
}

fn sqlserver_recognise() -> Vec<(&'static str, TypeCategory)> {
    use TypeCategory::*;
    vec![
        ("bit", Boolean),
        ("tinyint", Number),
        ("smallint", Number),
        ("int", Number),
        ("bigint", Number),
        ("decimal", Decimal),
        ("numeric", Decimal),
        ("money", Decimal),
        ("smallmoney", Decimal),
        ("float", Decimal),
        ("real", Decimal),
        ("datetimeoffset", DateTime),
        ("datetime2", DateTime),
        ("datetime", DateTime),
        ("smalldatetime", DateTime),
        ("date", Date),
        ("time", Time),
        ("uniqueidentifier", Identifier),
        ("varbinary", Binary),
        ("binary", Binary),
        ("image", Binary),
        ("nvarchar", Text),
        ("varchar", Text),
        ("nchar", Text),
        ("char", Text),
        ("ntext", Text),
        ("text", Text),
        ("xml", Other),
    ]
}

fn sqlite_recognise() -> Vec<(&'static str, TypeCategory)> {
    use TypeCategory::*;
    vec![
        ("bool", Boolean),
        ("integer", Number),
        ("int", Number),
        ("bigint", Number),
        ("smallint", Number),
        ("real", Decimal),
        ("double", Decimal),
        ("float", Decimal),
        ("numeric", Decimal),
        ("decimal", Decimal),
        ("datetime", DateTime),
        ("timestamp", DateTime),
        ("date", Date),
        ("time", Time),
        ("json", Json),
        ("blob", Binary),
        ("text", Text),
        ("varchar", Text),
        ("char", Text),
        ("clob", Text),
    ]
}

/// What kind of value a column holds, from its native type, as the interface classifies it.
pub fn category(kind: DbKind, data_type: &str) -> TypeCategory {
    let t = data_type.trim().to_ascii_lowercase();
    drivers()
        .into_iter()
        .find(|d| d.kind == Some(kind))
        .and_then(|d| d.recognise.into_iter().find(|(prefix, _)| t.starts_with(prefix)).map(|(_, c)| c))
        .unwrap_or(TypeCategory::Other)
}

pub fn drivers() -> Vec<DriverInfo> {
    vec![
        DriverInfo {
            id: "postgres",
            name: "PostgreSQL",
            kind: Some(DbKind::Postgres),
            default_port: 5432,
            url_example: "postgres://user:password@host:5432/database",
            types: postgres_types(),
            recognise: postgres_recognise(),
        },
        DriverInfo {
            id: "mysql",
            name: "MySQL",
            kind: Some(DbKind::Mysql),
            default_port: 3306,
            url_example: "mysql://user:password@host:3306/database",
            types: mysql_types(),
            recognise: mysql_recognise(),
        },
        DriverInfo {
            id: "mariadb",
            name: "MariaDB",
            kind: Some(DbKind::Mysql),
            default_port: 3306,
            url_example: "mariadb://user:password@host:3306/database",
            types: mysql_types(),
            recognise: mysql_recognise(),
        },
        DriverInfo {
            id: "sqlite",
            name: "SQLite",
            kind: Some(DbKind::Sqlite),
            default_port: 0,
            url_example: "sqlite:///path/to/database.db",
            types: sqlite_types(),
            recognise: sqlite_recognise(),
        },
        DriverInfo {
            id: "sqlserver",
            name: "SQL Server",
            kind: Some(DbKind::Sqlserver),
            default_port: 1433,
            url_example: "sqlserver://user:password@host:1433/database",
            types: sqlserver_types(),
            recognise: sqlserver_recognise(),
        },
    ]
}
