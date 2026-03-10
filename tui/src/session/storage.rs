//! SQLite persistence for sessions and conversation history.

use anyhow::Result;
use rusqlite::params;

use crate::config::Config;

/// SQLite storage backend.
pub struct Storage {
    conn: rusqlite::Connection,
}

impl Storage {
    pub fn new(_config: &Config) -> Result<Self> {
        let db_path = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("caboose")
            .join("sessions.db");

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = rusqlite::Connection::open(&db_path)?;
        // WAL mode + NORMAL sync — dramatically reduces fsync overhead
        // while remaining crash-safe. Without this, every INSERT/UPDATE
        // does a full fsync (~10-50ms on macOS APFS), causing UI freezes.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                model TEXT,
                provider TEXT,
                turn_count INTEGER NOT NULL DEFAULT 0,
                cwd TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;

        // Migration: add columns for existing DBs that lack them
        for col in &[
            "model TEXT",
            "provider TEXT",
            "turn_count INTEGER NOT NULL DEFAULT 0",
            "cwd TEXT",
        ] {
            let _ = conn.execute(&format!("ALTER TABLE sessions ADD COLUMN {col}"), []);
        }

        // Migrate: create observations table (Phase 5) — non-fatal
        if let Err(e) = crate::memory::observations::create_tables(&conn) {
            tracing::warn!("Failed to create observations table: {e}");
        }

        // Migrate: create memory FTS5 index (Phase 5) — non-fatal
        if let Err(e) = crate::memory::search::create_tables(&conn) {
            tracing::warn!("Failed to create FTS5 index: {e}");
        }

        Ok(Self { conn })
    }

    /// Get a reference to the underlying connection (for observation logging).
    pub fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }

    /// Insert a new session row.
    pub fn insert_session(&self, session: &super::Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, title, model, provider, turn_count, cwd, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session.id,
                session.title,
                session.model,
                session.provider,
                session.turn_count,
                session.cwd,
                session.created_at.to_rfc3339(),
                session.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Update session metadata (title, turn count, updated_at).
    pub fn update_session(&self, session: &super::Session) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET title = ?1, model = ?2, provider = ?3,
             turn_count = ?4, updated_at = ?5 WHERE id = ?6",
            params![
                session.title,
                session.model,
                session.provider,
                session.turn_count,
                session.updated_at.to_rfc3339(),
                session.id,
            ],
        )?;
        Ok(())
    }

    /// List recent sessions, most recently updated first.
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<super::Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, model, provider, turn_count, cwd, created_at, updated_at
             FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            Ok(super::Session {
                id: row.get(0)?,
                title: row.get(1)?,
                model: row.get(2)?,
                provider: row.get(3)?,
                turn_count: row.get::<_, i32>(4)? as u32,
                cwd: row.get(5)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// List sessions with concatenated message content for search.
    /// Content is truncated to ~2000 chars per session.
    pub fn list_sessions_with_content(&self, limit: usize) -> Result<Vec<SessionSearchResult>> {
        let sessions = self.list_sessions(limit)?;
        let mut results = Vec::with_capacity(sessions.len());

        let mut stmt = self
            .conn
            .prepare("SELECT content FROM messages WHERE session_id = ?1 ORDER BY id ASC")?;

        for session in sessions {
            let mut content_parts: Vec<String> = Vec::new();
            let mut total_len = 0usize;
            let rows = stmt.query_map(params![session.id], |row| row.get::<_, String>(0))?;
            for row in rows {
                let text = row?;
                if total_len + text.len() > 2000 {
                    let remaining = 2000_usize.saturating_sub(total_len);
                    if remaining > 0 {
                        // Truncate at char boundary
                        let safe_end = text.floor_char_boundary(remaining);
                        if safe_end > 0 {
                            content_parts.push(text[..safe_end].to_string());
                        }
                    }
                    break;
                }
                total_len += text.len() + 1; // +1 for separator
                content_parts.push(text);
            }
            let content_index = content_parts.join(" ");
            results.push(SessionSearchResult {
                session,
                content_index,
            });
        }

        Ok(results)
    }

    /// Load a session by ID.
    pub fn get_session(&self, id: &str) -> Result<Option<super::Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, model, provider, turn_count, cwd, created_at, updated_at
             FROM sessions WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            let created_str: String = row.get(6)?;
            let updated_str: String = row.get(7)?;
            Ok(super::Session {
                id: row.get(0)?,
                title: row.get(1)?,
                model: row.get(2)?,
                provider: row.get(3)?,
                turn_count: row.get::<_, i32>(4)? as u32,
                cwd: row.get(5)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
                updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Utc),
            })
        })?;

        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Save a message to the session.
    pub fn insert_message(&self, session_id: &str, role: &str, content: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, chrono::Utc::now().to_rfc3339(),],
        )?;
        Ok(())
    }

    /// Load all messages for a session, in order.
    pub fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, created_at FROM messages
             WHERE session_id = ?1 ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            Ok(StoredMessage {
                role: row.get(0)?,
                content: row.get(1)?,
            })
        })?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }

    /// Delete a session and its messages.
    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE session_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(())
    }
}

