//! File-based memory persistence with directory management.

use anyhow::Result;
use std::path::PathBuf;

const MAX_LINES: usize = 200;

const MEMORY_TEMPLATE: &str = "# Memory\n\n\
    This file stores persistent memories across sessions.\n\
    Edit it directly or let the assistant manage it.\n";

/// Loaded memory context for system prompt injection.
pub struct MemoryContext {
    /// Contents of project MEMORY.md (first 200 lines), or None if missing/disabled.
    pub project: Option<String>,
    /// Contents of global MEMORY.md (first 200 lines), or None if missing/disabled.
    pub global: Option<String>,
}

/// File-based memory store with global and project scopes.
pub struct MemoryStore {
    global_dir: PathBuf,
    project_dir: PathBuf,
    enabled: bool,
}

impl MemoryStore {
    pub fn new(global_dir: PathBuf, project_dir: PathBuf, enabled: bool) -> Self {
        Self {
            global_dir,
            project_dir,
            enabled,
        }
    }

    /// Create memory directories and empty MEMORY.md files if they don't exist.
    pub fn init(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        Self::ensure_memory_file(&self.global_dir)?;
        Self::ensure_memory_file(&self.project_dir)?;
        Ok(())
    }

    /// Read MEMORY.md contents for system prompt injection.
    /// Returns empty context if disabled.
    pub fn load_context(&self) -> MemoryContext {
        if !self.enabled {
            return MemoryContext {
                project: None,
                global: None,
            };
        }
        MemoryContext {
            global: Self::read_truncated(&self.global_dir.join("MEMORY.md")),
            project: Self::read_truncated(&self.project_dir.join("MEMORY.md")),
        }
    }

    /// Get the project memory directory path.
    pub fn project_dir(&self) -> &PathBuf {
        &self.project_dir
    }

    /// Get the global memory directory path.
    #[allow(dead_code)]
    pub fn global_dir(&self) -> &PathBuf {
        &self.global_dir
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Reindex FTS5 from memory files. Reads all .md files from both directories.
    pub fn reindex(&self, conn: &rusqlite::Connection) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let mut lines: Vec<(&str, String)> = Vec::new();

        for (scope, dir) in [("project", &self.project_dir), ("global", &self.global_dir)] {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if entry.path().extension().is_some_and(|e| e == "md")
                        && let Ok(content) = std::fs::read_to_string(entry.path())
                    {
                        for line in content.lines() {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                                lines.push((scope, trimmed.to_string()));
                            }
                        }
                    }
                }
            }
        }

        let refs: Vec<(&str, &str)> = lines.iter().map(|(s, c)| (*s, c.as_str())).collect();
        crate::memory::search::reindex_from_lines(conn, &refs)
    }

    fn ensure_memory_file(dir: &PathBuf) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("MEMORY.md");
        if !path.exists() {
            std::fs::write(&path, MEMORY_TEMPLATE)?;
        }
        Ok(())
    }

    fn read_truncated(path: &PathBuf) -> Option<String> {
        let content = std::fs::read_to_string(path).ok()?;
        let lines: Vec<&str> = content.lines().take(MAX_LINES).collect();
        if lines.is_empty() {
            return None;
        }
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_directories_and_memory_files() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        let store = MemoryStore::new(
            global.path().to_path_buf(),
            project.path().to_path_buf(),
            true,
        );
        store.init().unwrap();

        assert!(global.path().join("MEMORY.md").exists());
        assert!(project.path().join("MEMORY.md").exists());
    }

    #[test]
    fn init_does_not_overwrite_existing() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        std::fs::create_dir_all(project.path()).unwrap();
        std::fs::write(project.path().join("MEMORY.md"), "# Existing\n- fact one\n").unwrap();

        let store = MemoryStore::new(
            global.path().to_path_buf(),
            project.path().to_path_buf(),
            true,
        );
        store.init().unwrap();

        let content = std::fs::read_to_string(project.path().join("MEMORY.md")).unwrap();
        assert!(content.contains("fact one"));
    }

    #[test]
    fn load_context_reads_memory_files() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        std::fs::create_dir_all(global.path()).unwrap();
        std::fs::create_dir_all(project.path()).unwrap();
        std::fs::write(global.path().join("MEMORY.md"), "global fact").unwrap();
        std::fs::write(project.path().join("MEMORY.md"), "project fact").unwrap();

        let store = MemoryStore::new(
            global.path().to_path_buf(),
            project.path().to_path_buf(),
            true,
        );
        let ctx = store.load_context();
        assert_eq!(ctx.global.as_deref(), Some("global fact"));
        assert_eq!(ctx.project.as_deref(), Some("project fact"));
    }

    #[test]
    fn load_context_truncates_at_200_lines() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        std::fs::create_dir_all(project.path()).unwrap();
        let long_content: String = (0..300).map(|i| format!("line {i}\n")).collect();
        std::fs::write(project.path().join("MEMORY.md"), &long_content).unwrap();

        let store = MemoryStore::new(
            global.path().to_path_buf(),
            project.path().to_path_buf(),
            true,
        );
        let ctx = store.load_context();
        let project = ctx.project.unwrap();
        let lines: Vec<_> = project.lines().collect();
        assert_eq!(lines.len(), 200);
    }

    #[test]
    fn disabled_store_returns_empty_context() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        std::fs::create_dir_all(project.path()).unwrap();
        std::fs::write(project.path().join("MEMORY.md"), "should not see this").unwrap();

        let store = MemoryStore::new(
            global.path().to_path_buf(),
            project.path().to_path_buf(),
            false, // disabled
        );
        let ctx = store.load_context();
        assert!(ctx.global.is_none());
        assert!(ctx.project.is_none());
    }

    #[test]
    fn reindex_and_search_round_trip() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        std::fs::create_dir_all(project.path()).unwrap();
        std::fs::write(
            project.path().join("MEMORY.md"),
            "# Memory\n- Project uses Rust with tokio\n- Tests run with cargo test\n",
        )
        .unwrap();

        let store = MemoryStore::new(
            global.path().to_path_buf(),
            project.path().to_path_buf(),
            true,
        );

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::memory::search::create_tables(&conn).unwrap();
        store.reindex(&conn).unwrap();

        let results = crate::memory::search::search(&conn, "Rust tokio", 10).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }
}
