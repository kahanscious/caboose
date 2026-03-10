//! Config schema types — project-level configuration options.

use serde::{Deserialize, Serialize};

/// Project-level configuration (.caboose/config.toml).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProjectConfig {
    /// Default provider for this project
    pub provider: Option<String>,
    /// Default model for this project
    pub model: Option<String>,
    /// Custom system prompt for this project
    pub system_prompt: Option<String>,
    /// Tools configuration
    pub tools: Option<ToolsConfig>,
}

/// Configuration for a single CLI tool in the registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolConfig {
    /// Shell command to execute (e.g. "cargo test").
    pub command: String,
    /// Human-readable description shown to the LLM.
    pub description: String,
    /// Optional typed argument schema. Keys are arg names.
    #[serde(default)]
    pub args: Option<std::collections::HashMap<String, CliToolArg>>,
    /// Per-tool permission override: "auto" (default), "always_approve", "deny".
    #[serde(default)]
    pub permission: Option<String>,
    /// Output format hint: "text" (default), "json", "markdown".
    #[serde(default)]
    pub output_format: Option<String>,
    /// Maximum output lines before truncation.
    #[serde(default)]
    pub max_output_lines: Option<usize>,
}

/// A single argument definition for a CLI tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolArg {
    /// JSON Schema type: "string", "boolean", "number", "integer".
    #[serde(rename = "type")]
    pub arg_type: String,
    /// Description shown to the LLM.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether this argument is required.
    #[serde(default)]
    pub required: Option<bool>,
    /// Default value (as string — parsed at execution time).
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// Enum of allowed values.
    #[serde(default, rename = "enum")]
    pub enum_values: Option<Vec<String>>,
}

/// Configuration for a single executable tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutableToolConfig {
    /// Path to the executable (relative to project root or absolute).
    pub path: String,
    /// Timeout in seconds (default: 60).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Per-tool permission override: "auto" (default), "always_approve", "deny".
    #[serde(default)]
    pub permission: Option<String>,
    /// Human-readable description. If omitted, discovered via --schema.
    #[serde(default)]
    pub description: Option<String>,
    /// Typed argument schema. If omitted, discovered via --schema.
    #[serde(default)]
    pub args: Option<std::collections::HashMap<String, CliToolArg>>,
}

/// Tool-specific configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Commands to always allow without approval
    pub allow_commands: Option<Vec<String>>,
    /// Commands to never allow
    pub deny_commands: Option<Vec<String>>,
    /// Additional environment variable names to strip from tool execution
    pub additional_secret_names: Option<Vec<String>>,
    /// CLI tool registry — named tools with shell commands.
    #[serde(default)]
    pub registry: Option<std::collections::HashMap<String, CliToolConfig>>,
    /// Executable tool registry — named tools with JSON stdin/stdout protocol.
    #[serde(default)]
    pub executable: Option<std::collections::HashMap<String, ExecutableToolConfig>>,
}

/// Lifecycle hooks configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// Hooks that fire when a session starts.
    #[serde(default, rename = "SessionStart")]
    pub session_start: Vec<HookEntry>,
    /// Hooks that fire when a session ends.
    #[serde(default, rename = "SessionEnd")]
    pub session_end: Vec<HookEntry>,
    /// Hooks that fire when the user submits a prompt.
    #[serde(default, rename = "UserPromptSubmit")]
    pub user_prompt_submit: Vec<HookEntry>,
    /// Hooks that fire before a tool executes.
    #[serde(default, rename = "PreToolUse")]
    pub pre_tool_use: Vec<HookEntry>,
    /// Hooks that fire after a tool succeeds.
    #[serde(default, rename = "PostToolUse")]
    pub post_tool_use: Vec<HookEntry>,
    /// Hooks that fire after a tool fails.
    #[serde(default, rename = "PostToolUseFailure")]
    pub post_tool_use_failure: Vec<HookEntry>,
    /// Hooks that fire when a permission request would be shown.
    #[serde(default, rename = "PermissionRequest")]
    pub permission_request: Vec<HookEntry>,
    /// Hooks that fire when the agent finishes responding.
    #[serde(default, rename = "Stop")]
    pub stop: Vec<HookEntry>,
    /// Hooks that fire before conversation compaction.
    #[serde(default, rename = "PreCompact")]
    pub pre_compact: Vec<HookEntry>,
    /// Hooks that fire on system notifications.
    #[serde(default, rename = "Notification")]
    pub notification: Vec<HookEntry>,
    /// Hooks that fire when a subagent spawns.
    #[serde(default, rename = "SubagentStart")]
    pub subagent_start: Vec<HookEntry>,
    /// Hooks that fire when a subagent finishes.
    #[serde(default, rename = "SubagentStop")]
    pub subagent_stop: Vec<HookEntry>,
    /// Hooks that fire on periodic repo-level init.
    #[serde(default, rename = "Setup")]
    pub setup: Vec<HookEntry>,
}

