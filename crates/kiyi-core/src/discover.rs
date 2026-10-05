//! Finds databases running on this machine, so a first-time user can connect in one click.
//!
//! Ports are only a hint; the protocol decides: a MySQL/MariaDB server greets the client
//! as soon as it connects, while PostgreSQL waits and answers an SSLRequest with 'S' or 'N'.

use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::DbKind;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDatabase {
    pub kind: DbKind,
    /// Catalog id: "postgres", "mysql" or "mariadb".
    pub driver: &'static str,
    pub host: &'static str,
    pub port: u16,
    pub version: Option<String>,
}

const PORTS: &[u16] = &[5432, 5433, 5434, 55432, 3306, 3307, 3308, 53306];
const STEP: Duration = Duration::from_millis(400);

/// PostgreSQL SSLRequest: length 8, code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

async fn probe(port: u16) -> Option<LocalDatabase> {
    let mut stream = timeout(STEP, TcpStream::connect(("127.0.0.1", port))).await.ok()?.ok()?;
    let mut buf = [0u8; 256];
    if let Ok(Ok(n)) = timeout(STEP, stream.read(&mut buf)).await {
        if n > 5 {
            // Handshake v10: 4-byte header, protocol byte 10, then a NUL-terminated version.
            let body = &buf[4..n];
            let version = body.get(1..).and_then(|v| v.split(|b| *b == 0).next()).map(|v| String::from_utf8_lossy(v).into_owned());
            let mariadb = version.as_deref().is_some_and(|v| v.to_ascii_lowercase().contains("mariadb"));
            return Some(LocalDatabase {
                kind: DbKind::Mysql,
                driver: if mariadb { "mariadb" } else { "mysql" },
                host: "localhost",
                port,
                version: version.map(|v| v.split('-').next().unwrap_or(&v).to_string()),
            });
        }
        return None;
    }
    // Silent server: ask the PostgreSQL way.
    stream.write_all(&SSL_REQUEST).await.ok()?;
    let mut answer = [0u8; 1];
    timeout(STEP, stream.read_exact(&mut answer)).await.ok()?.ok()?;
    matches!(answer[0], b'S' | b'N').then_some(LocalDatabase { kind: DbKind::Postgres, driver: "postgres", host: "localhost", port, version: None })
}

pub async fn local_databases() -> Vec<LocalDatabase> {
    let found = futures::future::join_all(PORTS.iter().map(|p| probe(*p))).await;
    found.into_iter().flatten().collect()
}
