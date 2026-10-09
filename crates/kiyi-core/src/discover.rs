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
    /// The Docker container publishing this port, when there is one.
    pub container: Option<String>,
}

const PORTS: &[u16] = &[5432, 5433, 5434, 55432, 3306, 3307, 3308, 53306];
const STEP: Duration = Duration::from_millis(400);

/// PostgreSQL SSLRequest: length 8, code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 4, 210, 22, 47];

async fn probe(port: u16) -> Option<LocalDatabase> {
    let mut stream = timeout(STEP, TcpStream::connect(("127.0.0.1", port))).await.ok()?.ok()?;
    let mut buf = [0u8; 256];
    if let Ok(Ok(n)) = timeout(STEP, stream.read(&mut buf)).await {
        // Handshake v10: 3-byte length, sequence 0, protocol byte 10, then a NUL-terminated version.
        // Anything else that greets first (SSH, SMTP…) isn't a database.
        if n > 5 && is_mysql_handshake(&buf[..n]) {
            let body = &buf[4..n];
            let version = body.get(1..).and_then(|v| v.split(|b| *b == 0).next()).map(|v| String::from_utf8_lossy(v).into_owned());
            let mariadb = version.as_deref().is_some_and(|v| v.to_ascii_lowercase().contains("mariadb"));
            return Some(LocalDatabase {
                kind: DbKind::Mysql,
                driver: if mariadb { "mariadb" } else { "mysql" },
                host: "localhost",
                port,
                version: version.map(|v| v.split('-').next().unwrap_or(&v).to_string()),
                container: None,
            });
        }
        return None;
    }
    // Silent server: ask the PostgreSQL way.
    stream.write_all(&SSL_REQUEST).await.ok()?;
    let mut answer = [0u8; 1];
    timeout(STEP, stream.read_exact(&mut answer)).await.ok()?.ok()?;
    matches!(answer[0], b'S' | b'N').then_some(LocalDatabase { kind: DbKind::Postgres, driver: "postgres", host: "localhost", port, version: None, container: None })
}

fn is_mysql_handshake(packet: &[u8]) -> bool {
    let len = u32::from_le_bytes([packet[0], packet[1], packet[2], 0]) as usize;
    packet[3] == 0 && packet[4] == 10 && len > 1 && len + 4 >= packet.len()
}

/// Ports published by running Docker containers, with the container's name. Docker not
/// installed or not running simply means none.
async fn docker_ports() -> Vec<(u16, String)> {
    let Some(docker) = crate::tunnel::find_tool("docker") else { return vec![] };
    let run = tokio::process::Command::new(docker).args(["ps", "--format", "{{.Names}}\t{{.Ports}}"]).kill_on_drop(true).output();
    match timeout(Duration::from_secs(3), run).await {
        Ok(Ok(out)) if out.status.success() => parse_docker_ps(&String::from_utf8_lossy(&out.stdout)),
        _ => vec![],
    }
}

/// `name<TAB>0.0.0.0:55432->5432/tcp, [::]:55432->5432/tcp` → `(55432, name)`.
fn parse_docker_ps(text: &str) -> Vec<(u16, String)> {
    let mut found: Vec<(u16, String)> = Vec::new();
    for line in text.lines() {
        let Some((name, ports)) = line.split_once('\t') else { continue };
        for mapping in ports.split(',') {
            let Some((host_side, _)) = mapping.trim().split_once("->") else { continue };
            let Some(port) = host_side.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) else { continue };
            if !found.iter().any(|(p, _)| *p == port) {
                found.push((port, name.trim().to_string()));
            }
        }
    }
    found
}

pub async fn local_databases() -> Vec<LocalDatabase> {
    let docker = docker_ports().await;
    let mut ports: Vec<u16> = PORTS.to_vec();
    ports.extend(docker.iter().map(|(p, _)| *p).filter(|p| !PORTS.contains(p)));
    let found = futures::future::join_all(ports.iter().map(|p| probe(*p))).await;
    found
        .into_iter()
        .flatten()
        .map(|mut db| {
            db.container = docker.iter().find(|(p, _)| *p == db.port).map(|(_, n)| n.clone());
            db
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_real_mysql_greeting_counts() {
        assert!(!super::is_mysql_handshake(b"SSH-2.0-OpenSSH_9.7\r\n"));
        assert!(!super::is_mysql_handshake(b"220 mail.example.com ESMTP\r\n"));
        let mut greeting = vec![0x4a, 0, 0, 0, 10];
        greeting.extend_from_slice(b"8.4.11\0");
        greeting.resize(0x4a + 4, 0);
        assert!(super::is_mysql_handshake(&greeting));
    }

    #[test]
    fn reads_docker_port_mappings() {
        let text = "dev-postgres-1\t0.0.0.0:55432->5432/tcp, [::]:55432->5432/tcp\nweb\t80/tcp\ncache\t127.0.0.1:6380->6379/tcp\n";
        assert_eq!(super::parse_docker_ps(text), [(55432, "dev-postgres-1".to_string()), (6380, "cache".to_string())]);
    }
}
