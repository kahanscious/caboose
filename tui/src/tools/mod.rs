//! Tool registry — definitions and dispatch for all agent tools.

pub mod diagnostics;
pub mod executable;
pub mod fetch;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod patch;
pub mod read;
pub mod shell;
pub mod web_search;
pub mod write;

use crate::provider::ToolDefinition;

/// Registry of available tools. Provides tool definitions to send to the LLM.
pub struct ToolRegistry {
    tools: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub fn new(
        cli_tools: Option<&std::collections::HashMap<String, crate::config::schema::CliToolConfig>>,
        exec_tools: Option<
            &std::collections::HashMap<String, crate::config::schema::ExecutableToolConfig>,
        >,
        scm_provider: &crate::scm::detection::ScmProvider,
    ) -> Self {
        let mut result = Self {
            tools: vec![
                tool_def(
                    "read_file",
                    "Read the contents of a file",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to read" },
                            "offset": { "type": "integer", "description": "Line offset to start reading from (0-indexed). Use with limit for targeted reads." },
                            "limit": { "type": "integer", "description": "Maximum number of lines to read (default: 200). Use with offset for targeted reads of large files." }
                        },
                        "required": ["path"]
                    }),
                ),
                tool_def(
                    "write_file",
                    "Create a new file or completely replace a file's contents. IMPORTANT: Only use this for creating new files or when you intend to replace the entire file. For modifying existing files (adding lines, changing sections, fixing code), use edit_file or apply_patch instead — they preserve existing content.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to write" },
                            "content": { "type": "string", "description": "Complete file content" }
                        },
                        "required": ["path", "content"]
                    }),
                ),
                tool_def(
                    "edit_file",
                    "Make targeted edits to an existing file using search/replace. PREFERRED for modifying files — preserves all content outside the matched region. Use read_file first to see the exact text to match in old_string. For inserting at a location, set old_string to the text before the insertion point and new_string to that same text plus the new content.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to edit" },
                            "old_string": { "type": "string", "description": "Exact text to find (must match uniquely)" },
                            "new_string": { "type": "string", "description": "Replacement text" }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }),
                ),
                tool_def(
                    "glob",
                    "Find files matching a glob pattern",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "pattern": { "type": "string", "description": "Glob pattern" },
                            "path": { "type": "string", "description": "Root directory" }
                        },
                        "required": ["pattern"]
                    }),
                ),
                tool_def(
                    "grep",
                    "Search file contents with regex",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "pattern": { "type": "string", "description": "Regex pattern" },
                            "path": { "type": "string", "description": "Directory to search" },
                            "include": { "type": "string", "description": "File glob filter" }
                        },
                        "required": ["pattern"]
                    }),
                ),
                tool_def(
                    "run_command",
                    "Execute a shell command",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "command": { "type": "string", "description": "Shell command to run" },
                            "timeout": { "type": "integer", "description": "Timeout in milliseconds" }
                        },
                        "required": ["command"]
                    }),
                ),
                tool_def(
                    "list_directory",
                    "List files and directories",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Directory path" }
                        },
                        "required": ["path"]
                    }),
                ),
                tool_def(
                    "apply_patch",
                    "Apply a unified diff to one or more files. Preferred over edit_file when making changes to multiple files or multiple locations in a single file. The diff must be in standard unified diff format with `--- a/path` and `+++ b/path` headers. Always read_file before generating a diff to ensure accurate context.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "diff": { "type": "string", "description": "Standard unified diff text. Multi-file supported." }
                        },
                        "required": ["diff"]
                    }),
                ),
                tool_def(
                    "fetch",
                    "Fetch content from a URL",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "url": { "type": "string", "description": "URL to fetch" }
                        },
                        "required": ["url"]
                    }),
                ),
                tool_def(
                    "web_search",
                    "Search the web for current information. Returns relevant results with titles, URLs, and snippets. Use this when you need up-to-date information that may not be in your training data.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Search query" }
                        },
                        "required": ["query"]
                    }),
                ),
                tool_def(
                    "diagnostics",
                    "Get compiler/linter diagnostics (errors, warnings) via LSP. If path is provided, returns diagnostics for that file. If path is omitted, returns all known diagnostics across the workspace (files the LSP has already analyzed).",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "File path to check. If omitted, returns diagnostics for all analyzed files." }
                        },
                        "required": []
                    }),
                ),
                tool_def(
                    "lsp",
                    "Interact with Language Server Protocol (LSP) servers for code intelligence.\n\n\
                     Supported operations:\n\
                     - goToDefinition: Find where a symbol is defined (requires path, line, character)\n\
                     - findReferences: Find all references to a symbol (requires path, line, character)\n\
                     - hover: Get type/doc info for a symbol (requires path, line, character)\n\
                     - goToImplementation: Find implementations of a trait/interface (requires path, line, character)\n\
                     - documentSymbol: List all symbols in a file (requires path)\n\
                     - workspaceSymbol: Search symbols across the workspace (requires query)\n\n\
                     Line and character are 1-based (as shown in editors).",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "operation": {
                                "type": "string",
                                "description": "The LSP operation to perform.",
                                "enum": ["goToDefinition", "findReferences", "hover", "goToImplementation", "documentSymbol", "workspaceSymbol"]
                            },
                            "path": {
                                "type": "string",
                                "description": "File path. Required for all operations except workspaceSymbol."
                            },
                            "line": {
                                "type": "integer",
                                "description": "Line number (1-based). Required for position-based operations."
                            },
                            "character": {
                                "type": "integer",
                                "description": "Character offset (1-based). Required for position-based operations."
                            },
                            "query": {
                                "type": "string",
                                "description": "Search query. Required for workspaceSymbol."
                            }
                        },
                        "required": ["operation"]
                    }),
                ),
                tool_def(
                    "todo_write",
                    "Create or update the task outline displayed to the user. Use for multi-step tasks to show progress. Each call replaces the entire task list.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "todos": {
                                "type": "array",
                                "description": "The task list. Each call replaces the entire list.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "content": { "type": "string", "description": "Imperative form: what to do (e.g. 'Run tests')" },
                                        "active_form": { "type": "string", "description": "Present participle: shown when in progress (e.g. 'Running tests')" },
                                        "status": { "type": "string", "enum": ["pending", "in_progress", "completed", "cancelled"], "description": "Task status" }
                                    },
                                    "required": ["content", "active_form", "status"]
                                }
                            }
                        },
                        "required": ["todos"]
                    }),
                ),
                tool_def(
                    "todo_read",
                    "Read the current task outline. Returns the current task list as JSON, or empty if no tasks exist.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {},
                        "required": []
                    }),
                ),
                tool_def(
                    "ask_user",
                    "Ask the user a question and wait for their answer. Use this to get clarifying input, preferences, or decisions. Supports structured multiple-choice questions. Each question can have 2-8 options. The user can always provide a custom answer instead of choosing an option.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "questions": {
                                "type": "array",
                                "description": "1-8 questions to ask the user.",
                                "minItems": 1,
                                "maxItems": 8,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "question": { "type": "string", "description": "The full question text to display" },
                                        "header": { "type": "string", "description": "Short label (max 30 chars) for display" },
                                        "options": {
                                            "type": "array",
                                            "description": "2-8 answer choices",
                                            "minItems": 2,
                                            "maxItems": 8,
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "label": { "type": "string", "description": "Short option label (1-5 words)" },
                                                    "description": { "type": "string", "description": "What this option means" }
                                                },
                                                "required": ["label", "description"]
                                            }
                                        },
                                        "multiSelect": { "type": "boolean", "description": "Allow selecting multiple options (default: false)" }
                                    },
                                    "required": ["question", "header", "options"]
                                }
                            }
                        },
                        "required": ["questions"]
                    }),
                ),
                tool_def(
                    "spawn_agent",
                    "Spawn an autonomous sub-agent that works in an isolated git worktree. Use this when you identify tasks that can be done in parallel without conflicting file edits. Each sub-agent gets its own branch and worktree, runs independently, and its changes are merged back when complete. The sub-agent inherits the current permission mode. Do NOT spawn agents for tasks that depend on each other's output — those must be sequential.",
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "task": {
                                "type": "string",
                                "description": "A clear, self-contained description of the work to do. Include enough context for the agent to work independently (file paths, function names, expected behavior)."
                            }
                        },
                        "required": ["task"]
                    }),
                ),
            ],
        };

        // Append CLI tool definitions from config
        if let Some(registry) = cli_tools {
            let mut sorted_keys: Vec<&String> = registry.keys().collect();
            sorted_keys.sort(); // deterministic order
            for key in sorted_keys {
                let cli = &registry[key];
                let name = format!("cli_{key}");
                let description = format!(
                    "{}\n\nThis runs the shell command: `{}`",
                    cli.description, cli.command
                );

                let input_schema = if let Some(ref args) = cli.args {
                    // Build typed schema from arg definitions
                    let mut properties = serde_json::Map::new();
                    let mut required = Vec::new();
                    for (arg_name, arg) in args {
                        let mut prop = serde_json::Map::new();
                        prop.insert("type".into(), serde_json::json!(arg.arg_type));
                        if let Some(ref desc) = arg.description {
                            prop.insert("description".into(), serde_json::json!(desc));
                        }
                        if let Some(ref enum_vals) = arg.enum_values {
                            prop.insert("enum".into(), serde_json::json!(enum_vals));
                        }
                        properties.insert(arg_name.clone(), serde_json::Value::Object(prop));
                        if arg.required.unwrap_or(false) {
                            required.push(serde_json::json!(arg_name));
                        }
                    }
                    serde_json::json!({
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    })
                } else {
                    // Simple mode: single optional "args" string
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "args": { "type": "string", "description": "Additional arguments" }
                        },
                        "required": []
                    })
                };

                result
                    .tools
                    .push(tool_def(&name, &description, input_schema));
            }
        }

        // Append executable tool definitions from config
        if let Some(executables) = exec_tools {
            let mut sorted_keys: Vec<&String> = executables.keys().collect();
            sorted_keys.sort();
            for key in sorted_keys {
                let exec = &executables[key];
                let name = format!("exec_{key}");

                let description = exec
                    .description
                    .as_deref()
                    .unwrap_or("Executable tool (run with --schema to discover)");
                let description =
                    format!("{description}\n\nThis runs the executable: `{}`", exec.path);

                let input_schema = if let Some(ref args) = exec.args {
                    let mut properties = serde_json::Map::new();
                    let mut required = Vec::new();
                    for (arg_name, arg) in args {
                        let mut prop = serde_json::Map::new();
                        prop.insert("type".into(), serde_json::json!(arg.arg_type));
                        if let Some(ref desc) = arg.description {
                            prop.insert("description".into(), serde_json::json!(desc));
                        }
                        if let Some(ref enum_vals) = arg.enum_values {
                            prop.insert("enum".into(), serde_json::json!(enum_vals));
                        }
                        properties.insert(arg_name.clone(), serde_json::Value::Object(prop));
                        if arg.required.unwrap_or(false) {
                            required.push(serde_json::json!(arg_name));
                        }
                    }
                    serde_json::json!({
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    })
                } else {
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "input": { "type": "string", "description": "Input to pass to the tool" }
                        },
                        "required": []
                    })
                };

                result
                    .tools
                    .push(tool_def(&name, &description, input_schema));
            }
        }

        // Append SCM tool definitions if provider detected and CLI available
        match scm_provider {
            crate::scm::detection::ScmProvider::GitHub if crate::scm::detection::has_gh_cli() => {
                for tool in crate::scm::tools::github_tool_definitions() {
                    result.tools.push(tool);
                }
            }
            crate::scm::detection::ScmProvider::GitLab if crate::scm::detection::has_glab_cli() => {
                for tool in crate::scm::tools::gitlab_tool_definitions() {
                    result.tools.push(tool);
                }
            }
            _ => {}
        }

        result
    }

    /// Get tool definitions to send to the LLM provider.
    pub fn definitions(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Get tool definitions, optionally excluding interactive tools
    /// that require tool-calling support (like ask_user).
    #[allow(dead_code)]
    pub fn definitions_for_model(&self, supports_tools: bool) -> Vec<&ToolDefinition> {
        if supports_tools {
            self.tools.iter().collect()
        } else {
            self.tools.iter().filter(|d| d.name != "ask_user").collect()
        }
    }
}

