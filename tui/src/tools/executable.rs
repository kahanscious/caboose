//! Executable tool discovery and execution — JSON stdin/stdout protocol.
use crate::config::schema::{CliToolArg, ExecutableToolConfig};
use std::collections::HashMap;

/// Discovered schema from running `path --schema`.
#[derive(Debug, Clone)]
pub struct DiscoveredSchema {
    #[allow(dead_code)] // Used in tests; available for future use
    pub name: String,
    pub description: String,
    pub args: Option<HashMap<String, CliToolArg>>,
}

/// Run `path --schema` and parse the JSON output.
pub async fn discover_schema(path: &str) -> Option<DiscoveredSchema> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new(path)
            .arg("--schema")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;

    let name = parsed.get("name")?.as_str()?.to_string();
    let description = parsed.get("description")?.as_str()?.to_string();
    let args = parsed.get("args").and_then(|a| {
        let obj = a.as_object()?;
        let mut map = HashMap::new();
        for (k, v) in obj {
            let arg = CliToolArg {
                arg_type: v.get("type")?.as_str()?.to_string(),
                description: v
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string()),
                required: v.get("required").and_then(|r| r.as_bool()),
                default: v.get("default").and_then(|d| match d {
                    serde_json::Value::String(s) => Some(toml::Value::String(s.clone())),
                    serde_json::Value::Bool(b) => Some(toml::Value::Boolean(*b)),
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Some(toml::Value::Integer(i))
                        } else {
                            n.as_f64().map(toml::Value::Float)
                        }
                    }
                    _ => None,
                }),
                enum_values: v.get("enum").and_then(|e| {
                    e.as_array().map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                }),
            };
            map.insert(k.clone(), arg);
        }
        Some(map)
    });

    Some(DiscoveredSchema {
        name,
        description,
        args,
    })
}

/// Execute an executable tool: pipe JSON to stdin, read stdout.
pub async fn execute(
    config: &ExecutableToolConfig,
    tool_name: &str,
    input: &serde_json::Value,
) -> crate::agent::tools::ToolResult {
    let timeout_secs = config.timeout.unwrap_or(60);

    let exec_input = serde_json::json!({
        "args": input,
        "context": {
            "working_directory": std::env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
                .unwrap_or_default(),
        }
    });
    let input_str = serde_json::to_string(&exec_input).unwrap_or_default();

    let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
        let mut child = tokio::process::Command::new(&config.path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(input_str.as_bytes()).await;
            drop(stdin);
        }

        child.wait_with_output().await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if !output.status.success() {
                return crate::agent::tools::ToolResult {
                    tool_use_id: String::new(),
                    output: if stderr.is_empty() { stdout } else { stderr },
                    is_error: true,
                    tool_name: Some(format!("exec_{tool_name}")),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }

            // Try to parse as structured JSON output
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(stdout.trim())
                && let Some(output_text) = parsed.get("output").and_then(|o| o.as_str())
            {
                let is_error = parsed
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                return crate::agent::tools::ToolResult {
                    tool_use_id: String::new(),
                    output: output_text.to_string(),
                    is_error,
                    tool_name: Some(format!("exec_{tool_name}")),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }

            // Plain text output
            crate::agent::tools::ToolResult {
                tool_use_id: String::new(),
                output: stdout,
                is_error: false,
                tool_name: Some(format!("exec_{tool_name}")),
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            }
        }
        Ok(Err(e)) => crate::agent::tools::ToolResult {
            tool_use_id: String::new(),
            output: format!("Failed to execute tool: {e}"),
            is_error: true,
            tool_name: Some(format!("exec_{tool_name}")),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        },
        Err(_) => crate::agent::tools::ToolResult {
            tool_use_id: String::new(),
            output: format!("Tool '{tool_name}' timed out after {timeout_secs}s"),
            is_error: true,
            tool_name: Some(format!("exec_{tool_name}")),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        },
    }
}

