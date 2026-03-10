//! MCP server lifecycle management.

use std::collections::HashMap;

use crate::agent::tools::ToolResult;
use crate::config::schema::{McpConfig, McpServerConfig};
use crate::provider::ToolDefinition;

/// A validated MCP tool call ready to execute on a background task.
/// Contains a cloned `Peer` so it can be sent across threads.
pub struct McpPreparedCall {
    peer: rmcp::service::Peer<rmcp::RoleClient>,
    tool_name: String,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
    full_name: String,
}

impl McpPreparedCall {
    /// Execute the tool call with a 30s timeout. Safe to run on a spawned task.
    pub async fn execute(self) -> ToolResult {
        let call_fut = self.peer.call_tool(rmcp::model::CallToolRequestParams {
            meta: None,
            name: self.tool_name.into(),
            arguments: self.arguments,
            task: None,
        });
        let result = match tokio::time::timeout(std::time::Duration::from_secs(30), call_fut).await
        {
            Ok(r) => r,
            Err(_) => {
                return ToolResult {
                    tool_use_id: String::new(),
                    output: format!("MCP tool call timed out after 30s: {}", self.full_name),
                    is_error: true,
                    tool_name: Some(self.full_name),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }
        };

        match result {
            Ok(tool_result) => {
                let output = tool_result
                    .content
                    .iter()
                    .map(|c| match &c.raw {
                        rmcp::model::RawContent::Text(t) => t.text.clone(),
                        _ => "[non-text content]".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ToolResult {
                    tool_use_id: String::new(),
                    output,
                    is_error: tool_result.is_error.unwrap_or(false),
                    tool_name: Some(self.full_name),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                }
            }
            Err(e) => ToolResult {
                tool_use_id: String::new(),
                output: format!("MCP tool call failed: {e}"),
                is_error: true,
                tool_name: Some(self.full_name),
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            },
        }
    }
}

/// Status of an MCP server connection.
#[derive(Debug, Clone)]
pub enum ServerStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

impl ServerStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Error(_) => "error",
        }
    }
}

/// A connected (or pending) MCP server.
pub struct McpServer {
    pub name: String,
    pub config: McpServerConfig,
    pub status: ServerStatus,
    /// Whether this server is a built-in preset (vs user-defined).
    pub is_preset: bool,
    /// Tools discovered from this server, namespaced as `server_name:tool_name`.
    pub tools: Vec<ToolDefinition>,
    /// The rmcp service handle (if connected).
    pub service: Option<rmcp::service::RunningService<rmcp::RoleClient, ()>>,
}

/// Parse a namespaced tool name "server:tool" into (server, tool).
pub fn parse_tool_name(name: &str) -> Option<(&str, &str)> {
    let colon = name.find(':')?;
    if colon == 0 || colon == name.len() - 1 {
        return None;
    }
    Some((&name[..colon], &name[colon + 1..]))
}

/// Result of a successful background MCP connection.
pub struct McpConnectResult {
    pub tools: Vec<ToolDefinition>,
    pub service: rmcp::service::RunningService<rmcp::RoleClient, ()>,
}

/// Manages MCP server connections and tool discovery.
pub struct McpManager {
    pub servers: HashMap<String, McpServer>,
}

impl McpManager {
    /// Create a manager from config, merging built-in presets.
    ///
    /// Presets not present in user config are added as disabled.
    /// Presets present in user config use the user's settings (including `disabled`).
    /// Non-preset servers from user config are added as-is.
    pub fn from_config(config: &McpConfig) -> Self {
        let mut servers = HashMap::new();

        // Add built-in presets first
        for preset in crate::mcp::presets::builtin_presets() {
            let cfg = if let Some(user_cfg) = config.servers.get(preset.id) {
                if user_cfg.removed {
                    continue; // User removed this preset — skip entirely
                }
                // User has this preset in their config — use their settings
                user_cfg.clone()
            } else {
                // Not in user config — use preset default (disabled)
                preset.config.clone()
            };
            servers.insert(
                preset.id.to_string(),
                McpServer {
                    name: preset.id.to_string(),
                    config: cfg,
                    status: ServerStatus::Disconnected,
                    is_preset: true,
                    tools: Vec::new(),
                    service: None,
                },
            );
        }

        // Add user-defined (non-preset) servers
        let preset_ids: Vec<&str> = crate::mcp::presets::builtin_presets()
            .iter()
            .map(|p| p.id)
            .collect();
        for (name, cfg) in &config.servers {
            if !preset_ids.contains(&name.as_str()) {
                servers.insert(
                    name.clone(),
                    McpServer {
                        name: name.clone(),
                        config: cfg.clone(),
                        status: ServerStatus::Disconnected,
                        is_preset: false,
                        tools: Vec::new(),
                        service: None,
                    },
                );
            }
        }

        Self { servers }
    }

