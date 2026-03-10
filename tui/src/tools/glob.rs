//! Glob tool — find files matching a pattern.

use anyhow::Result;
use serde_json::Value;

use crate::agent::tools::ToolResult;

/// Find files matching a glob pattern.
pub async fn execute(input: &Value) -> Result<ToolResult> {
    let pattern = input["pattern"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern'"))?;
    let root = input["path"].as_str().unwrap_or(".");

    let full_pattern = if pattern.starts_with('/') || pattern.starts_with('.') {
        pattern.to_string()
    } else {
        format!("{root}/{pattern}")
    };

    match glob::glob(&full_pattern) {
        Ok(paths) => {
            let matches: Vec<String> = paths
                .filter_map(|p| p.ok())
                .map(|p| p.display().to_string())
                .collect();
            Ok(ToolResult {
                tool_use_id: String::new(),
                output: if matches.is_empty() {
                    "No files found".to_string()
                } else {
                    matches.join("\n")
                },
                is_error: false,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            })
        }
        Err(e) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Invalid glob pattern: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}
