use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    Mysql,
}

impl DbKind {
    pub fn default_port(self) -> u16 {
        match self {
            DbKind::Postgres => 5432,
            DbKind::Mysql => 3306,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvTag {
    Local,
    Staging,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyFull,
}

/// Everything about a connection except the password, which lives in the OS keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: Option<String>,
    pub ssl_mode: SslMode,
    pub env: EnvTag,
    pub read_only: bool,
    /// Catalog id ("postgres", "mariadb"…) for display; the driver itself follows `kind`.
    #[serde(default)]
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedUrl {
    pub config: ConnectionConfig,
    pub password: Option<String>,
}

fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// Best guess at the environment from the host name; the user can always change it.
pub fn guess_env(host: &str) -> EnvTag {
    let h = host.to_ascii_lowercase();
    if h == "localhost" || h == "127.0.0.1" || h == "::1" || h.ends_with(".local") || h == "host.docker.internal" {
        EnvTag::Local
    } else if h.contains("stag") || h.contains("stg") || h.contains("dev") || h.contains("test") {
        EnvTag::Staging
    } else {
        EnvTag::Production
    }
}

/// Parses `postgres://user:pass@host:5432/db?sslmode=require` style URLs.
pub fn parse_url(input: &str) -> Result<ParsedUrl> {
    let url = Url::parse(input.trim()).map_err(|e| Error::InvalidUrl(e.to_string()))?;
    let driver = match url.scheme() {
        "postgresql" => "postgres",
        other => other,
    };
    let kind = match url.scheme() {
        "postgres" | "postgresql" => DbKind::Postgres,
        "mysql" | "mariadb" => DbKind::Mysql,
        other => return Err(Error::InvalidUrl(format!("unsupported scheme: {other}"))),
    };
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost")
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    let database = Some(decode(url.path().trim_start_matches('/'))).filter(|d| !d.is_empty());

    let mut ssl_mode = if guess_env(&host) == EnvTag::Local { SslMode::Disable } else { SslMode::Prefer };
    for (key, value) in url.query_pairs() {
        if key == "sslmode" || key == "ssl-mode" || key == "ssl_mode" {
            ssl_mode = match value.to_ascii_lowercase().as_str() {
                "disable" | "disabled" => SslMode::Disable,
                "require" | "required" => SslMode::Require,
                "verify-full" | "verify_identity" | "verify-identity" => SslMode::VerifyFull,
                _ => SslMode::Prefer,
            };
        }
    }

    let env = guess_env(&host);
    let name = match &database {
        Some(db) => format!("{db} @ {host}"),
        None => host.clone(),
    };
    Ok(ParsedUrl {
        config: ConnectionConfig {
            id: String::new(),
            name,
            kind,
            port: url.port().unwrap_or(kind.default_port()),
            host,
            user: decode(url.username()),
            database,
            ssl_mode,
            env,
            read_only: env == EnvTag::Production,
            driver: Some(driver.to_string()),
        },
        password: url.password().map(decode),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_postgres_url() {
        let p = parse_url("postgresql://app%40corp:p%2Fss@db.prod.example.com:6543/shop?sslmode=require").unwrap();
        assert_eq!(p.config.kind, DbKind::Postgres);
        assert_eq!(p.config.user, "app@corp");
        assert_eq!(p.password.as_deref(), Some("p/ss"));
        assert_eq!(p.config.port, 6543);
        assert_eq!(p.config.database.as_deref(), Some("shop"));
        assert_eq!(p.config.ssl_mode, SslMode::Require);
        assert_eq!(p.config.env, EnvTag::Production);
        assert!(p.config.read_only);
    }

    #[test]
    fn parses_mysql_url_defaults() {
        let p = parse_url("mysql://root@localhost").unwrap();
        assert_eq!(p.config.kind, DbKind::Mysql);
        assert_eq!(p.config.port, 3306);
        assert_eq!(p.config.database, None);
        assert_eq!(p.config.env, EnvTag::Local);
        assert!(!p.config.read_only);
        assert_eq!(p.password, None);
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert!(parse_url("mongodb://localhost").is_err());
    }
}
