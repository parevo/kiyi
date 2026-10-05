//! What the app knows about each database engine: how to present it, how to connect,
//! and friendly column types. The UI is built from this list, so adding an engine
//! means adding a driver and an entry here — no UI changes.

use serde::Serialize;

use crate::config::DbKind;

#[derive(Debug, Clone, Copy, Serialize)]
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
    /// What a non-developer sees: "Metin", "Tam sayı"…
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
        ty!("Metin", Text, "text", "İsim, açıklama, adres gibi her türlü yazı"),
        ty!("Kısa metin", Text, "varchar(255)", "En fazla 255 karakter"),
        ty!("Tam sayı", Number, "integer", "-2 milyar ile +2 milyar arası"),
        ty!("Büyük tam sayı", Number, "bigint", "Kimlik numaraları ve çok büyük sayılar"),
        ty!("Ondalık sayı", Decimal, "double precision", "Ölçüm gibi yaklaşık değerler"),
        ty!("Para / kesin sayı", Decimal, "numeric(12,2)", "Kuruşu kuruşuna doğru tutar"),
        ty!("Evet / Hayır", Boolean, "boolean", "Doğru ya da yanlış"),
        ty!("Tarih", Date, "date", "Gün, ay, yıl"),
        ty!("Tarih ve saat", DateTime, "timestamptz", "Saat dilimiyle birlikte an"),
        ty!("Saat", Time, "time", "Sadece saat"),
        ty!("Benzersiz kimlik (UUID)", Identifier, "uuid", "Tahmin edilemeyen kimlik"),
        ty!("JSON", Json, "jsonb", "Esnek, iç içe veri"),
        ty!("Dosya / ikili veri", Binary, "bytea", "Ham bayt dizisi"),
        ty!("Metin listesi", List, "text[]", "Birden fazla metin"),
    ]
}

fn mysql_types() -> Vec<TypeOption> {
    vec![
        ty!("Metin", Text, "text", "İsim, açıklama, adres gibi her türlü yazı"),
        ty!("Kısa metin", Text, "varchar(255)", "En fazla 255 karakter"),
        ty!("Tam sayı", Number, "int", "-2 milyar ile +2 milyar arası"),
        ty!("Büyük tam sayı", Number, "bigint", "Kimlik numaraları ve çok büyük sayılar"),
        ty!("Ondalık sayı", Decimal, "double", "Ölçüm gibi yaklaşık değerler"),
        ty!("Para / kesin sayı", Decimal, "decimal(12,2)", "Kuruşu kuruşuna doğru tutar"),
        ty!("Evet / Hayır", Boolean, "tinyint(1)", "Doğru ya da yanlış"),
        ty!("Tarih", Date, "date", "Gün, ay, yıl"),
        ty!("Tarih ve saat", DateTime, "datetime", "Tarih ve saat"),
        ty!("Saat", Time, "time", "Sadece saat"),
        ty!("Benzersiz kimlik (UUID)", Identifier, "char(36)", "Tahmin edilemeyen kimlik"),
        ty!("JSON", Json, "json", "Esnek, iç içe veri"),
        ty!("Dosya / ikili veri", Binary, "blob", "Ham bayt dizisi"),
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

pub fn drivers() -> Vec<DriverInfo> {
    vec![
        DriverInfo {
            id: "postgres",
            name: "PostgreSQL",
            kind: Some(DbKind::Postgres),
            default_port: 5432,
            url_example: "postgres://kullanici:sifre@sunucu:5432/veritabani",
            types: postgres_types(),
            recognise: postgres_recognise(),
        },
        DriverInfo {
            id: "mysql",
            name: "MySQL",
            kind: Some(DbKind::Mysql),
            default_port: 3306,
            url_example: "mysql://kullanici:sifre@sunucu:3306/veritabani",
            types: mysql_types(),
            recognise: mysql_recognise(),
        },
        DriverInfo {
            id: "mariadb",
            name: "MariaDB",
            kind: Some(DbKind::Mysql),
            default_port: 3306,
            url_example: "mariadb://kullanici:sifre@sunucu:3306/veritabani",
            types: mysql_types(),
            recognise: mysql_recognise(),
        },
        DriverInfo {
            id: "sqlite",
            name: "SQLite",
            kind: None,
            default_port: 0,
            url_example: "",
            types: vec![],
            recognise: vec![],
        },
        DriverInfo {
            id: "sqlserver",
            name: "SQL Server",
            kind: None,
            default_port: 1433,
            url_example: "",
            types: vec![],
            recognise: vec![],
        },
    ]
}
