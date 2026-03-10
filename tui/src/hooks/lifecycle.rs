//! Lifecycle hooks — fire shell commands at agent lifecycle events.

use crate::config::schema::HookEntry;
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

/// Result from firing a single hook.
#[derive(Debug)]
#[allow(dead_code)]
pub struct HookResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub action: Option<HookAction>,
}

/// Parsed action from hook JSON output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookAction {
    Allow,
    Deny(String),
    Ask,
    Continue,
}

/// Fire all hooks, passing `context` as JSON to each hook's stdin.
/// Hooks run concurrently. Returns results in order.
pub async fn fire_hooks(hooks: &[HookEntry], context: Value) -> Vec<HookResult> {
    let context_str = serde_json::to_string(&context).unwrap_or_default();
    let mut handles = Vec::new();

    for entry in hooks {
        let cmd = entry.command.clone();
        let timeout_secs = entry.timeout.unwrap_or(30);
        let input = context_str.clone();

        let handle =
            tokio::spawn(async move { run_hook_command(&cmd, &input, timeout_secs).await });
        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(_) => results.push(HookResult {
                success: false,
                stdout: String::new(),
                stderr: "Hook task panicked".into(),
                action: None,
            }),
        }
    }
    results
}

/// Fire hooks filtered by tool name (for PreToolUse, PostToolUse, etc.).
pub async fn fire_hooks_for_tool(
    hooks: &[HookEntry],
    context: Value,
    tool_name: &str,
) -> Vec<HookResult> {
    let filtered: Vec<&HookEntry> = hooks
        .iter()
        .filter(|h| {
            h.match_tools
                .as_ref()
                .map(|tools| tools.iter().any(|t| t == tool_name))
                .unwrap_or(true) // no filter = match all
        })
        .collect();

    if filtered.is_empty() {
        return Vec::new();
    }

    let owned: Vec<HookEntry> = filtered.into_iter().cloned().collect();
    fire_hooks(&owned, context).await
}

async fn run_hook_command(command: &str, stdin_input: &str, timeout_secs: u64) -> HookResult {
    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(stdin_input.as_bytes()).await;
            drop(stdin);
        }

        child.wait_with_output().await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let action = parse_action(&stdout);
            HookResult {
                success: output.status.success(),
                stdout,
                stderr,
                action,
            }
        }
        Ok(Err(e)) => HookResult {
            success: false,
            stdout: String::new(),
            stderr: format!("Failed to spawn hook: {e}"),
            action: None,
        },
        Err(_) => HookResult {
            success: false,
            stdout: String::new(),
            stderr: "Hook timed out".into(),
            action: None,
        },
    }
}

/// Parse optional context string from hook stdout JSON.
/// Returns the "context" field value if present in `{"action":"allow","context":"..."}`.
pub fn parse_context(stdout: &str) -> Option<String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    parsed.get("context")?.as_str().map(|s| s.to_string())
}

/// Parse optional must_keep string from hook stdout JSON.
/// Returns the "must_keep" field value if present in `{"must_keep":"..."}`.
pub fn parse_must_keep(stdout: &str) -> Option<String> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    parsed.get("must_keep")?.as_str().map(|s| s.to_string())
}

