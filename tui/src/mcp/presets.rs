//! Built-in MCP server presets — well-known servers users can toggle on.

use crate::config::schema::McpServerConfig;

/// A built-in MCP server preset.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct McpPreset {
    /// Unique identifier (used as the server name in config).
    pub id: &'static str,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Short description shown in the UI.
    pub description: &'static str,
    /// Default server configuration.
    pub config: McpServerConfig,
    /// Environment variables the server needs (for UI warnings).
    pub env_vars_needed: &'static [&'static str],
}

/// Return all built-in MCP server presets.
pub fn builtin_presets() -> Vec<McpPreset> {
    vec![
        McpPreset {
            id: "context7",
            display_name: "Context7",
            description: "Up-to-date library documentation",
            config: McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@upstash/context7-mcp@latest".to_string()],
                env: std::collections::HashMap::new(),
                disabled: true,
                removed: false,
            },
            env_vars_needed: &[],
        },
        McpPreset {
            id: "fetch",
            display_name: "Fetch",
            description: "HTTP fetch and web content",
            config: McpServerConfig {
                command: "npx".to_string(),
                args: vec![
                    "-y".to_string(),
                    "@modelcontextprotocol/server-fetch".to_string(),
                ],
                env: std::collections::HashMap::new(),
                disabled: true,
                removed: false,
            },
            env_vars_needed: &[],
        },
    ]
}

/// Look up a preset by id.
pub fn find_preset(id: &str) -> Option<McpPreset> {
    builtin_presets().into_iter().find(|p| p.id == id)
}

/// Return all preset IDs.
#[allow(dead_code)]
pub fn preset_ids() -> Vec<&'static str> {
    builtin_presets().iter().map(|p| p.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_non_empty() {
        assert!(!builtin_presets().is_empty());
    }

    #[test]
    fn all_presets_start_disabled() {
        for preset in builtin_presets() {
            assert!(
                preset.config.disabled,
                "preset {} should start disabled",
                preset.id
            );
        }
    }

    #[test]
    fn all_presets_have_valid_command() {
        for preset in builtin_presets() {
            assert!(!preset.config.command.is_empty());
            assert!(!preset.config.args.is_empty());
        }
    }

    #[test]
    fn find_preset_by_id() {
        assert!(find_preset("context7").is_some());
        assert!(find_preset("fetch").is_some());
        assert!(find_preset("nonexistent").is_none());
    }

    #[test]
    fn preset_ids_returns_all() {
        let ids = preset_ids();
        assert!(ids.contains(&"context7"));
        assert!(ids.contains(&"fetch"));
    }
}