fn tool_def(name: &str, description: &str, input_schema: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

/// Tool definition for `generate_skill` — used only during skill creation.
/// Not part of the default ToolRegistry; added conditionally when creation mode is active.
pub fn generate_skill_tool_def() -> ToolDefinition {
    tool_def(
        "generate_skill",
        "Generate the final skill template after gathering enough context from the user. \
         Call this tool when you have enough information to produce a complete, well-structured skill.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "skillContent": {
                    "type": "string",
                    "description": "The complete skill template in markdown format, including YAML frontmatter."
                },
                "companionFilesJson": {
                    "type": "string",
                    "description": "Optional JSON array of companion files: [{\"name\":\"examples.md\",\"content\":\"...\"}]. Use for large reference material that should be $INCLUDE()d from the main skill."
                }
            },
            "required": ["skillContent"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_includes_todo_write() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(
            defs.iter().any(|d| d.name == "todo_write"),
            "todo_write tool not found in registry"
        );
    }

    #[test]
    fn registry_includes_todo_read() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(
            defs.iter().any(|d| d.name == "todo_read"),
            "todo_read tool not found in registry"
        );
    }

    #[test]
    fn generate_skill_tool_definition_exists() {
        let def = super::generate_skill_tool_def();
        assert_eq!(def.name, "generate_skill");
        assert!(
            def.input_schema
                .get("properties")
                .unwrap()
                .get("skillContent")
                .is_some()
        );
    }

    #[test]
    fn registry_includes_ask_user() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(
            defs.iter().any(|d| d.name == "ask_user"),
            "ask_user tool not found in registry"
        );
    }

    #[test]
    fn ask_user_schema_has_questions_array() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        let ask_user = defs.iter().find(|d| d.name == "ask_user").unwrap();
        let props = ask_user.input_schema.get("properties").unwrap();
        let questions = props.get("questions").unwrap();
        assert_eq!(questions.get("type").unwrap(), "array");
    }

    #[test]
    fn registry_includes_diagnostics() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(
            defs.iter().any(|d| d.name == "diagnostics"),
            "diagnostics tool not found in registry"
        );
    }

    #[test]
    fn ask_user_excluded_when_tools_not_supported() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions_for_model(false);
        assert!(!defs.iter().any(|d| d.name == "ask_user"));
    }

    #[test]
    fn ask_user_included_when_tools_supported() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions_for_model(true);
        assert!(defs.iter().any(|d| d.name == "ask_user"));
    }

    #[test]
    fn registry_includes_web_search() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(
            defs.iter().any(|d| d.name == "web_search"),
            "web_search tool not found in registry"
        );
    }

    #[test]
    fn registry_includes_cli_tools_simple() {
        let mut cli_tools = std::collections::HashMap::new();
        cli_tools.insert(
            "test".into(),
            crate::config::schema::CliToolConfig {
                command: "cargo test".into(),
                description: "Run tests".into(),
                args: None,
                permission: None,
                output_format: None,
                max_output_lines: None,
            },
        );
        let registry = ToolRegistry::new(
            Some(&cli_tools),
            None,
            &crate::scm::detection::ScmProvider::Unknown,
        );
        let defs = registry.definitions();
        let cli_test = defs
            .iter()
            .find(|d| d.name == "cli_test")
            .expect("cli_test not found");
        assert!(cli_test.description.contains("Run tests"));
        assert!(cli_test.description.contains("cargo test"));
    }

    #[test]
    fn registry_includes_cli_tools_with_typed_args() {
        let mut args = std::collections::HashMap::new();
        args.insert(
            "environment".into(),
            crate::config::schema::CliToolArg {
                arg_type: "string".into(),
                description: Some("Target env".into()),
                required: Some(true),
                default: None,
                enum_values: Some(vec!["staging".into(), "production".into()]),
            },
        );
        args.insert(
            "dry_run".into(),
            crate::config::schema::CliToolArg {
                arg_type: "boolean".into(),
                description: Some("Preview mode".into()),
                required: None,
                default: None,
                enum_values: None,
            },
        );

        let mut cli_tools = std::collections::HashMap::new();
        cli_tools.insert(
            "deploy".into(),
            crate::config::schema::CliToolConfig {
                command: "make deploy".into(),
                description: "Deploy".into(),
                args: Some(args),
                permission: None,
                output_format: None,
                max_output_lines: None,
            },
        );
        let registry = ToolRegistry::new(
            Some(&cli_tools),
            None,
            &crate::scm::detection::ScmProvider::Unknown,
        );
        let defs = registry.definitions();
        let deploy = defs
            .iter()
            .find(|d| d.name == "cli_deploy")
            .expect("cli_deploy not found");

        let props = deploy.input_schema.get("properties").unwrap();
        assert!(props.get("environment").is_some());
        assert!(props.get("dry_run").is_some());

        let env_prop = props.get("environment").unwrap();
        assert_eq!(env_prop.get("type").unwrap(), "string");

        let required = deploy
            .input_schema
            .get("required")
            .unwrap()
            .as_array()
            .unwrap();
        let req_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(req_strs.contains(&"environment"));
        assert!(!req_strs.contains(&"dry_run"));
    }

    #[test]
    fn registry_works_without_cli_tools() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(!defs.iter().any(|d| d.name.starts_with("cli_")));
        assert!(defs.iter().any(|d| d.name == "read_file")); // builtins still there
    }

    #[test]
    fn registry_includes_exec_tools_from_config() {
        let mut exec_tools = std::collections::HashMap::new();
        exec_tools.insert(
            "query".into(),
            crate::config::schema::ExecutableToolConfig {
                path: ".caboose/tools/query.py".into(),
                timeout: Some(30),
                permission: None,
                description: Some("Query database".into()),
                args: None,
            },
        );
        let registry = ToolRegistry::new(
            None,
            Some(&exec_tools),
            &crate::scm::detection::ScmProvider::Unknown,
        );
        let defs = registry.definitions();
        let exec_query = defs
            .iter()
            .find(|d| d.name == "exec_query")
            .expect("exec_query not found");
        assert!(exec_query.description.contains("Query database"));
    }

    #[test]
    fn registry_includes_exec_tools_with_typed_args() {
        let mut args = std::collections::HashMap::new();
        args.insert(
            "sql".into(),
            crate::config::schema::CliToolArg {
                arg_type: "string".into(),
                description: Some("SQL query".into()),
                required: Some(true),
                default: None,
                enum_values: None,
            },
        );
        let mut exec_tools = std::collections::HashMap::new();
        exec_tools.insert(
            "db".into(),
            crate::config::schema::ExecutableToolConfig {
                path: "db.sh".into(),
                timeout: None,
                permission: None,
                description: Some("Database tool".into()),
                args: Some(args),
            },
        );
        let registry = ToolRegistry::new(
            None,
            Some(&exec_tools),
            &crate::scm::detection::ScmProvider::Unknown,
        );
        let defs = registry.definitions();
        let db = defs
            .iter()
            .find(|d| d.name == "exec_db")
            .expect("exec_db not found");
        let props = db.input_schema.get("properties").unwrap();
        assert!(props.get("sql").is_some());
    }

    #[test]
    fn registry_includes_spawn_agent() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        let spawn = defs.iter().find(|d| d.name == "spawn_agent").expect("spawn_agent not found");
        let props = spawn.input_schema.get("properties").unwrap();
        assert!(props.get("task").is_some());
        let required = spawn.input_schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("task")));
    }

    #[test]
    fn registry_works_without_exec_tools() {
        let registry = ToolRegistry::new(None, None, &crate::scm::detection::ScmProvider::Unknown);
        let defs = registry.definitions();
        assert!(!defs.iter().any(|d| d.name.starts_with("exec_")));
    }

    #[test]
    fn full_config_to_registry_round_trip() {
        let toml_str = r#"
allow_commands = ["cargo"]
deny_commands = ["rm"]

[registry.test]
command = "cargo test"
description = "Run project tests"

[registry.lint]
command = "cargo clippy"
description = "Run linter"
"#;
        let tools_config: crate::config::schema::ToolsConfig = toml::from_str(toml_str).unwrap();
        let registry = ToolRegistry::new(
            tools_config.registry.as_ref(),
            None,
            &crate::scm::detection::ScmProvider::Unknown,
        );
        let defs = registry.definitions();

        // Built-in tools still present
        assert!(defs.iter().any(|d| d.name == "read_file"));
        assert!(defs.iter().any(|d| d.name == "run_command"));

        // CLI tools present with prefix
        assert!(defs.iter().any(|d| d.name == "cli_test"));
        assert!(defs.iter().any(|d| d.name == "cli_lint"));

        // CLI tools have correct descriptions
        let cli_test = defs.iter().find(|d| d.name == "cli_test").unwrap();
        assert!(cli_test.description.contains("Run project tests"));
        assert!(cli_test.description.contains("cargo test"));
    }
}
