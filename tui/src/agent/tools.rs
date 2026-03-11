//! Tool call parsing and execution dispatch.

use anyhow::Result;
use serde_json::Value;

/// Result of executing a tool.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub output: String,
    pub is_error: bool,
    /// Which tool produced this result (e.g. "read_file", "write_file").
    pub tool_name: Option<String>,
    /// The file path the tool operated on, if applicable.
    pub file_path: Option<String>,
    /// Files this tool modified on disk (used by post-tool hooks).
    pub files_modified: Vec<std::path::PathBuf>,
    /// Lines added by this tool invocation.
    pub lines_added: usize,
    /// Lines removed by this tool invocation.
    pub lines_removed: usize,
}

/// Parse and dispatch a tool call to the appropriate tool handler.
///
/// After dispatching, the result is annotated with the tool name and the file
/// path extracted from the input JSON (if present), so callers can track which
/// files each tool touched.
///
/// `additional_secrets` is forwarded to `run_command` for env filtering.
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    name: &str,
    input: &Value,
    additional_secrets: &[String],
    mcp_manager: Option<&mut crate::mcp::McpManager>,
    lsp_manager: Option<&mut crate::lsp::LspManager>,
    services: Option<&crate::config::schema::ServicesConfig>,
    cli_tools: Option<&std::collections::HashMap<String, crate::config::schema::CliToolConfig>>,
    deny_list: &[String],
    exec_tools: Option<
        &std::collections::HashMap<String, crate::config::schema::ExecutableToolConfig>,
    >,
) -> Result<ToolResult> {
    // MCP tool routing — namespaced names contain ':'
    if name.contains(':') {
        if let Some(manager) = mcp_manager {
            return Ok(manager.call_tool(name, input).await);
        }
        return Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("MCP tool '{name}' requested but MCP is not available"),
            is_error: true,
            tool_name: Some(name.to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        });
    }

    // Try common parameter name variants for file path extraction.
    // Weaker models sometimes use "file_path" or "filename" instead of "path".
    let file_path = input
        .get("path")
        .or_else(|| input.get("file_path"))
        .or_else(|| input.get("filename"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut result = match name {
        "read_file" => crate::tools::read::execute(input).await?,
        "write_file" => crate::tools::write::execute_write(input).await?,
        "edit_file" => crate::tools::write::execute_edit(input).await?,
        "glob" => crate::tools::glob::execute(input).await?,
        "grep" => crate::tools::grep::execute(input).await?,
        "run_command" => crate::tools::shell::execute_with_env(input, additional_secrets).await?,
        "list_directory" => crate::tools::read::execute_list_dir(input).await?,
        "apply_patch" => crate::tools::patch::execute(input).await?,
        "fetch" => crate::tools::fetch::execute(input).await?,
        "diagnostics" => {
            if let Some(lsp) = lsp_manager {
                crate::tools::diagnostics::execute(input, lsp).await?
            } else {
                ToolResult {
                    tool_use_id: String::new(),
                    output: "LSP is not available".to_string(),
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                }
            }
        }
        "lsp" => {
            if let Some(lsp) = lsp_manager {
                crate::tools::lsp::execute(input, lsp).await?
            } else {
                ToolResult {
                    tool_use_id: String::new(),
                    output: "LSP is not available".to_string(),
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                }
            }
        }
        "web_search" => {
            // Resolve provider and API key from services config, falling back to env
            let (provider, api_key_env) = services
                .and_then(|s| s.services.get("web_search"))
                .filter(|sc| sc.enabled)
                .map(|sc| {
                    (
                        sc.provider.as_str(),
                        sc.api_key_env.as_deref().unwrap_or("TAVILY_API_KEY"),
                    )
                })
                .unwrap_or(("tavily", "TAVILY_API_KEY"));

            match std::env::var(api_key_env) {
                Ok(key) if !key.is_empty() => {
                    crate::tools::web_search::execute(input, provider, &key).await?
                }
                _ => ToolResult {
                    tool_use_id: String::new(),
                    output: format!(
                        "Web search requires an API key. Set the {api_key_env} environment variable, \
                         or configure [services.web_search] in config.toml:\n\n\
                         [services.web_search]\n\
                         provider = \"tavily\"\n\
                         api_key_env = \"TAVILY_API_KEY\""
                    ),
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                },
            }
        }
        name if name.starts_with("cli_") => {
            let tool_key = &name[4..];
            let Some(cli_config) = cli_tools.and_then(|r| r.get(tool_key)) else {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("Unknown CLI tool: {name}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            };

            // Build the full command from config + args
            let full_command = if let Some(ref arg_defs) = cli_config.args {
                // Typed args mode: interpolate $var templates or append --key=value
                let mut cmd = cli_config.command.clone();
                let has_templates = cmd.contains('$');
                if has_templates {
                    for arg_name in arg_defs.keys() {
                        let val = input
                            .get(arg_name)
                            .and_then(|v| match v {
                                Value::String(s) => Some(s.clone()),
                                Value::Bool(b) => Some(b.to_string()),
                                Value::Number(n) => Some(n.to_string()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        cmd = cmd.replace(&format!("${arg_name}"), &val);
                    }
                    cmd
                } else {
                    // Passthrough: append as --key=value
                    let mut parts = vec![cli_config.command.clone()];
                    for arg_name in arg_defs.keys() {
                        if let Some(val) = input.get(arg_name) {
                            match val {
                                Value::Bool(true) => parts.push(format!("--{arg_name}")),
                                Value::Bool(false) => {}
                                Value::String(s) => parts.push(format!("--{arg_name}={s}")),
                                Value::Number(n) => parts.push(format!("--{arg_name}={n}")),
                                _ => {}
                            }
                        }
                    }
                    parts.join(" ")
                }
            } else {
                // Simple mode: append optional args string
                let args = input.get("args").and_then(|v| v.as_str()).unwrap_or("");
                if args.is_empty() {
                    cli_config.command.clone()
                } else {
                    format!("{} {args}", cli_config.command)
                }
            };

            // Check deny list
            let deny_decision = crate::safety::command_policy::check(&full_command, &[], deny_list);
            if let crate::safety::command_policy::Decision::Deny(reason) = deny_decision {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("CLI tool blocked by deny list: {reason}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            }

            let cmd_input = serde_json::json!({ "command": full_command });
            crate::tools::shell::execute_with_env(&cmd_input, additional_secrets).await?
        }
        name if name.starts_with("exec_") => {
            let tool_key = &name[5..];
            let Some(exec_config) = exec_tools.and_then(|r| r.get(tool_key)) else {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("Unknown executable tool: {name}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            };

            // Check deny list against the executable path
            let deny_decision =
                crate::safety::command_policy::check(&exec_config.path, &[], deny_list);
            if let crate::safety::command_policy::Decision::Deny(reason) = deny_decision {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("Executable tool blocked by deny list: {reason}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            }

            crate::tools::executable::execute(exec_config, tool_key, input).await
        }
        "create_pr" | "list_prs" | "list_issues" | "check_ci" | "review_pr" | "merge_pr"
        | "create_mr" | "list_mrs" | "review_mr" | "merge_mr" => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let provider = crate::scm::detection::detect_provider(&cwd);
            match crate::scm::tools::execute_scm_tool(name, input, &cwd, &provider) {
                Ok(output) => ToolResult {
                    tool_use_id: String::new(),
                    output,
                    is_error: false,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                },
                Err(e) => ToolResult {
                    tool_use_id: String::new(),
                    output: e,
                    is_error: true,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                },
            }
        }
        _ => ToolResult {
            tool_use_id: String::new(),
            output: format!("Unknown tool: {name}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        },
    };

    result.tool_name = Some(name.to_string());
    result.file_path = file_path;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_builtin_tool_works_without_mcp() {
        let result = execute_tool(
            "list_directory",
            &serde_json::json!({"path": "."}),
            &[],
            None,
            None,
            None,
            None,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn execute_mcp_tool_without_manager_returns_error() {
        let result = execute_tool(
            "server:tool",
            &serde_json::json!({}),
            &[],
            None,
            None,
            None,
            None,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("MCP"));
    }

    #[test]
    fn tool_result_has_files_modified() {
        let result = ToolResult {
            tool_use_id: String::new(),
            output: String::new(),
            is_error: false,
            tool_name: None,
            file_path: None,
            files_modified: vec![std::path::PathBuf::from("/tmp/test.rs")],
            lines_added: 0,
            lines_removed: 0,
        };
        assert_eq!(result.files_modified.len(), 1);
    }

    #[tokio::test]
    async fn execute_unknown_builtin_returns_error() {
        let result = execute_tool(
            "nonexistent_tool",
            &serde_json::json!({}),
            &[],
            None,
            None,
            None,
            None,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn web_search_without_config_checks_env() {
        // SAFETY: This test is single-threaded and no other thread reads this env var.
        unsafe { std::env::remove_var("TAVILY_API_KEY") };
        let result = execute_tool(
            "web_search",
            &serde_json::json!({"query": "test"}),
            &[],
            None,
            None,
            None,
            None,
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("TAVILY_API_KEY") || result.output.contains("api key"));
    }

    #[tokio::test]
    async fn execute_cli_tool_simple() {
        let mut cli_tools = std::collections::HashMap::new();
        cli_tools.insert(
            "hello".into(),
            crate::config::schema::CliToolConfig {
                command: "echo hello".into(),
                description: "Say hello".into(),
                args: None,
                permission: None,
                output_format: None,
                max_output_lines: None,
            },
        );
        let result = execute_tool(
            "cli_hello",
            &serde_json::json!({}),
            &[],
            None,
            None,
            None,
            Some(&cli_tools),
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn execute_cli_tool_with_simple_args() {
        let mut cli_tools = std::collections::HashMap::new();
        cli_tools.insert(
            "greet".into(),
            crate::config::schema::CliToolConfig {
                command: "echo".into(),
                description: "Echo args".into(),
                args: None,
                permission: None,
                output_format: None,
                max_output_lines: None,
            },
        );
        let result = execute_tool(
            "cli_greet",
            &serde_json::json!({"args": "world"}),
            &[],
            None,
            None,
            None,
            Some(&cli_tools),
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("world"));
    }

    #[tokio::test]
    async fn execute_cli_tool_denied_by_deny_list() {
        let mut cli_tools = std::collections::HashMap::new();
        cli_tools.insert(
            "danger".into(),
            crate::config::schema::CliToolConfig {
                command: "rm -rf /".into(),
                description: "Dangerous".into(),
                args: None,
                permission: None,
                output_format: None,
                max_output_lines: None,
            },
        );
        let result = execute_tool(
            "cli_danger",
            &serde_json::json!({}),
            &[],
            None,
            None,
            None,
            Some(&cli_tools),
            &["rm".to_string()],
            None,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("deny"));
    }

    #[tokio::test]
    async fn execute_cli_tool_not_found() {
        let cli_tools = std::collections::HashMap::new();
        let result = execute_tool(
            "cli_nonexistent",
            &serde_json::json!({}),
            &[],
            None,
            None,
            None,
            Some(&cli_tools),
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unknown CLI tool"));
    }

    #[tokio::test]
    async fn execute_cli_tool_with_typed_args_template() {
        let mut args = std::collections::HashMap::new();
        args.insert(
            "name".into(),
            crate::config::schema::CliToolArg {
                arg_type: "string".into(),
                description: None,
                required: Some(true),
                default: None,
                enum_values: None,
            },
        );
        let mut cli_tools = std::collections::HashMap::new();
        cli_tools.insert(
            "greet".into(),
            crate::config::schema::CliToolConfig {
                command: "echo hello $name".into(),
                description: "Greet someone".into(),
                args: Some(args),
                permission: None,
                output_format: None,
                max_output_lines: None,
            },
        );
        let result = execute_tool(
            "cli_greet",
            &serde_json::json!({"name": "world"}),
            &[],
            None,
            None,
            None,
            Some(&cli_tools),
            &[],
            None,
        )
        .await
        .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello world"));
    }
}
