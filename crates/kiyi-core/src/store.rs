use std::path::PathBuf;

use crate::config::ConnectionConfig;
use crate::error::Result;

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
