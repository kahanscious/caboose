//! Persistent API key storage — auth.json with 0o600 permissions.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Entry in auth.json — matches OpenCode's format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthEntry {
    #[serde(rename = "type")]
    pub auth_type: String,
    pub key: String,
}

/// Persistent store for API keys backed by auth.json.
pub struct AuthStore {
    path: PathBuf,
    entries: HashMap<String, AuthEntry>,
}

impl AuthStore {
    /// Create a new store, loading from disk if the file exists.
    pub fn new(path: PathBuf) -> Self {
        let entries = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str(&content).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self { path, entries }
    }

    /// Default path: ~/.config/caboose/auth.json
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("caboose").join("auth.json"))
    }

    /// Get a key for a provider.
    pub fn get(&self, provider: &str) -> Option<&str> {
        self.entries.get(provider).map(|e| e.key.as_str())
    }

    /// Set a key for a provider (in memory only — call save() to persist).
    pub fn set(&mut self, provider: &str, key: &str) {
        self.entries.insert(
            provider.to_string(),
            AuthEntry {
                auth_type: "api".to_string(),
                key: key.to_string(),
            },
        );
    }

    /// Remove a key for a provider (in memory only — call save() to persist).
    pub fn remove(&mut self, provider: &str) {
        self.entries.remove(provider);
    }

    /// Write all keys to disk with 0o600 permissions.
    pub fn save(&self) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&self.entries)?;
        std::fs::write(&self.path, &json)?;

        // Set file permissions to owner-only (Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&self.path, perms)?;
        }

        Ok(())
    }

    /// Get all stored provider IDs.
    #[allow(dead_code)]
    pub fn providers(&self) -> Vec<&str> {
        self.entries.keys().map(|k| k.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");

        let mut store = AuthStore::new(path.clone());
        store.set("anthropic", "sk-ant-test123");
        store.set("openrouter", "sk-or-test456");
        store.save().unwrap();

        // Verify file exists and has correct permissions
        let meta = fs::metadata(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }

        // Reload from disk
        let loaded = AuthStore::new(path);
        assert_eq!(loaded.get("anthropic"), Some("sk-ant-test123"));
        assert_eq!(loaded.get("openrouter"), Some("sk-or-test456"));
        assert_eq!(loaded.get("gemini"), None);
    }

    #[test]
    fn load_nonexistent_file() {
        let store = AuthStore::new("/tmp/does-not-exist-auth.json".into());
        assert_eq!(store.get("anthropic"), None);
    }

    #[test]
    fn overwrite_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.json");

        let mut store = AuthStore::new(path.clone());
        store.set("openai", "old-key");
        store.save().unwrap();

        let mut store2 = AuthStore::new(path);
        store2.set("openai", "new-key");
        store2.save().unwrap();

        assert_eq!(store2.get("openai"), Some("new-key"));
    }
}