/// Discover schemas for executable tools that don't have description/args in config.
/// Returns an updated map with discovered descriptions and args filled in.
pub async fn discover_all(
    tools: &HashMap<String, ExecutableToolConfig>,
) -> HashMap<String, ExecutableToolConfig> {
    let mut result = tools.clone();
    for (name, config) in result.iter_mut() {
        // Skip if both description and args are already set
        if config.description.is_some() && config.args.is_some() {
            continue;
        }
        match discover_schema(&config.path).await {
            Some(schema) => {
                if config.description.is_none() {
                    config.description = Some(schema.description);
                }
                if config.args.is_none() {
                    config.args = schema.args;
                }
                tracing::info!("Discovered schema for executable tool '{name}'");
            }
            None => {
                tracing::warn!(
                    "Failed to discover schema for executable tool '{name}' at '{}'",
                    config.path
                );
                if config.description.is_none() {
                    config.description = Some(format!("Executable tool at {}", config.path));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a cross-platform test script. On Unix writes a bash script; on
    /// Windows writes a .cmd batch file. Returns the path to the script.
    fn write_test_script(
        dir: &std::path::Path,
        name: &str,
        unix: &str,
        windows: &str,
    ) -> std::path::PathBuf {
        #[cfg(unix)]
        {
            let p = dir.join(format!("{name}.sh"));
            std::fs::write(&p, unix).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
        #[cfg(windows)]
        {
            let _ = unix; // suppress unused warning
            let p = dir.join(format!("{name}.cmd"));
            std::fs::write(&p, windows).unwrap();
            p
        }
    }

    #[tokio::test]
    async fn discover_schema_from_echo_script() {
        let dir = std::env::temp_dir().join("caboose_test_exec");
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = write_test_script(
            &dir,
            "test-tool",
            r#"#!/bin/bash
if [ "$1" = "--schema" ]; then
    echo '{"name":"test_tool","description":"A test tool","args":{"input":{"type":"string","required":true}}}'
    exit 0
fi
cat
"#,
            r#"@echo off
if "%~1"=="--schema" (
    echo {"name":"test_tool","description":"A test tool","args":{"input":{"type":"string","required":true}}}
    exit /b 0
)
findstr "^"
"#,
        );

        let schema = discover_schema(script_path.to_str().unwrap()).await;
        assert!(schema.is_some());
        let s = schema.unwrap();
        assert_eq!(s.name, "test_tool");
        assert_eq!(s.description, "A test tool");
        assert!(s.args.is_some());
        let args = s.args.unwrap();
        assert_eq!(args["input"].arg_type, "string");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn discover_schema_returns_none_for_bad_script() {
        let schema = discover_schema("/nonexistent/path").await;
        assert!(schema.is_none());
    }

    #[tokio::test]
    async fn execute_simple_text_output() {
        let dir = std::env::temp_dir().join("caboose_test_exec2");
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = write_test_script(
            &dir,
            "echo-tool",
            r#"#!/bin/bash
if [ "$1" = "--schema" ]; then
    echo '{"name":"echo","description":"Echo input"}'
    exit 0
fi
INPUT=$(cat)
echo "Got: $INPUT"
"#,
            r#"@echo off
if "%~1"=="--schema" (
    echo {"name":"echo","description":"Echo input"}
    exit /b 0
)
set /p INPUT=
echo Got: %INPUT%
"#,
        );

        let config = ExecutableToolConfig {
            path: script_path.to_str().unwrap().to_string(),
            timeout: Some(10),
            permission: None,
            description: Some("Echo tool".into()),
            args: None,
        };

        let result = execute(&config, "echo", &serde_json::json!({"message": "hello"})).await;
        assert!(!result.is_error);
        assert!(result.output.contains("Got:"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_json_output() {
        let dir = std::env::temp_dir().join("caboose_test_exec3");
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = write_test_script(
            &dir,
            "json-tool",
            "#!/bin/bash\necho '{\"output\":\"structured result\",\"is_error\":false,\"metadata\":{\"key\":\"val\"}}'\n",
            "@echo off\necho {\"output\":\"structured result\",\"is_error\":false,\"metadata\":{\"key\":\"val\"}}\n",
        );

        let config = ExecutableToolConfig {
            path: script_path.to_str().unwrap().to_string(),
            timeout: Some(10),
            permission: None,
            description: Some("JSON tool".into()),
            args: None,
        };

        let result = execute(&config, "json", &serde_json::json!({})).await;
        assert!(!result.is_error);
        assert_eq!(result.output, "structured result");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_nonzero_exit_is_error() {
        let dir = std::env::temp_dir().join("caboose_test_exec4");
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = write_test_script(
            &dir,
            "fail-tool",
            "#!/bin/bash\necho \"something went wrong\" >&2\nexit 1\n",
            "@echo off\necho something went wrong 1>&2\nexit /b 1\n",
        );

        let config = ExecutableToolConfig {
            path: script_path.to_str().unwrap().to_string(),
            timeout: Some(10),
            permission: None,
            description: Some("Fail tool".into()),
            args: None,
        };

        let result = execute(&config, "fail", &serde_json::json!({})).await;
        assert!(result.is_error);
        assert!(result.output.contains("something went wrong"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn execute_timeout_is_error() {
        let dir = std::env::temp_dir().join("caboose_test_exec5");
        std::fs::create_dir_all(&dir).unwrap();
        let script_path = write_test_script(
            &dir,
            "slow-tool",
            "#!/bin/bash\nsleep 30\n",
            // ping with a high count and timeout acts as a cross-platform sleep
            "@echo off\nping -n 30 127.0.0.1 >nul\n",
        );

        let config = ExecutableToolConfig {
            path: script_path.to_str().unwrap().to_string(),
            timeout: Some(1), // 1 second timeout
            permission: None,
            description: Some("Slow tool".into()),
            args: None,
        };

        let result = execute(&config, "slow", &serde_json::json!({})).await;
        assert!(result.is_error);
        assert!(
            result.output.contains("timed out") || result.output.contains("timeout"),
            "Expected timeout message, got: {}",
            result.output,
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn discover_all_skips_configured_tools() {
        let mut tools = HashMap::new();
        tools.insert(
            "configured".into(),
            ExecutableToolConfig {
                path: "/nonexistent".into(),
                timeout: None,
                permission: None,
                description: Some("Already configured".into()),
                args: Some(HashMap::new()),
            },
        );
        let result = discover_all(&tools).await;
        assert_eq!(
            result["configured"].description.as_deref(),
            Some("Already configured")
        );
    }
}
