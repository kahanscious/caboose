//! Observation logging — records tool executions for end-of-session extraction.

use anyhow::Result;
use rusqlite::{Connection, params};

/// A single tool execution observation.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Observation {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub target: String,
    pub summary: String,
    pub created_at: String,
}

/// Create the observations table (called during DB migration).
pub fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS observations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            target TEXT,
            summary TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_observations_session
            ON observations(session_id);",
    )?;
    Ok(())
}

/// Record a tool execution observation.
pub fn record(
    conn: &Connection,
    session_id: &str,
    kind: &str,
    target: &str,
    summary: &str,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO observations (session_id, kind, target, summary, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, kind, target, summary, now],
    )?;
    Ok(())
}

/// Get all observations for a session, ordered by creation time.
pub fn for_session(conn: &Connection, session_id: &str) -> Result<Vec<Observation>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, kind, target, summary, created_at
         FROM observations WHERE session_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        Ok(Observation {
            id: row.get(0)?,
            session_id: row.get(1)?,
            kind: row.get(2)?,
            target: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            summary: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            created_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Count observations for a session.
pub fn count_for_session(conn: &Connection, session_id: &str) -> Result<u64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM observations WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count as u64)
}

/// Prune observations older than `days`. Returns number deleted.
pub fn prune(conn: &Connection, days: u32) -> Result<u64> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
    let deleted = conn.execute(
        "DELETE FROM observations WHERE created_at < ?1",
        params![cutoff],
    )?;
    Ok(deleted as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_tables(&conn).unwrap();
        conn
    }

    #[test]
    fn record_and_retrieve_observations() {
        let conn = test_conn();
        record(&conn, "s1", "read", "src/main.rs", "Read src/main.rs").unwrap();
        record(&conn, "s1", "write", "src/new.rs", "Created src/new.rs").unwrap();

        let obs = for_session(&conn, "s1").unwrap();
        assert_eq!(obs.len(), 2);
        assert_eq!(obs[0].kind, "read");
        assert_eq!(obs[1].kind, "write");
    }

    #[test]
    fn for_session_filters_by_session() {
        let conn = test_conn();
        record(&conn, "s1", "read", "a.rs", "Read a.rs").unwrap();
        record(&conn, "s2", "read", "b.rs", "Read b.rs").unwrap();

        let obs = for_session(&conn, "s1").unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].target, "a.rs");
    }

    #[test]
    fn prune_removes_old_observations() {
        let conn = test_conn();
        // Insert an observation with a timestamp 60 days ago
        let old_time = (chrono::Utc::now() - chrono::Duration::days(60)).to_rfc3339();
        conn.execute(
            "INSERT INTO observations (session_id, kind, target, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["s1", "read", "old.rs", "Read old.rs", old_time],
        )
        .unwrap();
        record(&conn, "s1", "read", "new.rs", "Read new.rs").unwrap();

        let pruned = prune(&conn, 30).unwrap();
        assert_eq!(pruned, 1);

        let obs = for_session(&conn, "s1").unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].target, "new.rs");
    }

    #[test]
    fn count_for_session_returns_correct_count() {
        let conn = test_conn();
        record(&conn, "s1", "read", "a.rs", "Read a.rs").unwrap();
        record(&conn, "s1", "write", "b.rs", "Write b.rs").unwrap();
        record(&conn, "s2", "read", "c.rs", "Read c.rs").unwrap();

        assert_eq!(count_for_session(&conn, "s1").unwrap(), 2);
        assert_eq!(count_for_session(&conn, "s2").unwrap(), 1);
        assert_eq!(count_for_session(&conn, "s3").unwrap(), 0);
    }
}
