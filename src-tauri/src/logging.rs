//! A local log file, so problems can be diagnosed without telemetry: the user copies the
//! diagnostics from Settings and sends them along with a bug report.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::fmt::writer::MakeWriterExt;

const MAX_BYTES: u64 = 5 * 1024 * 1024;
const CRASH_FILE: &str = "last-crash.txt";
const ISSUES_URL: &str = "https://github.com/parevo/kiyi/issues/new";
/// Browsers and GitHub cut off very long links; the rest of the log can be pasted by hand.
const MAX_ISSUE_BODY: usize = 6000;

pub struct LogFile(pub PathBuf);

/// Logs to stderr and to `<dir>/kiyi.log`, keeping the previous file as `kiyi.old.log`
/// once it grows past 5 MB. Panics are logged too.
pub fn init(dir: &Path, version: &str) -> PathBuf {
    let path = dir.join("kiyi.log");
    let _ = std::fs::create_dir_all(dir);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&path, dir.join("kiyi.old.log"));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path);
    let builder = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_ansi(false);
    match file {
        Ok(file) => builder.with_writer(Mutex::new(file).and(std::io::stderr)).init(),
        Err(_) => builder.with_writer(std::io::stderr).init(),
    }

    let default_hook = std::panic::take_hook();
    let crash = dir.join(CRASH_FILE);
    let running = version.to_string();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "panic", "{info}");
        // Kept for the next launch, which offers to report it.
        let _ = std::fs::write(&crash, format!("Kiyi {running} on {} {}\n\n{info}", std::env::consts::OS, std::env::consts::ARCH));
        default_hook(info);
    }));
    tracing::info!("Kiyi {version} starting on {} {}", std::env::consts::OS, std::env::consts::ARCH);
    path
}

/// Version, platform and the end of the log, for bug reports. Contains no passwords or keys:
/// those never reach the log.
pub fn diagnostics(path: &Path, version: &str) -> String {
    let tail: Vec<String> = File::open(path)
        .map(|f| BufReader::new(f).lines().map_while(Result::ok).collect::<Vec<_>>())
        .map(|lines| lines[lines.len().saturating_sub(200)..].to_vec())
        .unwrap_or_default();
    format!(
        "Kiyi {version}\nOS: {} {}\nLog: {}\n\n{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        path.display(),
        if tail.is_empty() { "(log is empty)".to_string() } else { tail.join("\n") }
    )
}

/// The last panic's message, removed once read so it's offered only once.
pub fn take_crash(log: &Path) -> Option<String> {
    let path = log.parent()?.join(CRASH_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(text)
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn issue_url(title: &str, body: &str) -> String {
    let mut body = body.to_string();
    if body.len() > MAX_ISSUE_BODY {
        let mut cut = MAX_ISSUE_BODY;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
        body.push_str("\n…(shortened; Settings → About & updates → Copy diagnostics has the rest)");
    }
    format!("{ISSUES_URL}?title={}&body={}", encode(title), encode(&body))
}

/// Only ever opens Kiyi's own issue page, so the webview can't use this to launch anything else.
pub fn open_issue(title: &str, body: &str) -> std::io::Result<()> {
    let url = issue_url(title, body);
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_links_are_encoded_and_bounded() {
        let url = issue_url("Crash & burn", "line 1\nçok uzun");
        assert!(url.starts_with("https://github.com/parevo/kiyi/issues/new?title=Crash%20%26%20burn&body=line%201%0A%C3%A7ok"), "{url}");
        let long = issue_url("t", &"ğ".repeat(10_000));
        assert!(long.len() < 6000 * 6 + 500);
        assert!(long.contains("shortened"));
    }
}
