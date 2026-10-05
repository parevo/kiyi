//! Passwords live in the OS keychain (macOS Keychain, Windows Credential Manager), never on disk.

use crate::error::Result;

const SERVICE: &str = "com.parevo.kiyi";

fn entry(connection_id: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, &format!("connection:{connection_id}"))?)
}

pub fn set_password(connection_id: &str, password: &str) -> Result<()> {
    entry(connection_id)?.set_password(password)?;
    Ok(())
}

pub fn get_password(connection_id: &str) -> Result<Option<String>> {
    match entry(connection_id)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn delete_password(connection_id: &str) -> Result<()> {
    match entry(connection_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn named(account: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, account)?)
}

/// App-level secrets (e.g. `ai:anthropic`), stored next to connection passwords.
pub fn get_secret(account: &str) -> Result<Option<String>> {
    match named(account)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set_secret(account: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(v) => named(account)?.set_password(v)?,
        None => match named(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}
