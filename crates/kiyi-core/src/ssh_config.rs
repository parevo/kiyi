//! Hosts from `~/.ssh/config`, so a bastion already set up for the terminal can be picked
//! from a list instead of retyped.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::JumpHost;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHost {
    /// The `Host` alias.
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    pub jump: Option<JumpHost>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// The hosts in the user's SSH config, in file order. Missing or unreadable config means none.
pub fn hosts() -> Vec<SshHost> {
    let Some(home) = home() else { return vec![] };
    let mut text = String::new();
    read_with_includes(&home.join(".ssh/config"), &home, &mut text, 0);
    parse(&text, &home)
}

/// Inlines `Include` files (relative paths are under ~/.ssh, simple `*` globs allowed).
fn read_with_includes(path: &Path, home: &Path, out: &mut String, depth: u8) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for line in text.lines() {
        let (key, value) = split(line);
        if depth < 5 && key.eq_ignore_ascii_case("include") {
            for pattern in value.split_whitespace() {
                let pattern = expand(pattern, home);
                let pattern = if pattern.is_absolute() { pattern } else { home.join(".ssh").join(pattern) };
                for file in glob(&pattern) {
                    read_with_includes(&file, home, out, depth + 1);
                }
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
}

fn glob(pattern: &Path) -> Vec<PathBuf> {
    let name = pattern.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some((prefix, suffix)) = name.split_once('*') else { return vec![pattern.to_path_buf()] };
    let Some(dir) = pattern.parent() else { return vec![] };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().map(|n| n.to_string_lossy()).is_some_and(|n| n.starts_with(prefix) && n.ends_with(suffix)))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn split(line: &str) -> (&str, &str) {
    let line = line.trim();
    if line.starts_with('#') {
        return ("", "");
    }
    let (key, rest) = line.split_once(|c: char| c.is_whitespace() || c == '=').unwrap_or((line, ""));
    (key.trim(), rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim().trim_matches('"'))
}

fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

/// `[user@]host[:port]`, the first hop of a ProxyJump list.
fn jump_spec(spec: &str) -> Option<(Option<String>, String, Option<u16>)> {
    let first = spec.split(',').next()?.trim();
    if first.is_empty() || first.eq_ignore_ascii_case("none") {
        return None;
    }
    let (user, rest) = match first.rsplit_once('@') {
        Some((u, r)) => (Some(u.to_string()), r),
        None => (None, first),
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().ok()),
        _ => (rest, None),
    };
    Some((user, host.to_string(), port))
}

pub fn parse(text: &str, home: &Path) -> Vec<SshHost> {
    #[derive(Default, Clone)]
    struct Block {
        patterns: Vec<String>,
        host: Option<String>,
        port: Option<u16>,
        user: Option<String>,
        identity: Option<String>,
        jump: Option<String>,
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut in_match = false;
    for line in text.lines() {
        let (key, value) = split(line);
        if key.is_empty() {
            continue;
        }
        match key.to_ascii_lowercase().as_str() {
            "host" => {
                in_match = false;
                blocks.push(Block { patterns: value.split_whitespace().map(String::from).collect(), ..Default::default() });
            }
            // Conditional blocks can't be evaluated here; skip what they set.
            "match" => in_match = true,
            _ if in_match => {}
            other => {
                let Some(b) = blocks.last_mut() else { continue };
                // ssh uses the first value it sees for each option.
                match other {
                    "hostname" if b.host.is_none() => b.host = Some(value.to_string()),
                    "port" if b.port.is_none() => b.port = value.parse().ok(),
                    "user" if b.user.is_none() => b.user = Some(value.to_string()),
                    "identityfile" if b.identity.is_none() => b.identity = Some(expand(value, home).to_string_lossy().into_owned()),
                    "proxyjump" if b.jump.is_none() => b.jump = Some(value.to_string()),
                    _ => {}
                }
            }
        }
    }

    // Like ssh: for each option, the first value from any matching block (in file order) wins,
    // so `Host *` defaults usually come last.
    let matching = |alias: &str| -> Vec<&Block> { blocks.iter().filter(|b| b.patterns.iter().any(|p| !p.starts_with('!') && wildcard(p, alias))).collect() };
    let first = |alias: &str, pick: &dyn Fn(&Block) -> Option<String>| matching(alias).into_iter().find_map(pick);

    let mut hosts = Vec::new();
    for b in &blocks {
        for alias in b.patterns.iter().filter(|p| !p.contains(['*', '?', '!'])) {
            let host = first(alias, &|b| b.host.clone()).unwrap_or_else(|| alias.clone()).replace("%h", alias);
            let user = first(alias, &|b| b.user.clone());
            let identity_file = first(alias, &|b| b.identity.clone());
            let port = first(alias, &|b| b.port.map(|p| p.to_string())).and_then(|p| p.parse().ok()).unwrap_or(22);
            let jump = first(alias, &|b| b.jump.clone()).and_then(|j| jump_spec(&j)).map(|(u, h, p)| {
                // The jump may itself be an alias from this file.
                let of = |pick: &dyn Fn(&Block) -> Option<String>| first(&h, pick);
                JumpHost {
                    host: of(&|b| b.host.clone()).unwrap_or_else(|| h.clone()),
                    port: p.or_else(|| of(&|b| b.port.map(|p| p.to_string())).and_then(|p| p.parse().ok())).unwrap_or(22),
                    user: u.or_else(|| of(&|b| b.user.clone())).unwrap_or_default(),
                }
            });
            if !hosts.iter().any(|h: &SshHost| h.alias == *alias) {
                hosts.push(SshHost { alias: alias.clone(), host, port, user, identity_file, jump });
            }
        }
    }
    hosts
}

fn wildcard(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((prefix, rest)) => {
            text.starts_with(prefix) && {
                let tail = &text[prefix.len()..];
                (0..=tail.len()).any(|i| tail.is_char_boundary(i) && wildcard(rest, &tail[i..]))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_hosts_defaults_and_jumps() {
        let home = Path::new("/home/me");
        let text = r#"
# comment
Host gateway
  HostName 3.120.10.5
  User admin
  Port 2222

Host bastion prod-bastion
  HostName 10.0.1.20
  IdentityFile ~/Downloads/prod key.pem
  ProxyJump gateway

Host db-*.internal
  User ubuntu

Host "quoted"
  HostName=quoted.example.com

Match host something
  User ignored

Host *
  User ec2-user
  IdentityFile ~/.ssh/default.pem
"#;
        let hosts = parse(text, home);
        let names: Vec<&str> = hosts.iter().map(|h| h.alias.as_str()).collect();
        assert_eq!(names, ["gateway", "bastion", "prod-bastion", "quoted"]);
        let gateway = &hosts[0];
        assert_eq!((gateway.host.as_str(), gateway.port, gateway.user.as_deref()), ("3.120.10.5", 2222, Some("admin")));
        assert_eq!(gateway.identity_file.as_deref(), Some("/home/me/.ssh/default.pem"));
        let bastion = &hosts[1];
        assert_eq!(bastion.host, "10.0.1.20");
        assert_eq!(bastion.user.as_deref(), Some("ec2-user"));
        assert_eq!(bastion.identity_file.as_deref(), Some("/home/me/Downloads/prod key.pem"));
        assert_eq!(bastion.jump, Some(JumpHost { host: "3.120.10.5".into(), port: 2222, user: "admin".into() }));
        assert_eq!(hosts[3].host, "quoted.example.com");
    }

    #[test]
    fn jump_specs() {
        assert_eq!(jump_spec("me@jump.example.com:2200,other"), Some((Some("me".into()), "jump.example.com".into(), Some(2200))));
        assert_eq!(jump_spec("jump"), Some((None, "jump".into(), None)));
        assert_eq!(jump_spec("none"), None);
    }
}
