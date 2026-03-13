//! File writing tools — write_file, edit_file.

use anyhow::Result;
use serde_json::Value;

use crate::agent::tools::ToolResult;

/// Extract path from input, trying common parameter name variants.
fn resolve_path(input: &Value) -> Option<&str> {
    input
        .get("path")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("filename"))
        .and_then(|v| v.as_str())
}

/// Compute line-level diff between old and new content.
/// Returns (lines_added, lines_removed) by trimming common prefix/suffix lines.
pub fn line_diff(old: &str, new: &str) -> (usize, usize) {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Find common prefix
    let prefix_len = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Find common suffix (after prefix)
    let old_remaining = old_lines.len() - prefix_len;
    let new_remaining = new_lines.len() - prefix_len;
    let suffix_len = old_lines[prefix_len..]
        .iter()
        .rev()
        .zip(new_lines[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(old_remaining)
        .min(new_remaining);

    let removed = old_remaining - suffix_len;
    let added = new_remaining - suffix_len;
    (added, removed)
}

/// Produce diff lines for preview: removed lines prefixed "- ", added lines prefixed "+ ".
/// Returns empty vec if old == new. Does NOT include context lines.
/// Skips common prefix/suffix lines (same logic as line_diff).
pub fn compute_diff_lines(old: &str, new: &str) -> Vec<String> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // Find common prefix
    let prefix_len = old_lines
        .iter()
        .zip(new_lines.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Find common suffix
    let old_rem = old_lines.len() - prefix_len;
    let new_rem = new_lines.len() - prefix_len;
    let suffix_len = old_lines[prefix_len..]
        .iter()
        .rev()
        .zip(new_lines[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
        .min(old_rem)
        .min(new_rem);

    let old_changed = &old_lines[prefix_len..old_lines.len() - suffix_len];
    let new_changed = &new_lines[prefix_len..new_lines.len() - suffix_len];

    let mut result = Vec::new();
    for line in old_changed {
        result.push(format!("- {line}"));
    }
    for line in new_changed {
        result.push(format!("+ {line}"));
    }
    result
}

/// Write content to a file (create or overwrite).
pub async fn execute_write(input: &Value) -> Result<ToolResult> {
    let path = resolve_path(input).ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let content = input["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'content'"))?;

    // Read old content for line diff computation
    let old_content = tokio::fs::read_to_string(path).await.ok();

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    match tokio::fs::write(path, content).await {
        Ok(()) => {
            let (lines_added, lines_removed) = match &old_content {
                Some(old) => line_diff(old, content),
                None => (content.lines().count(), 0), // New file
            };
            Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!("Wrote {} bytes to {path}", content.len()),
                is_error: false,
                tool_name: None,
                file_path: None,
                files_modified: vec![std::path::PathBuf::from(path)],
                lines_added,
                lines_removed,
            })
        }
        Err(e) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Error writing {path}: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}

/// Edit a file by replacing a search string with a replacement.
pub async fn execute_edit(input: &Value) -> Result<ToolResult> {
    let path = resolve_path(input).ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let old_string = input["old_string"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'old_string'"))?;
    let new_string = input["new_string"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'new_string'"))?;

    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            let count = content.matches(old_string).count();
            if count == 0 {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("old_string not found in {path}"),
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            }
            if count > 1 {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("old_string found {count} times in {path} — must be unique"),
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            }
            let new_content = content.replacen(old_string, new_string, 1);
            tokio::fs::write(path, &new_content).await?;

            let lines_removed = old_string.lines().count();
            let lines_added = new_string.lines().count();

            Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!("Edited {path}"),
                is_error: false,
                tool_name: None,
                file_path: None,
                files_modified: vec![std::path::PathBuf::from(path)],
                lines_added,
                lines_removed,
            })
        }
        Err(e) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Error reading {path}: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_file_populates_files_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "hello"
        });
        let result = execute_write(&input).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.files_modified.len(), 1);
        assert_eq!(result.files_modified[0], path);
    }

    #[tokio::test]
    async fn edit_file_populates_files_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "old_string": "hello",
            "new_string": "goodbye"
        });
        let result = execute_edit(&input).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.files_modified.len(), 1);
        assert_eq!(result.files_modified[0], path);
    }

    #[tokio::test]
    async fn edit_file_error_has_empty_files_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.txt");
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "old_string": "x",
            "new_string": "y"
        });
        let result = execute_edit(&input).await.unwrap();
        assert!(result.is_error);
        assert!(result.files_modified.is_empty());
    }

    #[tokio::test]
    async fn write_new_file_counts_all_lines_as_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "line1\nline2\nline3"
        });
        let result = execute_write(&input).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.lines_added, 3);
        assert_eq!(result.lines_removed, 0);
    }

    #[tokio::test]
    async fn write_overwrite_computes_actual_diff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "line1\nline2\nline3").unwrap();
        // Prepend a line — should be +1 -0
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "new_line\nline1\nline2\nline3"
        });
        let result = execute_write(&input).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.lines_added, 1);
        assert_eq!(result.lines_removed, 0);
    }

    #[tokio::test]
    async fn write_full_replace_counts_all_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "old1\nold2").unwrap();
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "new1\nnew2\nnew3"
        });
        let result = execute_write(&input).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.lines_added, 3);
        assert_eq!(result.lines_removed, 2);
    }

    #[test]
    fn line_diff_prepend() {
        let (added, removed) = line_diff("a\nb\nc", "new\na\nb\nc");
        assert_eq!(added, 1);
        assert_eq!(removed, 0);
    }

    #[test]
    fn line_diff_append() {
        let (added, removed) = line_diff("a\nb", "a\nb\nc");
        assert_eq!(added, 1);
        assert_eq!(removed, 0);
    }

    #[test]
    fn line_diff_middle_change() {
        let (added, removed) = line_diff("a\nb\nc", "a\nX\nc");
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
    }

    #[test]
    fn line_diff_identical() {
        let (added, removed) = line_diff("a\nb\nc", "a\nb\nc");
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn edit_file_counts_lines_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.txt");
        std::fs::write(&path, "line1\nline2\nline3").unwrap();
        let input = serde_json::json!({
            "path": path.to_str().unwrap(),
            "old_string": "line2",
            "new_string": "replaced1\nreplaced2"
        });
        let result = execute_edit(&input).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.lines_added, 2);
        assert_eq!(result.lines_removed, 1);
    }

    #[test]
    fn compute_diff_lines_basic_change() {
        let lines = compute_diff_lines("line1\nline2\nline3", "line1\nchanged\nline3");
        assert!(lines.iter().any(|l| l.starts_with("- line2")));
        assert!(lines.iter().any(|l| l.starts_with("+ changed")));
    }

    #[test]
    fn compute_diff_lines_new_file() {
        let lines = compute_diff_lines("", "hello\nworld");
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("+ hello"));
        assert!(lines[1].starts_with("+ world"));
    }

    #[test]
    fn compute_diff_lines_identical_returns_empty() {
        let lines = compute_diff_lines("a\nb", "a\nb");
        assert!(lines.is_empty());
    }
}
