//! Session management — CRUD operations and SQLite persistence.

pub mod snapshot;
pub mod storage;

use anyhow::Result;

use crate::config::Config;

/// A conversation session.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub turn_count: u32,
    pub cwd: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub parent_session_id: Option<String>,
    pub fork_message_count: Option<u32>,
    pub pins: Vec<String>,
}

/// Manages session lifecycle.
pub struct SessionManager {
    storage: storage::Storage,
}

impl SessionManager {
    pub fn new(config: &Config) -> Result<Self> {
        let storage = storage::Storage::new(config)?;
        Ok(Self { storage })
    }

    /// Create a new session and persist it.
    pub fn create(
        &self,
        model: Option<&str>,
        provider: Option<&str>,
        parent_session_id: Option<&str>,
        fork_message_count: Option<u32>,
    ) -> Result<Session> {
        let now = chrono::Utc::now();
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        let session = Session {
            id: uuid::Uuid::new_v4().to_string(),
            title: None,
            model: model.map(|s| s.to_string()),
            provider: provider.map(|s| s.to_string()),
            turn_count: 0,
            cwd,
            created_at: now,
            updated_at: now,
            parent_session_id: parent_session_id.map(|s| s.to_string()),
            fork_message_count,
            pins: vec![],
        };
        self.storage.insert_session(&session)?;
        Ok(session)
    }

    /// Copy messages from one session to another.
    pub fn copy_messages(&self, from_session_id: &str, to_session_id: &str) -> Result<u32> {
        self.storage.copy_messages(from_session_id, to_session_id)
    }

    /// Update session metadata.
    pub fn update(&self, session: &Session) -> Result<()> {
        self.storage.update_session(session)
    }

    /// List recent sessions.
    #[allow(dead_code)]
    pub fn list(&self, limit: usize) -> Result<Vec<Session>> {
        self.storage.list_sessions(limit)
    }

    /// List recent sessions with pre-fetched content for search.
    pub fn list_with_content(&self, limit: usize) -> Result<Vec<storage::SessionSearchResult>> {
        self.storage.list_sessions_with_content(limit)
    }

    /// Load a session by ID.
    pub fn get(&self, id: &str) -> Result<Option<Session>> {
        self.storage.get_session(id)
    }

    /// Save a chat message for a session.
    pub fn save_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        self.storage.insert_message(session_id, role, content)
    }

    /// Load all messages for a session.
    pub fn load_messages(&self, session_id: &str) -> Result<Vec<storage::StoredMessage>> {
        self.storage.load_messages(session_id)
    }

    /// Delete a session and all its messages.
    pub fn delete(&self, id: &str) -> Result<()> {
        self.storage.delete_session(id)
    }

    /// Update session pins.
    #[allow(dead_code)]
    pub fn update_pins(&self, session_id: &str, pins: &[String]) -> Result<()> {
        self.storage.update_pins(session_id, pins)
    }

    /// Get access to the underlying storage (for observation logging).
    pub fn storage(&self) -> &storage::Storage {
        &self.storage
    }
}
