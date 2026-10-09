use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    Mysql,
    /// A database file; `database` holds its path.
    Sqlite,
}

impl DbKind {
    pub fn default_port(self) -> u16 {
        match self {
            DbKind::Postgres => 5432,
            DbKind::Mysql => 3306,
            DbKind::Sqlite => 0,
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
    /// The certificate must chain to a trusted CA, whatever host name it was issued for.
    /// Used under the hood for "Verify" through a tunnel, where the client connects to
    /// 127.0.0.1 and the server's name can't match.
    VerifyCa,
    VerifyFull,
}

/// How Kiyi signs in to the database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum DbAuth {
    /// A password stored in the keychain (or none).
    #[default]
    Password,
    /// Amazon RDS / Aurora IAM authentication: a short-lived token from the AWS CLI.
    #[serde(rename_all = "camelCase")]
    AwsIam { region: Option<String>, profile: Option<String> },
}

/// An SSH server to hop through before the bastion (`ProxyJump`). Signs in like the bastion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JumpHost {
    pub host: String,
    pub port: u16,
    pub user: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum SshAuth {
    /// Keys loaded in ssh-agent (also 1Password / Secretive agents).
    Agent,
    /// A private key file; its passphrase, if any, is stored in the keychain.
    Key { path: String },
    /// Password stored in the keychain.
    Password,
}

/// How to reach a database that isn't directly reachable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TunnelConfig {
    #[serde(rename_all = "camelCase")]
    Ssh {
        host: String,
        port: u16,
        user: String,
        auth: SshAuth,
        #[serde(default)]
        jump: Option<JumpHost>,
    },
    /// AWS Systems Manager port forwarding through an EC2 instance.
    #[serde(rename_all = "camelCase")]
    Ssm { target: String, region: Option<String>, profile: Option<String> },
    /// `kubectl port-forward` to a service or pod; the database port is the one inside the cluster.
    #[serde(rename_all = "camelCase")]
    Kubernetes { target: String, namespace: Option<String>, context: Option<String> },
    /// Google Cloud SQL through the Cloud SQL Auth Proxy (`project:region:instance`).
    #[serde(rename_all = "camelCase")]
    CloudSql { instance: String },
}

impl TunnelConfig {
    /// Tunnels that pick the database themselves, so the host field doesn't apply.
    pub fn ignores_host(&self) -> bool {
        matches!(self, TunnelConfig::Kubernetes { .. } | TunnelConfig::CloudSql { .. })
    }
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
    #[serde(default)]
    pub tunnel: Option<TunnelConfig>,
    /// A CA certificate (PEM) to trust for SSL, e.g. the Amazon RDS bundle.
    #[serde(default)]
    pub ssl_root_cert: Option<String>,
    #[serde(default)]
    pub auth: DbAuth,
}

impl ConnectionConfig {
    /// A Unix domain socket instead of TCP: the host is a path such as `/tmp` or `/var/run/mysqld/mysqld.sock`.
    pub fn socket_path(&self) -> Option<&str> {
        let h = self.host.trim();
        h.starts_with('/').then_some(h)
    }
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
    // Look at whole name parts, so "postgres" isn't mistaken for "stg".
    let parts: Vec<&str> = h.split(['.', '-', '_']).collect();
    let has = |words: &[&str]| parts.iter().any(|p| words.iter().any(|w| p == w || (w.len() > 3 && p.starts_with(w))));
    if h.starts_with('/') || h == "localhost" || h == "127.0.0.1" || h == "::1" || h.ends_with(".local") || h == "host.docker.internal" || !h.contains('.') {
        EnvTag::Local
    } else if has(&["staging", "stage", "stg", "dev", "develop", "test", "qa", "uat", "sandbox"]) {
        EnvTag::Staging
    } else {
        EnvTag::Production
    }
}

