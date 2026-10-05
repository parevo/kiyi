//! Reaching databases that aren't directly reachable: an SSH bastion, or AWS SSM port
//! forwarding through an EC2 instance. Either way the database driver just connects to a
//! port on 127.0.0.1 and never knows a tunnel is involved.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::client;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Child;

use crate::config::{SshAuth, TunnelConfig};
use crate::error::{Error, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SSM_START_TIMEOUT: Duration = Duration::from_secs(40);

/// A running tunnel. Dropping it closes the tunnel.
pub struct Tunnel {
    pub local_port: u16,
    /// Human-readable steps for the connection test.
    pub steps: Vec<String>,
    _guard: Guard,
}

enum Guard {
    Task(tokio::task::JoinHandle<()>),
    #[allow(dead_code)] // held for kill_on_drop
    Process(Child),
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Guard::Task(t) = self {
            t.abort();
        }
    }
}

/// SSH server fingerprints seen before (trust on first use), persisted as JSON.
pub struct KnownHosts {
    path: PathBuf,
    hosts: Mutex<HashMap<String, String>>,
}

impl KnownHosts {
    pub fn load(path: PathBuf) -> Self {
        let hosts = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, hosts: Mutex::new(hosts) }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.hosts.lock().unwrap().get(key).cloned()
    }

    fn remember(&self, key: &str, fingerprint: &str) {
        let mut hosts = self.hosts.lock().unwrap();
        hosts.insert(key.to_string(), fingerprint.to_string());
        if let Ok(bytes) = serde_json::to_vec_pretty(&*hosts) {
            if let Some(dir) = self.path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&self.path, bytes);
        }
    }

    /// Forgets a server's fingerprint, e.g. after it was legitimately rebuilt.
    pub fn forget(&self, host: &str, port: u16) {
        let mut hosts = self.hosts.lock().unwrap();
        hosts.remove(&format!("{host}:{port}"));
        if let Ok(bytes) = serde_json::to_vec_pretty(&*hosts) {
            let _ = std::fs::write(&self.path, bytes);
        }
    }
}

pub async fn open(config: &TunnelConfig, secret: Option<&str>, target_host: &str, target_port: u16, known: &KnownHosts) -> Result<Tunnel> {
    match config {
        TunnelConfig::Ssh { host, port, user, auth } => open_ssh(host, *port, user, auth, secret, target_host, target_port, known).await,
        TunnelConfig::Ssm { target, region, profile } => open_ssm(target, region.as_deref(), profile.as_deref(), target_host, target_port).await,
    }
}

// ---- SSH

struct Checker {
    expected: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let fingerprint = match key {
            russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256),
            russh::keys::PublicKeyOrCertificate::Certificate(cert) => cert.public_key().fingerprint(HashAlg::Sha256),
        }
        .to_string();
        let ok = self.expected.as_deref().is_none_or(|e| e == fingerprint);
        *self.seen.lock().unwrap() = Some(fingerprint);
        Ok(ok)
    }
}

fn ssh_error(e: russh::Error, host: &str, port: u16) -> Error {
    let text = e.to_string();
    Error::Invalid(match &e {
        russh::Error::IO(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => format!("The SSH server at {host}:{port} refused the connection. Check the address and port."),
        russh::Error::IO(io) if io.kind() == std::io::ErrorKind::TimedOut => format!("The SSH server at {host}:{port} didn't respond. A firewall or security group may be blocking it."),
        _ if text.contains("lookup") || text.contains("nodename") => format!("The SSH host name {host} could not be resolved."),
        _ => format!("SSH: {text}"),
    })
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(rest),
        None => PathBuf::from(path),
    }
}

