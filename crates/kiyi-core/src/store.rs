use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::ConnectionConfig;
use crate::error::{Error, Result};

/// Saved connections, persisted as JSON in the app's config directory.
pub struct ConnectionStore {
    path: PathBuf,
    items: Vec<ConnectionConfig>,
}

impl ConnectionStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        let items = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, items })
    }

    pub fn list(&self) -> &[ConnectionConfig] {
        &self.items
    }

    pub fn get(&self, id: &str) -> Option<&ConnectionConfig> {
        self.items.iter().find(|c| c.id == id)
    }

    pub fn upsert(&mut self, config: ConnectionConfig) -> Result<()> {
        match self.items.iter_mut().find(|c| c.id == config.id) {
            Some(existing) => *existing = config,
            None => self.items.push(config),
        }
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.items.retain(|c| c.id != id);
        self.save()
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write-then-rename so a crash never leaves a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.items)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

/// A file of connections to move to another computer or share with a team. Passwords, keys and
/// passphrases are never in it: they stay in the keychain they were saved to.
#[derive(Serialize, Deserialize)]
struct ExportFile {
    kiyi: String,
    version: u32,
    connections: Vec<ConnectionConfig>,
}

const EXPORT_KIND: &str = "connections";

pub fn write_export(path: &Path, items: &[ConnectionConfig]) -> Result<()> {
    let file = ExportFile { kiyi: EXPORT_KIND.into(), version: 1, connections: items.to_vec() };
    std::fs::write(path, serde_json::to_vec_pretty(&file)?)?;
    Ok(())
}

/// Reads an export, or a plain list of connections like Kiyi's own `connections.json`.
pub fn read_export(path: &Path) -> Result<Vec<ConnectionConfig>> {
    let bytes = std::fs::read(path)?;
    if let Ok(file) = serde_json::from_slice::<ExportFile>(&bytes) {
        if file.kiyi == EXPORT_KIND {
            return Ok(file.connections);
        }
    }
    serde_json::from_slice::<Vec<ConnectionConfig>>(&bytes).map_err(|_| Error::Invalid("This file doesn't contain Kiyi connections.".into()))
}

/// The same database reached the same way, whatever it's called.
pub fn same_target(a: &ConnectionConfig, b: &ConnectionConfig) -> bool {
    a.kind == b.kind && a.host == b.host && a.port == b.port && a.user == b.user && a.database == b.database && a.tunnel == b.tunnel
}