fn parse_action(stdout: &str) -> Option<HookAction> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let action = parsed.get("action")?.as_str()?;
    match action {
        "allow" => Some(HookAction::Allow),
        "deny" => {
            let reason = parsed
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("denied by hook")
                .to_string();
            Some(HookAction::Deny(reason))
        }
        "ask" => Some(HookAction::Ask),
        "continue" => Some(HookAction::Continue),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fire_hook_returns_output() {
        let entry = HookEntry {
            command: "echo hello".into(),
            timeout: Some(5),
            match_tools: None,
        };
        let results = fire_hooks(&[entry], serde_json::json!({"event": "test"})).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].stdout.contains("hello"));
    }

    #[tokio::test]
    async fn fire_hook_captures_json_action() {
        let entry = HookEntry {
            command: r#"echo '{"action":"deny","reason":"blocked"}'"#.into(),
            timeout: Some(5),
            match_tools: None,
        };
        let results = fire_hooks(&[entry], serde_json::json!({})).await;
        assert_eq!(results[0].action, Some(HookAction::Deny("blocked".into())));
    }

    #[tokio::test]
    async fn fire_hook_match_tools_filters() {
        let entry = HookEntry {
            command: "echo matched".into(),
            timeout: Some(5),
            match_tools: Some(vec!["write_file".into()]),
        };
        let context = serde_json::json!({"tool_name": "read_file"});
        let results = fire_hooks_for_tool(&[entry], context, "read_file").await;
        assert!(results.is_empty()); // filtered out
    }

    #[tokio::test]
    async fn fire_hook_match_tools_passes() {
        let entry = HookEntry {
            command: "echo matched".into(),
            timeout: Some(5),
            match_tools: Some(vec!["write_file".into()]),
        };
        let context = serde_json::json!({"tool_name": "write_file"});
        let results = fire_hooks_for_tool(&[entry], context, "write_file").await;
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    #[tokio::test]
    async fn fire_hook_timeout() {
        let entry = HookEntry {
            command: "sleep 10".into(),
            timeout: Some(1),
            match_tools: None,
        };
        let results = fire_hooks(&[entry], serde_json::json!({})).await;
        assert!(!results[0].success);
    }

    #[tokio::test]
    async fn fire_hook_nonzero_exit_not_success() {
        let entry = HookEntry {
            command: "exit 1".into(),
            timeout: Some(5),
            match_tools: None,
        };
        let results = fire_hooks(&[entry], serde_json::json!({})).await;
        assert!(!results[0].success);
    }

    #[test]
    fn parse_action_allow() {
        assert_eq!(
            parse_action(r#"{"action":"allow"}"#),
            Some(HookAction::Allow)
        );
    }

    #[test]
    fn parse_action_deny_with_reason() {
        assert_eq!(
            parse_action(r#"{"action":"deny","reason":"not allowed"}"#),
            Some(HookAction::Deny("not allowed".into()))
        );
    }

    #[test]
    fn parse_action_empty() {
        assert_eq!(parse_action(""), None);
    }

    #[test]
    fn parse_action_non_json() {
        assert_eq!(parse_action("just text output"), None);
    }

    #[test]
    fn parse_must_keep_from_hook_output() {
        let stdout = r#"{"must_keep":"Always remember: the deploy target is staging"}"#;
        assert_eq!(
            parse_must_keep(stdout),
            Some("Always remember: the deploy target is staging".to_string()),
        );
    }

    #[test]
    fn parse_must_keep_none_when_missing() {
        assert_eq!(parse_must_keep(r#"{"action":"allow"}"#), None);
        assert_eq!(parse_must_keep(""), None);
    }

    #[test]
    fn parse_action_continue() {
        assert_eq!(
            parse_action(r#"{"action":"continue"}"#),
            Some(HookAction::Continue),
        );
    }

    #[test]
    fn parse_context_from_hook_output() {
        let stdout = r#"{"action":"allow","context":"Remember: always use TypeScript"}"#;
        assert_eq!(
            parse_context(stdout),
            Some("Remember: always use TypeScript".to_string()),
        );
    }

    #[test]
    fn parse_context_none_when_no_context() {
        assert_eq!(parse_context(r#"{"action":"allow"}"#), None);
        assert_eq!(parse_context(""), None);
        assert_eq!(parse_context("just text"), None);
    }

    #[test]
    fn parse_action_from_session_end_hook() {
        // SessionEnd hooks are non-blocking — they fire but don't return actions.
        // Verify the hook infrastructure handles no-action hooks gracefully.
        assert_eq!(parse_action("some log output"), None);
        assert_eq!(parse_action(""), None);
    }

    #[tokio::test]
    async fn fire_hook_allow_action() {
        let entry = HookEntry {
            command: r#"echo '{"action":"allow"}'"#.into(),
            timeout: Some(5),
            match_tools: None,
        };
        let results = fire_hooks(&[entry], serde_json::json!({})).await;
        assert_eq!(results[0].action, Some(HookAction::Allow));
    }
}