/// A single lifecycle hook entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEntry {
    /// Shell command to execute.
    pub command: String,
    /// Timeout in seconds (default: 30).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Only fire for these tool names (empty = all tools).
    #[serde(default)]
    pub match_tools: Option<Vec<String>>,
}

/// Memory system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Enable/disable the memory system entirely.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Enable/disable end-of-session auto-extraction.
    #[serde(default = "default_true")]
    pub auto_extract: bool,
    /// Days to retain raw observations before pruning.
    #[serde(default = "default_30")]
    pub observation_retention_days: u32,
}

fn default_true() -> bool {
    true
}
fn default_30() -> u32 {
    30
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_extract: true,
            observation_retention_days: 30,
        }
    }
}

/// Skills configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Inject skill awareness block into system prompt (default: true).
    #[serde(default = "default_true")]
    pub awareness: bool,
    /// Auto-detect and inject skill hints per turn (default: false).
    #[serde(default)]
    pub auto_hint: bool,
    /// Skill names to disable (case-insensitive).
    #[serde(default)]
    pub disabled: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            awareness: true,
            auto_hint: false,
            disabled: Vec::new(),
        }
    }
}

/// Behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorConfig {
    /// Show handoff prompt when context reaches 90% (default: true).
    #[serde(default = "default_true")]
    pub auto_handoff_prompt: bool,
    /// Model ID to use for compaction summarization. If unset, uses the active model.
    #[serde(default)]
    pub compaction_model: Option<String>,
    /// Number of recent tool results to keep inline (default: 10).
    /// Older results are moved to cold storage with compact stubs.
    #[serde(default)]
    pub hot_tail_size: Option<u32>,
    /// Maximum session cost in USD. When reached, agent pauses before sending next request.
    #[serde(default)]
    pub max_session_cost: Option<f64>,
    /// Context usage fraction at which auto-compaction triggers (default: 1.0 = 100%).
    /// Lower values compact earlier, saving cost but losing detail sooner.
    #[serde(default)]
    pub compaction_threshold: Option<f64>,
    /// Context usage fraction at which the handoff prompt appears (default: 0.9 = 90%).
    #[serde(default)]
    pub handoff_threshold: Option<f64>,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            auto_handoff_prompt: true,
            compaction_model: None,
            hot_tail_size: None,
            max_session_cost: None,
            compaction_threshold: None,
            handoff_threshold: None,
        }
    }
}

/// LSP configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    /// Enable/disable LSP entirely.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Per-language server configuration.
    #[serde(default)]
    pub servers: std::collections::HashMap<String, LspServerConfig>,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            servers: std::collections::HashMap::new(),
        }
    }
}

/// Configuration for a single LSP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerConfig {
    /// Command to spawn the LSP server process.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the server process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Disable this server (default: false).
    #[serde(default)]
    pub disabled: bool,
}

/// MCP servers configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::HashMap<String, McpServerConfig>,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to spawn the MCP server process.
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the server process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Disable this server (default: false).
    #[serde(default)]
    pub disabled: bool,
    /// Whether this server has been removed by the user (default: false).
    /// For presets, this prevents them from reappearing.
    #[serde(default)]
    pub removed: bool,
}

/// Roundhouse (multi-LLM planning) configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoundhouseSchemaConfig {
    /// Timeout for each secondary LLM during planning (seconds)
    pub planning_timeout: Option<u64>,
    /// Max tokens per secondary LLM during planning
    pub per_llm_token_budget: Option<u64>,
}

/// Circuits (scheduled recurring tasks) configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CircuitsConfig {
    /// Max concurrent circuits (default 5)
    pub max_concurrent: Option<usize>,
    /// Enable persistent circuits / daemon (default true)
    pub persistent_enabled: Option<bool>,
}

