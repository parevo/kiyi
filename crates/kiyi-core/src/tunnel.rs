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

use crate::config::{JumpHost, SshAuth, TunnelConfig};
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
        TunnelConfig::Ssh { host, port, user, auth, jump } => {
            open_ssh(host, *port, user, auth, jump.as_ref(), secret, target_host, target_port, known).await
        }
        TunnelConfig::Ssm { target, region, profile } => open_ssm(target, region.as_deref(), profile.as_deref(), target_host, target_port).await,
        TunnelConfig::Kubernetes { target, namespace, context } => open_kubernetes(target, namespace.as_deref(), context.as_deref(), target_port).await,
        TunnelConfig::CloudSql { instance } => open_cloud_sql(instance).await,
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

/// A key path as people paste it: Windows "Copy as path" adds quotes, and `~` means home
/// (Windows sets USERPROFILE rather than HOME).
fn key_path(path: &str) -> PathBuf {
    let path = path.trim();
    let path = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')).or_else(|| path.strip_prefix('\'').and_then(|p| p.strip_suffix('\''))).unwrap_or(path);
    let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

/// Signs in to one SSH server, directly or through an already open session (a jump host).
#[allow(clippy::too_many_arguments)]
async fn ssh_session(
    via: Option<&client::Handle<Checker>>,
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuth,
    secret: Option<&str>,
    known: &KnownHosts,
    steps: &mut Vec<String>,
) -> Result<client::Handle<Checker>> {
    let key = format!("{host}:{port}");
    let seen = Arc::new(Mutex::new(None));
    let checker = Checker { expected: known.get(&key), seen: seen.clone() };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        inactivity_timeout: None,
        ..Default::default()
    });

    let connected = match via {
        None => tokio::time::timeout(CONNECT_TIMEOUT, client::connect(config, (host, port), checker)).await,
        Some(jump) => {
            let channel = jump
                .channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0)
                .await
                .map_err(|_| Error::Invalid(format!("The jump server can't reach {host}:{port}. Check the address from the jump server's point of view.")))?;
            tokio::time::timeout(CONNECT_TIMEOUT, client::connect_stream(config, channel.into_stream(), checker)).await
        }
    }
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

    let denied = || Error::Invalid(format!("The SSH server at {host} didn't accept these credentials for {user}."));
    let result = match auth {
        SshAuth::Password => session
            .authenticate_password(user, secret.unwrap_or_default())
            .await
            .map_err(|e| ssh_error(e, host, port))?,
        SshAuth::Key { path } => {
            let path = key_path(path);
            if !path.is_file() {
                return Err(Error::Invalid(format!("There's no key file at {}. Choose the .pem or private key file again.", path.display())));
            }
            let private = russh::keys::load_secret_key(&path, secret.filter(|s| !s.is_empty())).map_err(|e| {
                Error::Invalid(match e {
                    russh::keys::Error::KeyIsEncrypted => "The SSH key is protected by a passphrase. Enter it in the connection settings.".to_string(),
                    russh::keys::Error::Decode(_) | russh::keys::Error::CouldNotReadKey => {
                        format!("{} isn't a private key Kiyi can read. Use the private key (for AWS, the .pem you downloaded), not the .pub file.", path.display())
                    }
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
    steps.push(format!("Signed in to {host} as {user}"));
    Ok(session)
}

#[allow(clippy::too_many_arguments)]
async fn open_ssh(
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuth,
    jump: Option<&JumpHost>,
    secret: Option<&str>,
    target_host: &str,
    target_port: u16,
    known: &KnownHosts,
) -> Result<Tunnel> {
    let mut steps = Vec::new();
    // The jump host signs in the same way as the bastion (same key, agent or password).
    let jump_session = match jump.filter(|j| !j.host.trim().is_empty()) {
        Some(j) => {
            let user = if j.user.trim().is_empty() { user } else { j.user.trim() };
            Some(Arc::new(ssh_session(None, j.host.trim(), if j.port == 0 { 22 } else { j.port }, user, auth, secret, known, &mut steps).await?))
        }
        None => None,
    };
    let handle = Arc::new(ssh_session(jump_session.as_deref(), host, port, user, auth, secret, known, &mut steps).await?);

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
        // Keeps the jump session alive for as long as the bastion session rides on it.
        let _jump = jump_session;
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

/// Where installers put command-line tools. GUI apps on macOS don't inherit the shell's PATH.
#[cfg(not(windows))]
const TOOL_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/usr/local/sessionmanagerplugin/bin", "/Applications/Docker.app/Contents/Resources/bin"];
#[cfg(windows)]
const TOOL_DIRS: &[&str] = &[
    r"C:\Program Files\Amazon\AWSCLIV2",
    r"C:\Program Files\Amazon\SessionManagerPlugin\bin",
    r"C:\Program Files\Docker\Docker\resources\bin",
    r"C:\Program Files (x86)\Google\Cloud SDK\google-cloud-sdk\bin",
];

/// Finds a command-line tool, looking beyond PATH.
pub(crate) fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(TOOL_DIRS.iter().map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        for sub in [".local/bin", "google-cloud-sdk/bin", ".rd/bin", ".docker/bin", "bin"] {
            dirs.push(home.join(sub));
        }
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

/// Runs a port-forwarding command (AWS CLI, kubectl, Cloud SQL proxy) until its local port
/// accepts connections. `explain` turns its error output into advice.
async fn spawn_forwarder(program: &Path, args: Vec<String>, local_port: u16, explain: impl Fn(&str) -> Option<String>) -> Result<Child> {
    // Helpers the program runs (the Session Manager plugin, gke-gcloud-auth-plugin) must be
    // findable even though GUI apps on macOS don't get the shell's PATH.
    let mut path = vec![program.parent().map(Path::to_path_buf).unwrap_or_default()];
    path.extend(TOOL_DIRS.iter().map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        path.push(Path::new(&home).join("google-cloud-sdk/bin"));
        path.push(Path::new(&home).join(".local/bin"));
    }
    path.extend(std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());

    let name = program.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut child = tokio::process::Command::new(program)
        .args(&args)
        .env("PATH", std::env::join_paths(path).unwrap_or_default())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::Invalid(format!("Couldn't start {name}: {e}")))?;

    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut err = String::new();
            use tokio::io::AsyncReadExt;
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut err).await;
            }
            if err.trim().is_empty() {
                if let Some(mut stdout) = child.stdout.take() {
                    let _ = stdout.read_to_string(&mut err).await;
                }
            }
            let err = err.trim();
            return Err(Error::Invalid(explain(err).unwrap_or_else(|| {
                if err.is_empty() {
                    format!("{name} exited ({status}).")
                } else {
                    format!("{name}: {}", err.lines().last().unwrap_or(err))
                }
            })));
        }
        if TcpStream::connect(("127.0.0.1", local_port)).await.is_ok() {
            // These tools log a line per connection; an unread pipe would fill up and freeze the tunnel.
            if let Some(out) = child.stdout.take() {
                tokio::spawn(drain(out));
            }
            if let Some(err) = child.stderr.take() {
                tokio::spawn(drain(err));
            }
            return Ok(child);
        }
        if started.elapsed() > SSM_START_TIMEOUT {
            return Err(Error::Invalid(format!("{name} didn't open the tunnel in time.")));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn drain(mut pipe: impl tokio::io::AsyncRead + Unpin) {
    let _ = tokio::io::copy(&mut pipe, &mut tokio::io::sink()).await;
}

async fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0").await?.local_addr()?.port())
}

async fn open_ssm(target: &str, region: Option<&str>, profile: Option<&str>, host: &str, port: u16) -> Result<Tunnel> {
    let aws = find_tool("aws").ok_or_else(|| Error::Invalid("AWS SSM needs the AWS CLI. Install it from aws.amazon.com/cli and sign in with `aws configure` or `aws sso login`.".into()))?;
    find_tool("session-manager-plugin")
        .ok_or_else(|| Error::Invalid("AWS SSM needs the Session Manager plugin. Install it from the AWS documentation (\"Install the Session Manager plugin for the AWS CLI\").".into()))?;
    let local_port = free_port().await?;
    let child = spawn_forwarder(&aws, ssm_args(target, region, profile, host, port, local_port), local_port, |err| {
        if err.contains("TargetNotConnected") {
            Some(format!("The instance {target} isn't connected to Systems Manager. Check that the SSM agent is running and the instance role allows it."))
        } else {
            aws_credentials_problem(err)
        }
    })
    .await?;
    let steps = vec![format!("SSM session through {target}"), format!("Forwarding to {host}:{port}")];
    Ok(Tunnel { local_port, steps, _guard: Guard::Process(child) })
}

fn aws_credentials_problem(err: &str) -> Option<String> {
    (err.contains("ExpiredToken") || err.contains("Unable to locate credentials") || err.contains("SSO") || err.contains("could not be found"))
        .then(|| "Your AWS credentials are missing or expired. Run `aws sso login` (or `aws configure`) and try again. If you use a named profile, enter it in the connection settings.".into())
}

/// The `kubectl port-forward` arguments. A bare name means a service.
pub fn kubernetes_args(target: &str, namespace: Option<&str>, context: Option<&str>, remote_port: u16, local_port: u16) -> Vec<String> {
    let target = target.trim();
    let target = if target.contains('/') { target.to_string() } else { format!("svc/{target}") };
    let mut args = vec!["port-forward".into(), "--address".into(), "127.0.0.1".into(), target, format!("{local_port}:{remote_port}")];
    if let Some(ns) = namespace.map(str::trim).filter(|n| !n.is_empty()) {
        args.extend(["--namespace".into(), ns.into()]);
    }
    if let Some(ctx) = context.map(str::trim).filter(|c| !c.is_empty()) {
        args.extend(["--context".into(), ctx.into()]);
    }
    args
}

async fn open_kubernetes(target: &str, namespace: Option<&str>, context: Option<&str>, port: u16) -> Result<Tunnel> {
    let kubectl = find_tool("kubectl").ok_or_else(|| Error::Invalid("Kubernetes needs kubectl. Install it and make sure `kubectl get pods` works in a terminal.".into()))?;
    let local_port = free_port().await?;
    let args = kubernetes_args(target, namespace, context, port, local_port);
    let shown = args[3].clone();
    let child = spawn_forwarder(&kubectl, args, local_port, |err| {
        if err.contains("NotFound") || err.contains("not found") {
            Some(format!("Kubernetes couldn't find {shown}. Check the name and the namespace (`kubectl get svc -n <namespace>`)."))
        } else if err.contains("localhost:8080") {
            Some("kubectl has no cluster set up (no kubeconfig). Check that `kubectl get pods` works in a terminal, or enter the context to use.".into())
        } else if err.contains("context") && err.contains("does not exist") {
            Some("That Kubernetes context doesn't exist. `kubectl config get-contexts` lists them.".into())
        } else if err.contains("Unable to connect to the server") || err.contains("must be logged in") || err.contains("Unauthorized") || err.contains("credentials") {
            Some("kubectl can't reach the cluster or isn't signed in. Check `kubectl get pods` in a terminal, and the context.".into())
        } else if err.contains("does not have a port") || err.contains("doesn't have a port") {
            Some(format!("{shown} has no port {port}. Use the database port inside the cluster."))
        } else {
            None
        }
    })
    .await?;
    let steps = vec![format!("kubectl port-forward {shown}"), format!("Forwarding to port {port} in the cluster")];
    Ok(Tunnel { local_port, steps, _guard: Guard::Process(child) })
}

async fn open_cloud_sql(instance: &str) -> Result<Tunnel> {
    let instance = instance.trim();
    if instance.split(':').count() != 3 {
        return Err(Error::Invalid("Use the instance connection name, project:region:instance (shown on the instance's overview page).".into()));
    }
    let proxy = find_tool("cloud-sql-proxy").ok_or_else(|| {
        Error::Invalid("Cloud SQL needs the Cloud SQL Auth Proxy. Install it (\"cloud-sql-proxy\" in the Google Cloud docs) and sign in with `gcloud auth application-default login`.".into())
    })?;
    let local_port = free_port().await?;
    let args = vec![instance.to_string(), "--address".into(), "127.0.0.1".into(), "--port".into(), local_port.to_string()];
    let child = spawn_forwarder(&proxy, args, local_port, |err| {
        if err.contains("could not find default credentials") || err.contains("credentials") || err.contains("invalid_grant") {
            Some("Google Cloud credentials are missing or expired. Run `gcloud auth application-default login` and try again.".into())
        } else if err.contains("NOT_AUTHORIZED") || err.contains("403") {
            Some(format!("Your Google account can't connect to {instance}. It needs the Cloud SQL Client role."))
        } else if err.contains("404") || err.contains("does not exist") {
            Some(format!("Cloud SQL instance {instance} wasn't found. Check the instance connection name."))
        } else {
            None
        }
    })
    .await?;
    let steps = vec![format!("Cloud SQL Auth Proxy to {instance}")];
    Ok(Tunnel { local_port, steps, _guard: Guard::Process(child) })
}

/// The region in an RDS endpoint (`name.abc123.eu-west-1.rds.amazonaws.com`).
pub fn rds_region(host: &str) -> Option<&str> {
    let parts: Vec<&str> = host.trim().trim_end_matches('.').split('.').collect();
    let i = parts.iter().position(|p| *p == "rds")?;
    (i >= 1 && parts.get(i + 1) == Some(&"amazonaws")).then(|| parts[i - 1])
}

/// A 15-minute RDS IAM authentication token, used as the password.
pub async fn rds_auth_token(host: &str, port: u16, user: &str, region: Option<&str>, profile: Option<&str>) -> Result<String> {
    let aws = find_tool("aws").ok_or_else(|| Error::Invalid("IAM sign-in needs the AWS CLI. Install it from aws.amazon.com/cli and sign in with `aws configure` or `aws sso login`.".into()))?;
    let region = region.map(str::trim).filter(|r| !r.is_empty()).or_else(|| rds_region(host));
    let mut cmd = tokio::process::Command::new(&aws);
    cmd.args(["rds", "generate-db-auth-token", "--hostname", host.trim(), "--port", &port.to_string(), "--username", user.trim()]);
    if let Some(r) = region {
        cmd.args(["--region", r]);
    }
    if let Some(p) = profile.map(str::trim).filter(|p| !p.is_empty()) {
        cmd.args(["--profile", p]);
    }
    let out = cmd.stdin(std::process::Stdio::null()).output().await.map_err(|e| Error::Invalid(format!("Couldn't start the AWS CLI: {e}")))?;
    let err = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(Error::Invalid(aws_credentials_problem(&err).unwrap_or_else(|| format!("AWS: {}", err.trim()))));
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        return Err(Error::Invalid("The AWS CLI returned no IAM token.".into()));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_paths_as_pasted() {
        let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap());
        assert_eq!(key_path("  ~/.ssh/aws.pem "), home.join(".ssh/aws.pem"));
        assert_eq!(key_path(r#""/keys/my key.pem""#), PathBuf::from("/keys/my key.pem"));
        assert_eq!(key_path("'/keys/a.pem'"), PathBuf::from("/keys/a.pem"));
        assert_eq!(key_path("/keys/a.pem"), PathBuf::from("/keys/a.pem"));
    }

    #[test]
    fn kubernetes_command_line() {
        assert_eq!(kubernetes_args("postgres", Some("db"), None, 5432, 61000), ["port-forward", "--address", "127.0.0.1", "svc/postgres", "61000:5432", "--namespace", "db"]);
        assert_eq!(kubernetes_args(" pod/pg-0 ", None, Some("prod"), 5432, 1)[3..], ["pod/pg-0", "1:5432", "--context", "prod"]);
    }

    #[test]
    fn rds_regions() {
        assert_eq!(rds_region("mydb.abc123xyz.eu-west-1.rds.amazonaws.com"), Some("eu-west-1"));
        assert_eq!(rds_region("cluster.cluster-ro-abc.us-east-2.rds.amazonaws.com."), Some("us-east-2"));
        assert_eq!(rds_region("db.example.com"), None);
    }

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
