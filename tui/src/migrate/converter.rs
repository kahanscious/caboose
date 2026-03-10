use crate::config::schema::McpServerConfig;
use serde_json::Value;
use std::collections::HashMap;

/// Convert a Claude Code MCP server config to Caboose format
pub fn convert_mcp_server(name: &str, claude_config: &Value) -> Option<(String, McpServerConfig)> {
    let command = claude_config.get("command")?.as_str()?.to_string();
    let args: Vec<String> = claude_config
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let env: HashMap<String, String> = claude_config
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    Some((
        name.to_string(),
        McpServerConfig {
            command,
            args,
            env,
            disabled: false,
            removed: false,
        },
    ))
}

/// Convert Claude Code system prompt to CABOOSE.md content
pub fn convert_system_prompt(prompt: &str) -> String {
    format!(
        "# Project Instructions\n\n\
         <!-- Migrated from Claude Code -->\n\n\
         {prompt}\n"
    )
}

/// Summary of a migration result
#[derive(Debug, Default)]
pub struct MigrationResult {
    pub mcp_servers_added: Vec<String>,
    pub system_prompt_migrated: bool,
    pub claude_md_converted: Vec<String>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_convert_mcp_server() {
        let config = json!({
            "command": "npx",
            "args": ["-y", "@some/mcp-server"],
            "env": { "API_KEY": "test" }
        });
        let (name, server) = convert_mcp_server("myserver", &config).unwrap();
        assert_eq!(name, "myserver");
        assert_eq!(server.command, "npx");
        assert_eq!(server.args.len(), 2);
    }

    #[test]
    fn test_convert_mcp_server_missing_command() {
        let config = json!({ "args": ["foo"] });
        assert!(convert_mcp_server("bad", &config).is_none());
    }

    #[test]
    fn test_convert_system_prompt() {
        let result = convert_system_prompt("always use TypeScript");
        assert!(result.contains("always use TypeScript"));
        assert!(result.contains("Migrated from Claude Code"));
    }
}
