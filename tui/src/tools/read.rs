//! File reading tools — read_file, list_directory.

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

/// Read a file's contents, optionally with offset and limit.
pub async fn execute(input: &Value) -> Result<ToolResult> {
    let path = resolve_path(input).ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;
    let offset = input["offset"].as_u64().unwrap_or(0) as usize;
    let limit = input["limit"].as_u64().unwrap_or(200) as usize;

    match tokio::fs::read_to_string(path).await {
        Ok(content) => {
            let total_lines = content.lines().count();
            let lines: Vec<&str> = content.lines().skip(offset).take(limit).collect();
            let shown = lines.len();
            let numbered: String = lines
                .iter()
                .enumerate()
                .map(|(i, line)| format!("{:>6}\t{}", offset + i + 1, line))
                .collect::<Vec<_>>()
                .join("\n");
            let output = if offset + shown < total_lines {
                format!(
                    "{}\n\n[Showing lines {}-{} of {} total. Use offset/limit to read other sections.]",
                    numbered,
                    offset + 1,
                    offset + shown,
                    total_lines
                )
            } else {
                numbered
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

/// List directory contents.
pub async fn execute_list_dir(input: &Value) -> Result<ToolResult> {
    let path = resolve_path(input).ok_or_else(|| anyhow::anyhow!("missing 'path'"))?;

    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => {
            let mut items = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                items.push(if is_dir { format!("{name}/") } else { name });
            }
            items.sort();
            Ok(ToolResult {
                tool_use_id: String::new(),
                output: items.join("\n"),
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
            output: format!("Error listing {path}: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}
