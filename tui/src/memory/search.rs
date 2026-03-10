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

/// Search memories by keyword using FTS5. Returns results ranked by relevance.
#[allow(dead_code)]
pub fn search(conn: &Connection, query: &str, limit: usize) -> Result<Vec<MemoryHit>> {
    // FTS5 query — use simple prefix matching
    let fts_query = query
        .split_whitespace()
        .map(|w| format!("\"{}\"", w.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ");

    if fts_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT mi.content, mi.source, rank
         FROM memory_fts
         JOIN memory_index mi ON mi.rowid = memory_fts.rowid
         WHERE memory_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
        Ok(MemoryHit {
            content: row.get(0)?,
            source: row.get(1)?,
            rank: row.get(2)?,
        })
    })?;

    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
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

        let results = search(&conn, "bun npm", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("bun"));
    }

    #[test]
    fn search_returns_ranked_results() {
        let conn = test_conn();
        let lines = vec![
            ("project", "Project uses React with TypeScript"),
            ("project", "TypeScript strict mode enabled"),
            ("global", "Prefer TypeScript over JavaScript"),
        ];
        reindex_from_lines(&conn, &lines).unwrap();

        let results = search(&conn, "TypeScript", 10).unwrap();
        assert!(results.len() >= 2);
    }

    #[test]
    fn search_respects_limit() {
        let conn = test_conn();
        let lines: Vec<_> = (0..20)
            .map(|i| ("project", format!("Memory item {i} about Rust")))
            .collect();
        let refs: Vec<_> = lines.iter().map(|(s, c)| (*s, c.as_str())).collect();
        reindex_from_lines(&conn, &refs).unwrap();

        let results = search(&conn, "Rust", 5).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn reindex_clears_old_entries() {
        let conn = test_conn();
        reindex_from_lines(&conn, &[("project", "old fact")]).unwrap();
        reindex_from_lines(&conn, &[("project", "new fact")]).unwrap();

        let results = search(&conn, "old", 10).unwrap();
        assert!(results.is_empty());
        let results = search(&conn, "new", 10).unwrap();
        assert_eq!(results.len(), 1);
    }
}
