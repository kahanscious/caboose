//! Shell command execution — sandboxed command runner.

use anyhow::Result;
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

use crate::agent::tools::ToolResult;

/// Maximum output size in bytes before truncation.
const MAX_OUTPUT_BYTES: usize = 50_000;
/// Maximum output lines before truncation.
const MAX_OUTPUT_LINES: usize = 2_000;
/// Default timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Execute a shell command with timeout and output limits.
///
/// Uses a filtered environment that strips secret variables (API keys, tokens, etc.)
/// to prevent leaking credentials through shell commands.
#[allow(dead_code)]
pub async fn execute(input: &Value) -> Result<ToolResult> {
    execute_with_env(input, &[]).await
}

/// Execute a shell command with a filtered environment.
///
/// `additional_secrets` lists extra env var names to strip beyond the built-in patterns.
pub async fn execute_with_env(input: &Value, additional_secrets: &[String]) -> Result<ToolResult> {
    let command = input["command"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'command'"))?;
    let timeout_ms = input["timeout"]
        .as_u64()
        .unwrap_or(DEFAULT_TIMEOUT_SECS * 1000);

    let safe_env = crate::safety::env_filter::filtered_env(additional_secrets);

    let result = tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .env_clear()
            .envs(safe_env)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut combined = String::new();
            if !stdout.is_empty() {
                combined.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push_str("\n--- stderr ---\n");
                }
                combined.push_str(&stderr);
            }

            // Truncate if needed
            let truncated = truncate_output(&combined);
            let exit_code = output.status.code().unwrap_or(-1);

            Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!("[exit code: {exit_code}]\n{truncated}"),
                is_error: !output.status.success(),
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            })
        }
        Ok(Err(e)) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Failed to execute command: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
        Err(_) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Command timed out after {timeout_ms}ms"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}

fn truncate_output(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() > MAX_OUTPUT_LINES || output.len() > MAX_OUTPUT_BYTES {
        let truncated: String = lines
            .iter()
            .take(MAX_OUTPUT_LINES)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        let result = if truncated.len() > MAX_OUTPUT_BYTES {
            truncated[..MAX_OUTPUT_BYTES].to_string()
        } else {
            truncated
        };
        format!("{result}\n\n[output truncated]")
    } else {
        output.to_string()
    }
}
