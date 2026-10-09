//! A local log file, so problems can be diagnosed without telemetry: the user copies the
//! diagnostics from Settings and sends them along with a bug report.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::fmt::writer::MakeWriterExt;

const MAX_BYTES: u64 = 5 * 1024 * 1024;

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
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "panic", "{info}");
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