#[allow(clippy::too_many_arguments)]
async fn open_ssh(host: &str, port: u16, user: &str, auth: &SshAuth, secret: Option<&str>, target_host: &str, target_port: u16, known: &KnownHosts) -> Result<Tunnel> {
    let mut steps = Vec::new();
    let key = format!("{host}:{port}");
    let seen = Arc::new(Mutex::new(None));
    let checker = Checker { expected: known.get(&key), seen: seen.clone() };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        inactivity_timeout: None,
        ..Default::default()
    });

    let connected = tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, (host, port), checker))
        .await
        .map_err(|_| Error::Invalid(format!("The SSH server at {host}:{port} didn't respond in time.")))?;
    let mut session = match connected {
        Ok(s) => s,
        Err(e) => {
            let presented = seen.lock().unwrap().clone();
            return Err(match (known.get(&key), presented) {
                (Some(expected), Some(actual)) if expected != actual => Error::Invalid(format!(
                    "The SSH server's identity has changed (now {actual}). This can mean the server was rebuilt, or that someone is intercepting the connection. Kiyi won't connect until you trust the new identity."
                )),
                _ => ssh_error(e, host, port),
            });
        }
    };
    let fingerprint = seen.lock().unwrap().clone().unwrap_or_default();
    if known.get(&key).is_none() {
        known.remember(&key, &fingerprint);
    }
    steps.push(format!("SSH connection to {host}:{port} ({fingerprint})"));

    let denied = || Error::Invalid(format!("The SSH server didn't accept these credentials for {user}."));
    let result = match auth {
        SshAuth::Password => session
            .authenticate_password(user, secret.unwrap_or_default())
            .await
            .map_err(|e| ssh_error(e, host, port))?,
        SshAuth::Key { path } => {
            let path = expand_home(path);
            let private = russh::keys::load_secret_key(&path, secret).map_err(|e| {
                Error::Invalid(match e {
                    russh::keys::Error::KeyIsEncrypted => "The SSH key is protected by a passphrase. Enter it in the connection settings.".to_string(),
                    other => format!("Couldn't read the SSH key {}: {other}", path.display()),
                })
            })?;
            let hash = session.best_supported_rsa_hash().await.map_err(|e| ssh_error(e, host, port))?.flatten();
            session
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(private), hash))
                .await
                .map_err(|e| ssh_error(e, host, port))?
        }
        SshAuth::Agent => {
            #[cfg(unix)]
            let agent = russh::keys::agent::client::AgentClient::connect_env()
                .await
                .map_err(|_| Error::Invalid("No SSH agent is running (SSH_AUTH_SOCK isn't set). Use a key file instead.".into()))?;
            #[cfg(windows)]
            let agent = russh::keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
                .await
                .map_err(|_| Error::Invalid("The Windows OpenSSH agent isn't running. Start the \"OpenSSH Authentication Agent\" service, or use a key file.".into()))?;
            agent_auth(&mut session, user, agent, host, port).await?
        }
    };
    if !result.success() {
        return Err(denied());
    }
    steps.push(format!("Signed in as {user}"));

    let handle = Arc::new(session);
    // Prove the bastion can reach the database before handing out a port.
    let probe = handle
        .channel_open_direct_tcpip(target_host, target_port as u32, "127.0.0.1", 0)
        .await
        .map_err(|_| Error::Invalid(format!("The SSH server can't reach {target_host}:{target_port}. Check the database host from the bastion's point of view, and its security group.")))?;
    drop(probe);
    steps.push(format!("Forwarding to {target_host}:{target_port}"));

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_port = listener.local_addr()?.port();
    let target_host = target_host.to_string();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let (handle, target_host) = (handle.clone(), target_host.clone());
            tokio::spawn(async move {
                match handle.channel_open_direct_tcpip(target_host, target_port as u32, "127.0.0.1", 0).await {
                    Ok(channel) => {
                        let mut stream = channel.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                    }
                    Err(e) => tracing::warn!("ssh forward failed: {e}"),
                }
            });
        }
    });
    Ok(Tunnel { local_port, steps, _guard: Guard::Task(task) })
}

/// Tries each key the agent holds until the server accepts one.
async fn agent_auth<S>(
    session: &mut client::Handle<Checker>,
    user: &str,
    mut agent: russh::keys::agent::client::AgentClient<S>,
    host: &str,
    port: u16,
) -> Result<russh::client::AuthResult>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let identities = agent.request_identities().await.map_err(|e| Error::Invalid(format!("SSH agent: {e}")))?;
    let mut outcome = None;
    for identity in identities {
        let russh::keys::agent::AgentIdentity::PublicKey { key, .. } = identity else { continue };
        let hash = session.best_supported_rsa_hash().await.map_err(|e| ssh_error(e, host, port))?.flatten();
        match session.authenticate_publickey_with(user, key, hash, &mut agent).await {
            Ok(r) if r.success() => return Ok(r),
            Ok(r) => outcome = Some(r),
            Err(e) => return Err(Error::Invalid(format!("SSH agent: {e}"))),
        }
    }
    outcome.ok_or_else(|| Error::Invalid("The SSH agent has no keys loaded.".into()))
}

