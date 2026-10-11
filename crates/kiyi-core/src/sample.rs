//! The sample database offered on first launch, so there is something to try before
//! connecting a real one: a small store with customers, products and orders, as a SQLite file.

use crate::config::{ConnectionConfig, DbAuth, DbKind, EnvTag, SslMode};

pub const NAME: &str = "Acme Store (sample)";
pub const FILE_NAME: &str = "Acme Store sample.db";

/// The seed's statements, which are separated by blank lines; comment-only blocks are skipped.
pub fn statements() -> Vec<String> {
    include_str!("sample.sql")
        .split("\n\n")
        // Comment lines above a statement explain it; they aren't part of it.
        .map(|chunk| chunk.lines().skip_while(|l| l.trim().is_empty() || l.trim_start().starts_with("--")).collect::<Vec<_>>().join("\n"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub fn config(path: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: String::new(),
        name: NAME.into(),
        kind: DbKind::Sqlite,
        host: String::new(),
        port: 0,
        user: String::new(),
        database: Some(path.into()),
        ssl_mode: SslMode::Disable,
        env: EnvTag::Local,
        read_only: false,
        driver: Some("sqlite".into()),
        tunnel: None,
        ssl_root_cert: None,
        auth: DbAuth::Password,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn seed_splits_into_statements() {
        let s = super::statements();
        assert!(s[0].starts_with("CREATE TABLE customers"), "{}", s[0]);
        assert!(s.iter().any(|x| x.starts_with("CREATE TRIGGER") && x.ends_with("END;")));
        assert!(s.iter().all(|x| !x.starts_with("--")));
    }
}