/// Parses `postgres://user:pass@host:5432/db?sslmode=require` style URLs.
pub fn parse_url(input: &str) -> Result<ParsedUrl> {
    let input = input.trim();
    if let Some(path) = input.strip_prefix("sqlite://").or_else(|| input.strip_prefix("sqlite:")) {
        let path = decode(path.split('?').next().unwrap_or(path));
        let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
        return Ok(ParsedUrl {
            config: ConnectionConfig {
                id: String::new(),
                name,
                kind: DbKind::Sqlite,
                host: String::new(),
                port: 0,
                user: String::new(),
                database: Some(path),
                ssl_mode: SslMode::Disable,
                env: EnvTag::Local,
                read_only: false,
                driver: Some("sqlite".into()),
                tunnel: None,
                ssl_root_cert: None,
                auth: DbAuth::Password,
            },
            password: None,
        });
    }
    let url = Url::parse(input).map_err(|e| Error::InvalidUrl(e.to_string()))?;
    let driver = match url.scheme() {
        "postgresql" => "postgres",
        other => other,
    };
    let kind = match url.scheme() {
        "postgres" | "postgresql" => DbKind::Postgres,
        "mysql" | "mariadb" => DbKind::Mysql,
        other => return Err(Error::InvalidUrl(format!("unsupported scheme: {other}"))),
    };
    // libpq-style `?host=/tmp` (or MySQL's `?socket=`) points at a Unix socket.
    let socket = url.query_pairs().find(|(k, v)| (k == "host" || k == "socket") && v.starts_with('/')).map(|(_, v)| v.into_owned());
    let host = socket.unwrap_or_else(|| {
        url.host_str().filter(|h| !h.is_empty()).unwrap_or("localhost").trim_matches(|c| c == '[' || c == ']').to_string()
    });
    let database = Some(decode(url.path().trim_start_matches('/'))).filter(|d| !d.is_empty());

    let mut ssl_mode = if guess_env(&host) == EnvTag::Local { SslMode::Disable } else { SslMode::Prefer };
    for (key, value) in url.query_pairs() {
        if key == "sslmode" || key == "ssl-mode" || key == "ssl_mode" {
            ssl_mode = match value.to_ascii_lowercase().as_str() {
                "disable" | "disabled" => SslMode::Disable,
                "require" | "required" => SslMode::Require,
                "verify-full" | "verify_identity" | "verify-identity" => SslMode::VerifyFull,
                "verify-ca" | "verify_ca" => SslMode::VerifyCa,
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
            tunnel: None,
            ssl_root_cert: url.query_pairs().find(|(k, _)| k == "sslrootcert" || k == "ssl-ca").map(|(_, v)| v.into_owned()),
            auth: DbAuth::Password,
        },
        password: url.password().map(decode),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_socket_and_ca_urls() {
        let p = parse_url("postgres:///shop?host=/var/run/postgresql").unwrap();
        assert_eq!(p.config.host, "/var/run/postgresql");
        assert_eq!(p.config.socket_path(), Some("/var/run/postgresql"));
        assert_eq!(p.config.env, EnvTag::Local);
        let p = parse_url("postgres://u@db.prod.example.com/shop?sslmode=verify-ca&sslrootcert=/certs/ca.pem").unwrap();
        assert_eq!(p.config.ssl_mode, SslMode::VerifyCa);
        assert_eq!(p.config.ssl_root_cert.as_deref(), Some("/certs/ca.pem"));
    }

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
    fn parses_sqlite_paths() {
        let p = parse_url("sqlite:///Users/me/My%20Data/shop.db").unwrap();
        assert_eq!(p.config.kind, DbKind::Sqlite);
        assert_eq!(p.config.database.as_deref(), Some("/Users/me/My Data/shop.db"));
        assert_eq!(p.config.name, "shop.db");
    }

    #[test]
    fn guesses_environments_from_name_parts() {
        assert_eq!(guess_env("postgres"), EnvTag::Local, "a bare service name");
        assert_eq!(guess_env("db.stg.example.com"), EnvTag::Staging);
        assert_eq!(guess_env("orders-staging.abc.rds.amazonaws.com"), EnvTag::Staging);
        assert_eq!(guess_env("postgres.prod.example.com"), EnvTag::Production);
        assert_eq!(guess_env("latest.example.com"), EnvTag::Production, "'test' inside a word isn't a test server");
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert!(parse_url("mongodb://localhost").is_err());
    }
}