/// Configuration for a local LLM provider instance.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalProviderConfig {
    /// Provider type: "ollama", "lmstudio", "llamacpp", "custom"
    pub provider_type: String,
    /// Server address (e.g. "http://localhost:11434")
    pub address: String,
    /// Selected model name
    pub model: Option<String>,
    /// Display name for UI
    pub display_name: Option<String>,
}

/// SCM (GitHub/GitLab) integration configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScmConfig {
    /// Preferred SCM provider (auto-detected if not set)
    pub provider: Option<String>,
    /// Enable SCM tools (default true)
    pub enabled: Option<bool>,
}

/// External service configuration (web search, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServicesConfig {
    /// Named services: key is service name (e.g. "web_search"), value is config.
    #[serde(default, flatten)]
    pub services: std::collections::HashMap<String, ServiceConfig>,
}

/// Configuration for a single external service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Provider backend (e.g. "tavily").
    pub provider: String,
    /// Environment variable name holding the API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Enable/disable this service (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_config_roundtrip() {
        let cfg = LocalProviderConfig {
            provider_type: "ollama".to_string(),
            address: "http://localhost:11434".to_string(),
            model: Some("llama3".to_string()),
            display_name: Some("My Ollama".to_string()),
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        let parsed: LocalProviderConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.provider_type, "ollama");
        assert_eq!(parsed.address, "http://localhost:11434");
        assert_eq!(parsed.model.as_deref(), Some("llama3"));
    }

    #[test]
    fn parse_memory_config_defaults() {
        let config: MemoryConfig = toml::from_str("").unwrap();
        assert!(config.enabled);
        assert!(config.auto_extract);
        assert_eq!(config.observation_retention_days, 30);
    }

    #[test]
    fn parse_memory_config_disabled() {
        let toml_str = r#"
enabled = false
auto_extract = false
observation_retention_days = 7
"#;
        let config: MemoryConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.enabled);
        assert!(!config.auto_extract);
        assert_eq!(config.observation_retention_days, 7);
    }

    #[test]
    fn parse_mcp_server_config() {
        let toml_str = r#"
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "ghp_test123" }
"#;
        let config: McpConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.servers.len(), 2);

        let fs = &config.servers["filesystem"];
        assert_eq!(fs.command, "npx");
        assert_eq!(fs.args.len(), 3);
        assert!(fs.env.is_empty());

        let gh = &config.servers["github"];
        assert_eq!(gh.command, "npx");
        assert_eq!(gh.env["GITHUB_TOKEN"], "ghp_test123");
    }

    #[test]
    fn parse_empty_mcp_config() {
        let toml_str = "";
        let config: McpConfig = toml::from_str(toml_str).unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn parse_skills_config() {
        let toml_str = r#"
awareness = true
auto_hint = false
disabled = ["review", "test"]
"#;
        let config: SkillsConfig = toml::from_str(toml_str).unwrap();
        assert!(config.awareness);
        assert!(!config.auto_hint);
        assert_eq!(config.disabled, vec!["review", "test"]);
    }

    #[test]
    fn parse_skills_config_defaults() {
        let config: SkillsConfig = toml::from_str("").unwrap();
        assert!(config.awareness);
        assert!(!config.auto_hint);
        assert!(config.disabled.is_empty());
    }

    #[test]
    fn parse_behavior_config_defaults() {
        let config: BehaviorConfig = toml::from_str("").unwrap();
        assert!(config.auto_handoff_prompt);
    }

    #[test]
    fn parse_behavior_config_disabled() {
        let toml_str = "auto_handoff_prompt = false";
        let config: BehaviorConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.auto_handoff_prompt);
    }

    #[test]
    fn parse_behavior_compaction_model_set() {
        let toml_str = r#"compaction_model = "claude-haiku-4-5-20251001""#;
        let config: BehaviorConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.compaction_model.as_deref(),
            Some("claude-haiku-4-5-20251001")
        );
    }

    #[test]
    fn parse_behavior_compaction_model_unset() {
        let config: BehaviorConfig = toml::from_str("").unwrap();
        assert!(config.compaction_model.is_none());
    }

    #[test]
    fn parse_behavior_hot_tail_size() {
        let toml_str = "hot_tail_size = 20";
        let config: BehaviorConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hot_tail_size, Some(20));
    }

    #[test]
    fn parse_behavior_hot_tail_size_unset() {
        let config: BehaviorConfig = toml::from_str("").unwrap();
        assert!(config.hot_tail_size.is_none());
    }

    #[test]
    fn parse_behavior_max_session_cost() {
        let toml_str = "max_session_cost = 5.0";
        let config: BehaviorConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_session_cost, Some(5.0));
    }

    #[test]
    fn parse_behavior_max_session_cost_unset() {
        let config: BehaviorConfig = toml::from_str("").unwrap();
        assert!(config.max_session_cost.is_none());
    }

    #[test]
    fn parse_behavior_compaction_threshold() {
        let toml_str = "compaction_threshold = 0.75";
        let config: BehaviorConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.compaction_threshold, Some(0.75));
    }

    #[test]
    fn parse_behavior_handoff_threshold() {
        let toml_str = "handoff_threshold = 0.80";
        let config: BehaviorConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.handoff_threshold, Some(0.80));
    }

    #[test]
    fn parse_behavior_thresholds_unset() {
        let config: BehaviorConfig = toml::from_str("").unwrap();
        assert!(config.compaction_threshold.is_none());
        assert!(config.handoff_threshold.is_none());
    }

    #[test]
    fn parse_lsp_config() {
        let toml_str = r#"
[servers.typescript]
command = "typescript-language-server"
args = ["--stdio"]

[servers.rust]
command = "rust-analyzer"
disabled = true
"#;
        let config: LspConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.servers.len(), 2);
        assert_eq!(
            config.servers["typescript"].command,
            "typescript-language-server"
        );
        assert_eq!(config.servers["typescript"].args, vec!["--stdio"]);
        assert!(!config.servers["typescript"].disabled);
        assert!(config.servers["rust"].disabled);
    }

    #[test]
    fn parse_lsp_config_defaults() {
        let config: LspConfig = toml::from_str("").unwrap();
        assert!(config.enabled);
        assert!(config.servers.is_empty());
    }

    #[test]
    fn parse_server_without_optional_fields() {
        let toml_str = r#"
[servers.simple]
command = "my-mcp-server"
"#;
        let config: McpConfig = toml::from_str(toml_str).unwrap();
        let simple = &config.servers["simple"];
        assert_eq!(simple.command, "my-mcp-server");
        assert!(simple.args.is_empty());
        assert!(simple.env.is_empty());
        assert!(!simple.disabled);
    }

    #[test]
    fn parse_mcp_server_removed() {
        let toml_str = r#"
[servers.old]
command = "npx"
args = ["-y", "some-server"]
removed = true
"#;
        let config: McpConfig = toml::from_str(toml_str).unwrap();
        assert!(config.servers["old"].removed);
    }

    #[test]
    fn parse_services_config() {
        let toml_str = r#"
[web_search]
provider = "tavily"
api_key_env = "TAVILY_API_KEY"
"#;
        let config: ServicesConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.services.len(), 1);
        let ws = &config.services["web_search"];
        assert_eq!(ws.provider, "tavily");
        assert_eq!(ws.api_key_env.as_deref(), Some("TAVILY_API_KEY"));
        assert!(ws.enabled);
    }

    #[test]
    fn parse_services_config_disabled() {
        let toml_str = r#"
[web_search]
provider = "tavily"
api_key_env = "TAVILY_API_KEY"
enabled = false
"#;
        let config: ServicesConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.services["web_search"].enabled);
    }

    #[test]
    fn parse_services_config_empty() {
        let config: ServicesConfig = toml::from_str("").unwrap();
        assert!(config.services.is_empty());
    }

    #[test]
    fn parse_mcp_server_disabled() {
        let toml_str = r#"
[servers.ctx]
command = "npx"
args = ["-y", "@upstash/context7-mcp@latest"]
disabled = true
"#;
        let config: McpConfig = toml::from_str(toml_str).unwrap();
        assert!(config.servers["ctx"].disabled);
    }

    #[test]
    fn parse_cli_tool_registry() {
        let toml_str = r#"
[registry.test]
command = "cargo test"
description = "Run tests"

[registry.deploy]
command = "make deploy"
description = "Deploy to env"
permission = "always_approve"
output_format = "text"
max_output_lines = 200

[registry.deploy.args]
environment = { type = "string", description = "Target", required = true }
dry_run = { type = "boolean", description = "Preview", default = false }
"#;
        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        let registry = config.registry.unwrap();
        assert_eq!(registry.len(), 2);

        let test_tool = &registry["test"];
        assert_eq!(test_tool.command, "cargo test");
        assert!(test_tool.args.is_none());
        assert!(test_tool.permission.is_none());

        let deploy = &registry["deploy"];
        assert_eq!(deploy.command, "make deploy");
        assert_eq!(deploy.permission.as_deref(), Some("always_approve"));
        assert_eq!(deploy.output_format.as_deref(), Some("text"));
        assert_eq!(deploy.max_output_lines, Some(200));

        let args = deploy.args.as_ref().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args["environment"].arg_type, "string");
        assert!(args["environment"].required.unwrap_or(false));
    }

    #[test]
    fn parse_tools_config_without_registry() {
        let toml_str = r#"
allow_commands = ["ls"]
"#;
        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert!(config.registry.is_none());
    }

    #[test]
    fn parse_cli_tool_minimal() {
        let toml_str = r#"
[registry.hello]
command = "echo hello"
description = "Say hello"
"#;
        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        let registry = config.registry.unwrap();
        let hello = &registry["hello"];
        assert_eq!(hello.command, "echo hello");
        assert!(hello.args.is_none());
        assert!(hello.permission.is_none());
        assert!(hello.output_format.is_none());
        assert!(hello.max_output_lines.is_none());
    }

    #[test]
    fn parse_executable_tool_config() {
        let toml_str = r#"
[executable.db_query]
path = ".caboose/tools/db-query.py"
timeout = 120

[executable.lint]
path = ".caboose/tools/lint.sh"
permission = "deny"
description = "Run linter on a file"
[executable.lint.args]
file = { type = "string", required = true }
"#;
        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        let exec = config.executable.unwrap();
        assert_eq!(exec.len(), 2);

        let db = &exec["db_query"];
        assert_eq!(db.path, ".caboose/tools/db-query.py");
        assert_eq!(db.timeout, Some(120));
        assert!(db.description.is_none());
        assert!(db.args.is_none());

        let lint = &exec["lint"];
        assert_eq!(lint.path, ".caboose/tools/lint.sh");
        assert_eq!(lint.permission.as_deref(), Some("deny"));
        assert_eq!(lint.description.as_deref(), Some("Run linter on a file"));
        let args = lint.args.as_ref().unwrap();
        assert_eq!(args["file"].arg_type, "string");
        assert!(args["file"].required.unwrap_or(false));
    }

    #[test]
    fn parse_tools_config_without_executable() {
        let toml_str = r#"
allow_commands = ["ls"]
"#;
        let config: ToolsConfig = toml::from_str(toml_str).unwrap();
        assert!(config.executable.is_none());
    }

    #[test]
    fn parse_hooks_config() {
        let toml_str = r#"
[[PreToolUse]]
command = ".caboose/hooks/audit.sh"
timeout = 10

[[PreToolUse]]
command = ".caboose/hooks/block.py"
match_tools = ["write_file", "edit_file"]

[[SessionStart]]
command = ".caboose/hooks/load-env.sh"
"#;
        let config: HooksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.pre_tool_use.len(), 2);
        assert_eq!(config.pre_tool_use[0].command, ".caboose/hooks/audit.sh");
        assert_eq!(config.pre_tool_use[0].timeout, Some(10));
        assert!(
            config.pre_tool_use[1]
                .match_tools
                .as_ref()
                .unwrap()
                .contains(&"write_file".to_string())
        );
        assert_eq!(config.session_start.len(), 1);
    }

    #[test]
    fn parse_hooks_config_with_stop() {
        let toml_str = r#"
[[Stop]]
command = ".caboose/hooks/keep-going.sh"
"#;
        let config: HooksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.stop.len(), 1);
        assert_eq!(config.stop[0].command, ".caboose/hooks/keep-going.sh");
    }

    #[test]
    fn parse_hooks_config_with_pre_compact() {
        let toml_str = r#"
[[PreCompact]]
command = ".caboose/hooks/preserve-context.sh"
"#;
        let config: HooksConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.pre_compact.len(), 1);
    }

    #[test]
    fn parse_hooks_config_empty() {
        let config: HooksConfig = toml::from_str("").unwrap();
        assert!(config.pre_tool_use.is_empty());
        assert!(config.session_start.is_empty());
    }
}
