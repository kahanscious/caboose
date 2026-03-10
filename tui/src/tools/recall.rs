//! Recall tool — retrieve cold-stored tool outputs by ID.

use anyhow::Result;
use serde_json::Value;

use crate::agent::cold_storage::ColdStore;
use crate::agent::tools::ToolResult;

/// Retrieve a previously cold-stored tool output.
///
/// Takes `output_id` (required), `offset` (optional, default 0), and
/// `limit` (optional, default 200 lines). Returns the stored content
/// starting at `offset` lines, limited to `limit` lines, with an indicator
/// if the output was shortened.
pub fn execute(input: &Value, cold_store: &ColdStore) -> Result<ToolResult> {
    let output_id = input["output_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'output_id'"))?;
    let offset = input["offset"].as_u64().unwrap_or(0) as usize;
    let limit = input["limit"].as_u64().unwrap_or(200) as usize;

    match cold_store.recall(output_id)? {
        Some(content) => {
            let total_lines = content.lines().count();
            let lines: Vec<&str> = content.lines().skip(offset).take(limit).collect();
            let shown = lines.len();
            let body = lines.join("\n");

            let remaining = total_lines.saturating_sub(offset + shown);
            let output = if remaining > 0 || offset > 0 {
                let offset_info = if offset > 0 {
                    format!(" (starting at line {offset})")
                } else {
                    String::new()
                };
                format!(
                    "{body}\n\n[Showing {shown} of {total_lines} lines{offset_info}. {remaining} lines remaining. Use offset/limit to see more.]"
                )
            } else {
                body
            };

            Ok(ToolResult {
                tool_use_id: String::new(),
                output,
                is_error: false,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
            })
        }
        None => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("No stored output found for id '{output_id}'"),
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

    fn temp_store() -> (tempfile::TempDir, ColdStore) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-recall");
        store.base_dir = dir.path().join("cold").join("test-recall");
        (dir, store)
    }

    #[test]
    fn recall_stored_content() {
        let (_dir, store) = temp_store();
        let content = "line 1\nline 2\nline 3\n";
        let id = store.store("call-1", content).unwrap();

        let result = execute(&serde_json::json!({"output_id": id}), &store).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("line 1"));
        assert!(result.output.contains("line 2"));
        assert!(result.output.contains("line 3"));
    }

    #[test]
    fn recall_with_line_limit() {
        let (_dir, store) = temp_store();
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let id = store.store("call-2", &content).unwrap();

        let result =
            execute(&serde_json::json!({"output_id": id, "limit": 3}), &store).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("line 1"));
        assert!(result.output.contains("line 3"));
        assert!(!result.output.contains("line 4"));
        assert!(result.output.contains("Showing 3 of 10 lines"));
    }

    #[test]
    fn recall_nonexistent_id() {
        let (_dir, store) = temp_store();

        let result =
            execute(&serde_json::json!({"output_id": "does-not-exist"}), &store).unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("No stored output found"));
    }

    #[test]
    fn recall_missing_output_id() {
        let (_dir, store) = temp_store();

        let result = execute(&serde_json::json!({}), &store);
        assert!(result.is_err());
    }

    #[test]
    fn recall_with_offset() {
        let (_dir, store) = temp_store();
        let content = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let id = store.store("call-offset", &content).unwrap();

        let result = execute(
            &serde_json::json!({"output_id": id, "offset": 3, "limit": 3}),
            &store,
        )
        .unwrap();
        assert!(!result.is_error);
        // Should start at line 4 (0-indexed offset 3)
        assert!(result.output.contains("line 4"));
        assert!(result.output.contains("line 5"));
        assert!(result.output.contains("line 6"));
        assert!(!result.output.contains("line 3\n"));
        assert!(!result.output.contains("line 7"));
        assert!(result.output.contains("starting at line 3"));
        assert!(result.output.contains("4 lines remaining"));
    }

    #[test]
    fn recall_with_offset_beyond_content() {
        let (_dir, store) = temp_store();
        let content = "line 1\nline 2\nline 3\n";
        let id = store.store("call-beyond", content).unwrap();

        let result = execute(
            &serde_json::json!({"output_id": id, "offset": 100, "limit": 10}),
            &store,
        )
        .unwrap();
        assert!(!result.is_error);
        // Should show 0 of 3 lines since offset is past all lines
        assert!(result.output.contains("Showing 0 of 3 lines"));
    }
}