    /// Static helper: connect to a server and discover tools without holding &mut self.
    async fn do_connect(name: &str, config: &McpServerConfig) -> Result<McpConnectResult, String> {
        // Build the command.
        // On Windows, npm/npx are .cmd batch scripts that won't be found by
        // Command::new directly — run them through `cmd /C` so the shell
        // resolves the extension via PATHEXT.
        let mut cmd = if cfg!(windows) {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(&config.command);
            c
        } else {
            tokio::process::Command::new(&config.command)
        };
        for arg in &config.args {
            cmd.arg(arg);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        // Use the builder API so our stdio settings aren't overridden by
        // TokioChildProcess::new's defaults (which use Stdio::inherit for stderr).
        // Suppress stderr — MCP servers print banners that bleed through the TUI.
        use rmcp::transport::TokioChildProcess;
        let (transport, _stderr) = TokioChildProcess::builder(cmd)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn: {e}"))?;

        use rmcp::ServiceExt;
        let service = ().serve(transport).await.map_err(|e| format!("Failed to initialize: {e}"))?;

        let tools = match service.list_tools(None).await {
            Ok(result) => result
                .tools
                .iter()
                .map(|t| ToolDefinition {
                    name: format!("{}:{}", name, t.name),
                    description: t
                        .description
                        .as_ref()
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                    input_schema: serde_json::Value::Object(t.input_schema.as_ref().clone()),
                })
                .collect(),
            Err(e) => {
                let msg = format!("Failed to list tools: {e}");
                let _ = service.cancel().await;
                return Err(msg);
            }
        };

        Ok(McpConnectResult { tools, service })
    }

    /// Connect to a single server. Updates its status and discovers tools.
    pub async fn connect_server(&mut self, name: &str) -> Result<(), String> {
        let server = self.servers.get_mut(name).ok_or("Server not found")?;
        server.status = ServerStatus::Connecting;
        let config = server.config.clone();

        match Self::do_connect(name, &config).await {
            Ok(result) => {
                let server = self.servers.get_mut(name).ok_or("Server not found")?;
                server.tools = result.tools;
                server.service = Some(result.service);
                server.status = ServerStatus::Connected;
                Ok(())
            }
            Err(msg) => {
                if let Some(server) = self.servers.get_mut(name) {
                    server.status = ServerStatus::Error(msg.clone());
                }
                Err(msg)
            }
        }
    }

    /// Spawn a background task to connect a server. Results come back via the channel.
    pub fn connect_server_background(
        &mut self,
        name: &str,
        tx: tokio::sync::mpsc::UnboundedSender<(String, Result<McpConnectResult, String>)>,
    ) -> Result<(), String> {
        let server = self.servers.get_mut(name).ok_or("Server not found")?;
        server.status = ServerStatus::Connecting;
        let config = server.config.clone();
        let server_name = name.to_string();

        tokio::spawn(async move {
            let result = McpManager::do_connect(&server_name, &config).await;
            let _ = tx.send((server_name, result));
        });

        Ok(())
    }

    /// Connect all configured servers that are not disabled.
    /// Errors are stored per-server, not propagated.
    pub async fn connect_all(&mut self) {
        let names: Vec<String> = self
            .servers
            .iter()
            .filter(|(_, s)| !s.config.disabled)
            .map(|(n, _)| n.clone())
            .collect();
        for name in names {
            if let Err(e) = self.connect_server(&name).await {
                tracing::warn!("MCP server '{name}' failed to connect: {e}");
            }
        }
    }

    /// Disable a server (set disabled=true) and disconnect it.
    pub async fn disable_server(&mut self, name: &str) {
        if let Some(server) = self.servers.get_mut(name) {
            server.config.disabled = true;
            if let Some(service) = server.service.take() {
                let _ = service.cancel().await;
            }
            server.status = ServerStatus::Disconnected;
            server.tools.clear();
        }
    }

    /// Disconnect a single server.
    pub async fn disconnect_server(&mut self, name: &str) {
        if let Some(server) = self.servers.get_mut(name) {
            if let Some(service) = server.service.take() {
                let _ = service.cancel().await;
            }
            server.status = ServerStatus::Disconnected;
            server.tools.clear();
        }
    }

    /// Disconnect all servers and clean up child processes.
    pub async fn disconnect_all(&mut self) {
        for server in self.servers.values_mut() {
            if let Some(service) = server.service.take() {
                let _ = service.cancel().await;
            }
            server.status = ServerStatus::Disconnected;
            server.tools.clear();
        }
    }

    /// Call a tool on an MCP server. `name` must be "server:tool" format.
    pub async fn call_tool(&mut self, name: &str, input: &serde_json::Value) -> ToolResult {
        let (server_name, tool_name) = match parse_tool_name(name) {
            Some(parsed) => parsed,
            None => {
                return ToolResult {
                    tool_use_id: String::new(),
                    output: format!("Invalid MCP tool name: {name}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }
        };

        let server = match self.servers.get(server_name) {
            Some(s) => s,
            None => {
                return ToolResult {
                    tool_use_id: String::new(),
                    output: format!("MCP server not found: {server_name}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }
        };

        let service = match &server.service {
            Some(s) => s,
            None => {
                return ToolResult {
                    tool_use_id: String::new(),
                    output: format!("MCP server '{server_name}' is not connected"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }
        };

        let arguments = input.as_object().cloned();

        let call_fut = service.call_tool(rmcp::model::CallToolRequestParams {
            meta: None,
            name: tool_name.to_string().into(),
            arguments,
            task: None,
        });
        let result = match tokio::time::timeout(std::time::Duration::from_secs(30), call_fut).await
        {
            Ok(r) => r,
            Err(_) => {
                return ToolResult {
                    tool_use_id: String::new(),
                    output: format!("MCP tool call timed out after 30s: {name}"),
                    is_error: true,
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                };
            }
        };

        match result {
            Ok(tool_result) => {
                let output = tool_result
                    .content
                    .iter()
                    .map(|c| match &c.raw {
                        rmcp::model::RawContent::Text(t) => t.text.clone(),
                        _ => "[non-text content]".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ToolResult {
                    tool_use_id: String::new(),
                    output,
                    is_error: tool_result.is_error.unwrap_or(false),
                    tool_name: Some(name.to_string()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                }
            }
            Err(e) => ToolResult {
                tool_use_id: String::new(),
                output: format!("MCP tool call failed: {e}"),
                is_error: true,
                tool_name: Some(name.to_string()),
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            },
        }
    }

    /// Validate an MCP tool call and return a cloneable peer + parsed params.
    /// This does no async work — the caller can spawn the actual RPC on a background task.
    #[allow(clippy::result_large_err)]
    pub fn prepare_tool_call(
        &self,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<McpPreparedCall, ToolResult> {
        let (server_name, tool_name) = parse_tool_name(name).ok_or_else(|| ToolResult {
            tool_use_id: String::new(),
            output: format!("Invalid MCP tool name: {name}"),
            is_error: true,
            tool_name: Some(name.to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        })?;

        let server = self.servers.get(server_name).ok_or_else(|| ToolResult {
            tool_use_id: String::new(),
            output: format!("MCP server not found: {server_name}"),
            is_error: true,
            tool_name: Some(name.to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        })?;

        let service = server.service.as_ref().ok_or_else(|| ToolResult {
            tool_use_id: String::new(),
            output: format!("MCP server '{server_name}' is not connected"),
            is_error: true,
            tool_name: Some(name.to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        })?;

        Ok(McpPreparedCall {
            peer: service.peer().clone(),
            tool_name: tool_name.to_string(),
            arguments: input.as_object().cloned(),
            full_name: name.to_string(),
        })
    }

    /// Return all tool definitions from all connected servers.
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.servers
            .values()
            .filter(|s| matches!(s.status, ServerStatus::Connected))
            .flat_map(|s| s.tools.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_from_empty_config_has_presets() {
        let config = McpConfig::default();
        let manager = McpManager::from_config(&config);
        // Empty config still gets built-in presets
        let preset_count = crate::mcp::presets::builtin_presets().len();
        assert_eq!(manager.servers.len(), preset_count);
        // All presets are disabled by default
        for server in manager.servers.values() {
            assert!(server.is_preset);
            assert!(server.config.disabled);
        }
        assert!(manager.tool_definitions().is_empty());
    }

    #[test]
    fn manager_from_config_creates_disconnected_servers() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "test".to_string(),
            McpServerConfig {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let manager = McpManager::from_config(&config);
        let preset_count = crate::mcp::presets::builtin_presets().len();
        assert_eq!(manager.servers.len(), preset_count + 1);
        assert!(matches!(
            manager.servers["test"].status,
            ServerStatus::Disconnected
        ));
        assert!(!manager.servers["test"].is_preset);
    }

    #[test]
    fn presets_merged_with_user_config() {
        let mut servers = std::collections::HashMap::new();
        // User enables context7 preset
        servers.insert(
            "context7".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@upstash/context7-mcp@latest".to_string()],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let manager = McpManager::from_config(&config);
        assert!(manager.servers["context7"].is_preset);
        assert!(!manager.servers["context7"].config.disabled);
    }

    #[test]
    fn server_status_display() {
        assert_eq!(ServerStatus::Disconnected.label(), "disconnected");
        assert_eq!(ServerStatus::Connecting.label(), "connecting");
        assert_eq!(ServerStatus::Connected.label(), "connected");
        assert_eq!(ServerStatus::Error("bad".into()).label(), "error");
    }

    #[test]
    fn tool_definitions_empty_when_disconnected() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "test".to_string(),
            McpServerConfig {
                command: "echo".to_string(),
                args: vec![],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let manager = McpManager::from_config(&config);
        assert!(manager.tool_definitions().is_empty());
    }

    #[tokio::test]
    async fn connect_to_nonexistent_command_sets_error() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "bad".to_string(),
            McpServerConfig {
                command: "nonexistent-command-that-does-not-exist-12345".to_string(),
                args: vec![],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let mut manager = McpManager::from_config(&config);
        manager.connect_all().await;
        assert!(matches!(
            manager.servers["bad"].status,
            ServerStatus::Error(_)
        ));
    }

    #[test]
    fn call_tool_parses_namespaced_name() {
        assert_eq!(
            parse_tool_name("github:create_issue"),
            Some(("github", "create_issue"))
        );
        assert_eq!(parse_tool_name("db:query"), Some(("db", "query")));
        assert_eq!(parse_tool_name("read_file"), None);
        assert_eq!(parse_tool_name(""), None);
    }

    #[tokio::test]
    async fn call_tool_unknown_server_returns_error() {
        let config = McpConfig::default();
        let mut manager = McpManager::from_config(&config);
        let result = manager
            .call_tool("unknown:tool", &serde_json::json!({}))
            .await;
        assert!(result.is_error);
        assert!(result.output.contains("unknown"));
    }

    #[tokio::test]
    async fn disconnect_all_sets_disconnected() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "test".to_string(),
            McpServerConfig {
                command: "echo".to_string(),
                args: vec![],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let mut manager = McpManager::from_config(&config);
        // Even without connecting, disconnect_all should be safe
        manager.disconnect_all().await;
        assert!(matches!(
            manager.servers["test"].status,
            ServerStatus::Disconnected
        ));
    }

    #[tokio::test]
    async fn disconnect_unknown_server_is_safe() {
        let config = McpConfig::default();
        let mut manager = McpManager::from_config(&config);
        // Should not panic
        manager.disconnect_server("nonexistent").await;
    }

    #[tokio::test]
    async fn connect_all_skips_disabled() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "enabled".to_string(),
            McpServerConfig {
                command: "nonexistent-cmd-12345".to_string(),
                args: vec![],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        servers.insert(
            "disabled_one".to_string(),
            McpServerConfig {
                command: "nonexistent-cmd-12345".to_string(),
                args: vec![],
                env: std::collections::HashMap::new(),
                disabled: true,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let mut manager = McpManager::from_config(&config);
        manager.connect_all().await;
        // Enabled server tried to connect (and failed since command doesn't exist)
        assert!(matches!(
            manager.servers["enabled"].status,
            ServerStatus::Error(_)
        ));
        // Disabled server was skipped — still Disconnected
        assert!(matches!(
            manager.servers["disabled_one"].status,
            ServerStatus::Disconnected
        ));
    }

    #[tokio::test]
    async fn disable_server_sets_disabled_and_disconnects() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "test".to_string(),
            McpServerConfig {
                command: "echo".to_string(),
                args: vec![],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: false,
            },
        );
        let config = McpConfig { servers };
        let mut manager = McpManager::from_config(&config);
        manager.disable_server("test").await;
        assert!(manager.servers["test"].config.disabled);
        assert!(matches!(
            manager.servers["test"].status,
            ServerStatus::Disconnected
        ));
    }

    #[test]
    fn from_config_skips_removed_presets() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "context7".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@upstash/context7-mcp@latest".to_string()],
                env: std::collections::HashMap::new(),
                disabled: false,
                removed: true,
            },
        );
        let config = McpConfig { servers };
        let manager = McpManager::from_config(&config);
        assert!(!manager.servers.contains_key("context7"));
    }
}
