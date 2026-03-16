//! FTS5 search index for memory content.

use anyhow::Result;
use rusqlite::{Connection, params};

/// A search hit from the FTS5 index.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MemoryHit {
    pub content: String,
    pub source: String,
    pub rank: f64,
}

/// Create the FTS5 tables.
pub fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_index (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            content TEXT NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
            content,
            content='memory_index',
            content_rowid='rowid'
        );
        -- Triggers to keep FTS in sync
        CREATE TRIGGER IF NOT EXISTS memory_index_ai AFTER INSERT ON memory_index BEGIN
            INSERT INTO memory_fts(rowid, content) VALUES (new.rowid, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS memory_index_ad AFTER DELETE ON memory_index BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, content) VALUES('delete', old.rowid, old.content);
        END;",
    )?;
    Ok(())
}

/// Rebuild FTS index from parsed memory lines.
/// Each entry is (source_label, content_line).
pub fn reindex_from_lines(conn: &Connection, lines: &[(&str, &str)]) -> Result<()> {
    // Clear existing index
    conn.execute("DELETE FROM memory_index", [])?;

    let mut stmt = conn.prepare("INSERT INTO memory_index (source, content) VALUES (?1, ?2)")?;
    for (source, content) in lines {
        if !content.trim().is_empty() {
            stmt.execute(params![source, content])?;
        }
    }
    Ok(())
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
    fn reindex_populates_fts() {
        let conn = test_conn();
        let lines = vec![
            ("project", "User prefers bun over npm"),
            ("project", "Auth uses JWT tokens"),
            ("global", "Always use dark theme"),
        ];
        reindex_from_lines(&conn, &lines).unwrap();

        // Verify rows were inserted
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_index", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
    }

    #[test]
    fn reindex_clears_old_entries() {
        let conn = test_conn();
        reindex_from_lines(&conn, &[("project", "old fact")]).unwrap();
        reindex_from_lines(&conn, &[("project", "new fact")]).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_index", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