/// A stored message row.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub role: String,
    pub content: String,
}

/// A session with pre-fetched message content for search filtering.
#[derive(Debug, Clone)]
pub struct SessionSearchResult {
    pub session: super::Session,
    /// Concatenated message content for client-side search. Empty if no messages.
    pub content_index: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create an in-memory storage for testing.
    fn test_storage() -> Storage {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT,
                model TEXT,
                provider TEXT,
                turn_count INTEGER NOT NULL DEFAULT 0,
                cwd TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();
        Storage { conn }
    }

    fn make_session(id: &str) -> crate::session::Session {
        let now = chrono::Utc::now();
        crate::session::Session {
            id: id.to_string(),
            title: Some("Test session".to_string()),
            model: Some("claude-sonnet".to_string()),
            provider: Some("anthropic".to_string()),
            turn_count: 0,
            cwd: Some("/tmp".to_string()),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn insert_and_get_session() {
        let storage = test_storage();
        let session = make_session("test-1");
        storage.insert_session(&session).unwrap();

        let loaded = storage.get_session("test-1").unwrap().unwrap();
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.title, Some("Test session".to_string()));
        assert_eq!(loaded.model, Some("claude-sonnet".to_string()));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let storage = test_storage();
        assert!(storage.get_session("nope").unwrap().is_none());
    }

    #[test]
    fn list_sessions_ordered_by_updated() {
        let storage = test_storage();
        let mut s1 = make_session("s1");
        s1.title = Some("First".to_string());
        s1.updated_at = chrono::Utc::now() - chrono::Duration::hours(1);
        storage.insert_session(&s1).unwrap();

        let mut s2 = make_session("s2");
        s2.title = Some("Second".to_string());
        storage.insert_session(&s2).unwrap();

        let list = storage.list_sessions(10).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "s2"); // most recent first
        assert_eq!(list[1].id, "s1");
    }

    #[test]
    fn list_sessions_respects_limit() {
        let storage = test_storage();
        for i in 0..5 {
            storage
                .insert_session(&make_session(&format!("s{i}")))
                .unwrap();
        }
        let list = storage.list_sessions(3).unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn update_session() {
        let storage = test_storage();
        let mut session = make_session("u1");
        storage.insert_session(&session).unwrap();

        session.title = Some("Updated title".to_string());
        session.turn_count = 5;
        storage.update_session(&session).unwrap();

        let loaded = storage.get_session("u1").unwrap().unwrap();
        assert_eq!(loaded.title, Some("Updated title".to_string()));
        assert_eq!(loaded.turn_count, 5);
    }

    #[test]
    fn insert_and_load_messages() {
        let storage = test_storage();
        storage.insert_session(&make_session("m1")).unwrap();

        storage.insert_message("m1", "user", "Hello").unwrap();
        storage
            .insert_message("m1", "assistant", "Hi there!")
            .unwrap();

        let messages = storage.load_messages("m1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Hi there!");
    }

    #[test]
    fn list_sessions_with_content_includes_messages() {
        let storage = test_storage();
        let s1 = make_session("s1");
        storage.insert_session(&s1).unwrap();
        storage
            .insert_message("s1", "user", "Fix the authentication bug")
            .unwrap();
        storage
            .insert_message("s1", "assistant", "I'll look at the auth module")
            .unwrap();

        let results = storage.list_sessions_with_content(50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, "s1");
        assert!(
            results[0]
                .content_index
                .contains("Fix the authentication bug")
        );
        assert!(
            results[0]
                .content_index
                .contains("I'll look at the auth module")
        );
    }

    #[test]
    fn list_sessions_with_content_empty_messages() {
        let storage = test_storage();
        let s1 = make_session("s1");
        storage.insert_session(&s1).unwrap();

        let results = storage.list_sessions_with_content(50).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content_index, "");
    }

    #[test]
    fn list_sessions_with_content_respects_limit() {
        let storage = test_storage();
        for i in 0..5 {
            storage
                .insert_session(&make_session(&format!("s{i}")))
                .unwrap();
        }
        let results = storage.list_sessions_with_content(3).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn delete_session_removes_messages() {
        let storage = test_storage();
        storage.insert_session(&make_session("d1")).unwrap();
        storage.insert_message("d1", "user", "msg").unwrap();

        storage.delete_session("d1").unwrap();
        assert!(storage.get_session("d1").unwrap().is_none());
        assert!(storage.load_messages("d1").unwrap().is_empty());
    }
}
