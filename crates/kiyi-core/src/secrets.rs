//! Passwords live in the OS keychain (macOS Keychain, Windows Credential Manager, the Secret
//! Service on Linux), never on disk.
//!
//! The one exception is development tools (kiyi-devbridge), which can opt into a plain file so
//! tests run where no keychain exists (CI) and don't fill a developer's real one with test
//! passwords. The app never does.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::error::Result;

const SERVICE: &str = "com.parevo.kiyi";

static FILE_STORE: OnceLock<PathBuf> = OnceLock::new();
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// Development tools only: keep secrets in a plain JSON file at `path` instead of the keychain.
pub fn use_file_store_for_development(path: PathBuf) {
    let _ = FILE_STORE.set(path);
}

fn read_file(path: &PathBuf) -> BTreeMap<String, String> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn get(account: &str) -> Result<Option<String>> {
    if let Some(path) = FILE_STORE.get() {
        let _guard = FILE_LOCK.lock().unwrap();
        return Ok(read_file(path).get(account).cloned());
    }
    match keyring::Entry::new(SERVICE, account)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn set(account: &str, value: Option<&str>) -> Result<()> {
    if let Some(path) = FILE_STORE.get() {
        let _guard = FILE_LOCK.lock().unwrap();
        let mut all = read_file(path);
        match value {
            Some(v) => all.insert(account.to_string(), v.to_string()),
            None => all.remove(account),
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec(&all)?)?;
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, account)?;
    match value {
        Some(v) => entry.set_password(v)?,
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

fn connection(connection_id: &str) -> String {
    format!("connection:{connection_id}")
}

pub fn set_password(connection_id: &str, password: &str) -> Result<()> {
    set(&connection(connection_id), Some(password))
}

pub fn get_password(connection_id: &str) -> Result<Option<String>> {
    get(&connection(connection_id))
}

pub fn delete_password(connection_id: &str) -> Result<()> {
    set(&connection(connection_id), None)
}

/// App-level secrets (e.g. `ai:anthropic`), stored next to connection passwords.
pub fn get_secret(account: &str) -> Result<Option<String>> {
    get(account)
}

pub fn set_secret(account: &str, value: Option<&str>) -> Result<()> {
    set(account, value)
}
