//! Cold storage for large tool outputs.
//!
//! When a tool produces output that exceeds an inline threshold, the full
//! content is written to a file on disk and replaced in the conversation
//! with a compact stub.  The model can request the full content back via
//! a `recall` tool call if it needs more detail.

use std::path::PathBuf;

use anyhow::Result;

/// Identifies a piece of stored output.  Currently just the tool_use_id
/// but wrapped in a type alias for clarity.
pub type OutputId = String;

/// Sanitize a string for use as a filename — keep only alphanumeric, dash,
/// and underscore characters; replace everything else with `_`.
fn sanitize_filename(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Manages on-disk cold storage for a single session.
pub struct ColdStore {
    pub(crate) base_dir: PathBuf,
    #[allow(dead_code)]
    session_id: String,
}

impl ColdStore {
    /// Create a new cold store for the given session.
    ///
    /// Files are stored under `<data_dir>/caboose/cold/<session_id>/`.
    pub fn new(session_id: &str) -> Self {
        let base_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("caboose")
            .join("cold")
            .join(session_id);
        Self {
            base_dir,
            session_id: session_id.to_string(),
        }
    }

    /// Store tool output content to disk, returning an output ID that can be
    /// used to recall it later.
    pub fn store(&self, tool_use_id: &str, content: &str) -> Result<OutputId> {
        std::fs::create_dir_all(&self.base_dir)?;
        let safe_name = sanitize_filename(tool_use_id);
        let file_path = self.base_dir.join(format!("{safe_name}.txt"));
        std::fs::write(&file_path, content)?;
        Ok(tool_use_id.to_string())
    }

    /// Recall previously stored content by output ID.
    ///
    /// Returns `None` if the file does not exist (e.g. after cleanup).
    #[allow(dead_code)]
    pub fn recall(&self, output_id: &str) -> Result<Option<String>> {
        let safe_name = sanitize_filename(output_id);
        let file_path = self.base_dir.join(format!("{safe_name}.txt"));
        if file_path.exists() {
            let content = std::fs::read_to_string(&file_path)?;
            Ok(Some(content))
        } else {
            Ok(None)
        }
    }

    /// Delete the entire session cold storage directory.
    pub fn cleanup(&self) -> Result<()> {
        if self.base_dir.exists() {
            std::fs::remove_dir_all(&self.base_dir)?;
        }
        Ok(())
    }

    /// Remove cold storage directories older than `max_age`.
    pub fn cleanup_stale(max_age: std::time::Duration) -> Result<()> {
        let base = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("caboose")
            .join("cold");
        Self::cleanup_stale_in(&base, max_age)
    }

    /// Remove cold storage directories under `base` older than `max_age`.
    ///
    /// Factored out of [`cleanup_stale`] for testability.
    fn cleanup_stale_in(base: &std::path::Path, max_age: std::time::Duration) -> Result<()> {
        if !base.exists() {
            return Ok(());
        }
        for entry in std::fs::read_dir(base)? {
            let entry = entry?;
            if let Ok(metadata) = entry.metadata()
                && let Ok(modified) = metadata.modified()
                && modified.elapsed().unwrap_or_default() > max_age
            {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        Ok(())
    }
}

/// Build a compact stub string that replaces a large tool output in the
/// conversation context.
///
/// The stub includes the output ID (for recall), the tool name, and a
/// brief summary of the content (line count, byte size, first 3 lines).
pub fn build_stub(output_id: &str, tool_name: &str, tool_args: &str, content: &str) -> String {
    let line_count = content.lines().count();
    let byte_size = content.len();
    let size_str = if byte_size >= 1024 {
        format!("{}KB", byte_size / 1024)
    } else {
        format!("{}B", byte_size)
    };

    let first_lines: String = content.lines().take(3).collect::<Vec<_>>().join("\n");

    if tool_args.is_empty() {
        format!("[stored: {output_id}] {tool_name} → {line_count} lines, {size_str}\n{first_lines}")
    } else {
        format!(
            "[stored: {output_id}] {tool_name}({tool_args}) → {line_count} lines, {size_str}\n{first_lines}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_and_recall() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-session");
        // Override base_dir to use a temp directory
        store.base_dir = dir.path().join("cold").join("test-session");

        let content = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        let id = store.store("tool-call-42", content).unwrap();
        assert_eq!(id, "tool-call-42");

        // Recall should return the same content
        let recalled = store.recall(&id).unwrap();
        assert_eq!(recalled, Some(content.to_string()));

        // Cleanup should remove the directory
        store.cleanup().unwrap();
        let recalled_after = store.recall(&id).unwrap();
        assert_eq!(recalled_after, None);
    }

    #[test]
    fn test_recall_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-session");
        store.base_dir = dir.path().join("cold").join("test-session");

        let result = store.recall("does-not-exist").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_build_stub() {
        let content = "use std::path::PathBuf;\n\nfn main() {\n    println!(\"hello\");\n}\n";
        let stub = build_stub("abc-123", "read_file", r#"path: "src/main.rs""#, content);

        assert!(
            stub.contains("[stored: abc-123]"),
            "stub should contain the output id"
        );
        assert!(
            stub.contains("read_file"),
            "stub should contain the tool name"
        );
        assert!(
            stub.contains(r#"path: "src/main.rs""#),
            "stub should contain tool args"
        );
        assert!(stub.contains("5 lines"), "stub should contain line count");
        assert!(
            stub.contains("use std::path::PathBuf;"),
            "stub should contain first line"
        );
    }

    #[test]
    fn test_build_stub_large_content() {
        let content = "x\n".repeat(1000);
        let stub = build_stub("big-1", "read_file", "", &content);
        assert!(stub.contains("KB"), "large content should show KB size");
        assert!(
            stub.contains("1000 lines"),
            "should show correct line count"
        );
    }

    #[test]
    fn test_build_stub_small_content() {
        let content = "hello\n";
        let stub = build_stub("small-1", "read_file", "", content);
        assert!(stub.contains("6B"), "small content should show byte size");
    }

    #[test]
    fn test_build_stub_no_args() {
        let content = "x\n".repeat(100);
        let stub = build_stub("no-args", "unknown_tool", "", &content);
        assert!(
            stub.contains("unknown_tool →"),
            "no args should omit parens"
        );
        assert!(
            !stub.contains("unknown_tool("),
            "no args should not have parens"
        );
    }

    #[test]
    fn test_cleanup_stale_removes_old_directories() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("cold");

        // Create a cold store under the temp base and write a file
        let session_dir = base.join("old-session");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(session_dir.join("tool-1.txt"), "some output").unwrap();

        // cleanup_stale with Duration::ZERO should remove everything
        ColdStore::cleanup_stale_in(&base, std::time::Duration::ZERO).unwrap();

        assert!(!session_dir.exists(), "stale session dir should be removed");
    }

    #[test]
    fn test_cleanup_stale_keeps_recent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("cold");

        // Create a session directory with a file
        let session_dir = base.join("recent-session");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::write(session_dir.join("tool-1.txt"), "some output").unwrap();

        // cleanup_stale with a very long max_age should keep everything
        ColdStore::cleanup_stale_in(&base, std::time::Duration::from_secs(3600)).unwrap();

        assert!(session_dir.exists(), "recent session dir should be kept");
    }

    #[test]
    fn test_cleanup_stale_noop_when_base_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("nonexistent").join("cold");

        // Should succeed without error even when base doesn't exist
        ColdStore::cleanup_stale_in(&base, std::time::Duration::ZERO).unwrap();
    }

    #[test]
    fn test_sanitize_filename_clean_id() {
        assert_eq!(sanitize_filename("tool-call_42"), "tool-call_42");
    }

    #[test]
    fn test_sanitize_filename_strips_special_chars() {
        assert_eq!(sanitize_filename("tool/call:42..txt"), "tool_call_42__txt");
    }

    #[test]
    fn test_sanitize_filename_path_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "______etc_passwd");
    }

    #[test]
    fn test_store_and_recall_with_special_chars_in_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-sanitize");
        store.base_dir = dir.path().join("cold").join("test-sanitize");

        let content = "hello world";
        let id = store.store("tool/call:42", content).unwrap();
        assert_eq!(id, "tool/call:42"); // returned id is the original

        // Recall with the same original id should work
        let recalled = store.recall("tool/call:42").unwrap();
        assert_eq!(recalled, Some(content.to_string()));
    }
}
