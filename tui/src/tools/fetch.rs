//! Web fetch tool — retrieve content from URLs.

use anyhow::Result;
use serde_json::Value;

use crate::agent::tools::ToolResult;

/// Fetch content from a URL.
pub async fn execute(input: &Value) -> Result<ToolResult> {
    let url = input["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'url'"))?;

    let client = reqwest::Client::new();
    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status();
            match response.text().await {
                Ok(body) => {
                    // Truncate large responses
                    let output = if body.len() > 50_000 {
                        format!("{}\n\n[truncated at 50KB]", &body[..50_000])
                    } else {
                        body
                    };
                    Ok(ToolResult {
                        tool_use_id: String::new(),
                        output: format!("[{status}]\n{output}"),
                        is_error: !status.is_success(),
                        tool_name: None,
                        file_path: None,
                        files_modified: vec![],
                        lines_added: 0,
                        lines_removed: 0,
                    })
                }
                Err(e) => Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("Error reading response body: {e}"),
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                }),
            }
        }
        Err(e) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Fetch failed: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}