// ---- AWS SSM

/// GUI apps on macOS don't inherit the shell's PATH, so look where installers put these.
fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/usr/local/sessionmanagerplugin/bin"] {
        dirs.push(PathBuf::from(extra));
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/bin"));
    }
    if cfg!(windows) {
        dirs.push(PathBuf::from(r"C:\Program Files\Amazon\AWSCLIV2"));
        dirs.push(PathBuf::from(r"C:\Program Files\Amazon\SessionManagerPlugin\bin"));
    }
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// The `aws ssm start-session` arguments for forwarding to `host:port` through `target`.
pub fn ssm_args(target: &str, region: Option<&str>, profile: Option<&str>, host: &str, port: u16, local_port: u16) -> Vec<String> {
    let parameters = serde_json::json!({ "host": [host], "portNumber": [port.to_string()], "localPortNumber": [local_port.to_string()] });
    let mut args = vec![
        "ssm".into(),
        "start-session".into(),
        "--target".into(),
        target.into(),
        "--document-name".into(),
        "AWS-StartPortForwardingSessionToRemoteHost".into(),
        "--parameters".into(),
        parameters.to_string(),
    ];
    if let Some(r) = region.filter(|r| !r.is_empty()) {
        args.extend(["--region".into(), r.into()]);
    }
    if let Some(p) = profile.filter(|p| !p.is_empty()) {
        args.extend(["--profile".into(), p.into()]);
    }
    args
}

async fn open_ssm(target: &str, region: Option<&str>, profile: Option<&str>, host: &str, port: u16) -> Result<Tunnel> {
    let aws = find_tool("aws").ok_or_else(|| Error::Invalid("AWS SSM needs the AWS CLI. Install it from aws.amazon.com/cli and sign in with `aws configure` or `aws sso login`.".into()))?;
    let plugin = find_tool("session-manager-plugin")
        .ok_or_else(|| Error::Invalid("AWS SSM needs the Session Manager plugin. Install it from the AWS documentation (\"Install the Session Manager plugin for the AWS CLI\").".into()))?;

    // Reserve a free port for the plugin to listen on.
    let local_port = TcpListener::bind("127.0.0.1:0").await?.local_addr()?.port();
    let mut path = vec![plugin.parent().map(Path::to_path_buf).unwrap_or_default()];
    path.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());

    let mut child = tokio::process::Command::new(&aws)
        .args(ssm_args(target, region, profile, host, port, local_port))
        .env("PATH", std::env::join_paths(path).unwrap_or_default())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Invalid(format!("Couldn't start the AWS CLI: {e}")))?;

    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = stderr.read_to_string(&mut err).await;
            }
            let err = err.trim();
            return Err(Error::Invalid(if err.contains("TargetNotConnected") {
                format!("The instance {target} isn't connected to Systems Manager. Check that the SSM agent is running and the instance role allows it.")
            } else if err.contains("ExpiredToken") || err.contains("Unable to locate credentials") || err.contains("SSO") {
                "Your AWS credentials are missing or expired. Run `aws sso login` (or `aws configure`) and try again.".into()
            } else if err.is_empty() {
                format!("The AWS CLI exited ({status}).")
            } else {
                format!("AWS: {err}")
            }));
        }
        if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
            break;
        }
        if started.elapsed() > SSM_START_TIMEOUT {
            return Err(Error::Invalid("The SSM session didn't start in time.".into()));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let steps = vec![format!("SSM session through {target}"), format!("Forwarding to {host}:{port}")];
    Ok(Tunnel { local_port, steps, _guard: Guard::Process(child) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssm_command_line() {
        let args = ssm_args("i-0abc", Some("eu-west-1"), None, "db.internal", 5432, 61000);
        assert_eq!(&args[..4], ["ssm", "start-session", "--target", "i-0abc"]);
        let params: serde_json::Value = serde_json::from_str(&args[7]).unwrap();
        assert_eq!(params["host"][0], "db.internal");
        assert_eq!(params["localPortNumber"][0], "61000");
        assert_eq!(&args[8..], ["--region", "eu-west-1"]);
    }
}
