use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::Rng;
use rusqlite::{params, Connection};

const MAX_ACTIVE_DEVICES: usize = 10;

/// A paired device record returned from the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub paired_at: String,
    pub last_seen: Option<String>,
}

/// SQLite-backed device token store.
pub struct DeviceStore {
    db_path: PathBuf,
}

impl DeviceStore {
    /// Open (or create) the device store at the given path.
    pub fn new(db_path: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();
        let conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS devices (
                id           TEXT PRIMARY KEY,
                name         TEXT NOT NULL,
                token_prefix TEXT NOT NULL,
                token_hash   TEXT NOT NULL,
                paired_at    TEXT NOT NULL,
                last_seen    TEXT,
                revoked      INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_token_prefix
                ON devices (token_prefix)
                WHERE revoked = 0;",
        )?;
        Ok(Self { db_path })
    }

    fn connect(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    /// Return the number of non-revoked devices.
    pub fn active_count(&self) -> Result<usize> {
        let conn = self.connect()?;
        let count: usize = conn.query_row(
            "SELECT COUNT(*) FROM devices WHERE revoked = 0",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Pair a new device, returning the raw 64-hex-char token.
    ///
    /// Fails if there are already 10 active devices.
    pub fn pair(&self, device_id: &str, device_name: &str) -> Result<String> {
        if self.active_count()? >= MAX_ACTIVE_DEVICES {
            bail!("maximum number of active devices ({MAX_ACTIVE_DEVICES}) reached");
        }

        let token = generate_token();
        let prefix = token[..16].to_string();
        let hash = hash_token(&token)?;
        let now = chrono::Utc::now().to_rfc3339();

        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO devices (id, name, token_prefix, token_hash, paired_at, last_seen, revoked)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 0)",
            params![device_id, device_name, prefix, hash, now],
        )?;

        Ok(token)
    }

    /// Verify a token.  Returns the `Device` on success, `None` if the token
    /// does not match any active device.
    pub fn verify(&self, token: &str) -> Result<Option<Device>> {
        if token.len() < 16 {
            return Ok(None);
        }
        let prefix = &token[..16];
        let conn = self.connect()?;

        // Fetch all active candidates sharing the prefix.
        let mut stmt = conn.prepare(
            "SELECT id, name, token_hash, paired_at, last_seen
             FROM devices
             WHERE token_prefix = ?1 AND revoked = 0",
        )?;

        struct Row {
            id: String,
            name: String,
            hash: String,
            paired_at: String,
            _last_seen: Option<String>,
        }

        let rows: Vec<Row> = stmt
            .query_map(params![prefix], |row| {
                Ok(Row {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    hash: row.get(2)?,
                    paired_at: row.get(3)?,
                    _last_seen: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;

        for row in rows {
            if verify_token(token, &row.hash) {
                let now = chrono::Utc::now().to_rfc3339();
                conn.execute(
                    "UPDATE devices SET last_seen = ?1 WHERE id = ?2",
                    params![now, row.id],
                )?;
                return Ok(Some(Device {
                    id: row.id,
                    name: row.name,
                    paired_at: row.paired_at,
                    last_seen: Some(now),
                }));
            }
        }

        Ok(None)
    }

    /// Revoke a device by ID.  Returns `true` if a row was actually updated.
    pub fn revoke(&self, device_id: &str) -> Result<bool> {
        let conn = self.connect()?;
        let rows = conn.execute(
            "UPDATE devices SET revoked = 1 WHERE id = ?1 AND revoked = 0",
            params![device_id],
        )?;
        Ok(rows > 0)
    }

    /// List all active (non-revoked) devices.
    pub fn list(&self) -> Result<Vec<Device>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, paired_at, last_seen
             FROM devices
             WHERE revoked = 0
             ORDER BY paired_at ASC",
        )?;
        let devices = stmt
            .query_map([], |row| {
                Ok(Device {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    paired_at: row.get(2)?,
                    last_seen: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(devices)
    }
}

// ---------------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------------

/// Generate a 256-bit random token as 64 lowercase hex characters.
pub fn generate_token() -> String {
    let bytes: [u8; 32] = rand::thread_rng().r#gen();
    hex::encode(bytes)
}

/// Hash a token with Argon2id and a random salt.
pub fn hash_token(token: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(token.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2 hash error: {e}"))?
        .to_string();
    Ok(hash)
}

/// Verify a raw token against a stored Argon2 hash.
pub fn verify_token(token: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(token.as_bytes(), &parsed)
        .is_ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_store() -> (DeviceStore, NamedTempFile) {
        let f = NamedTempFile::new().unwrap();
        let store = DeviceStore::new(f.path()).unwrap();
        (store, f)
    }

    #[test]
    fn pair_and_verify() {
        let (store, _f) = temp_store();
        let token = store.pair("dev-1", "My Phone").unwrap();
        let device = store.verify(&token).unwrap().expect("should find device");
        assert_eq!(device.id, "dev-1");
        assert_eq!(device.name, "My Phone");
    }

    #[test]
    fn verify_wrong_token_returns_none() {
        let (store, _f) = temp_store();
        let _token = store.pair("dev-1", "My Phone").unwrap();
        // Craft a different valid-length hex token.
        let wrong: String = "aa".repeat(32);
        let result = store.verify(&wrong).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn revoke_prevents_verify() {
        let (store, _f) = temp_store();
        let token = store.pair("dev-1", "Laptop").unwrap();
        assert!(store.verify(&token).unwrap().is_some());
        assert!(store.revoke("dev-1").unwrap());
        assert!(store.verify(&token).unwrap().is_none());
    }

    #[test]
    fn list_shows_active_only() {
        let (store, _f) = temp_store();
        store.pair("dev-1", "Phone").unwrap();
        store.pair("dev-2", "Tablet").unwrap();
        store.revoke("dev-1").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "dev-2");
    }

    #[test]
    fn max_devices_enforced() {
        let (store, _f) = temp_store();
        for i in 0..10 {
            store.pair(&format!("dev-{i}"), &format!("Device {i}")).unwrap();
        }
        let err = store.pair("dev-10", "Overflow").unwrap_err();
        assert!(err.to_string().contains("maximum"));
    }
}
