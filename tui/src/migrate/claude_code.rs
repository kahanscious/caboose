use std::path::{Path, PathBuf};
use serde_json::Value;

/// Items discoverable from a Claude Code installation
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    pub settings_path: Option<PathBuf>,
    pub claude_md_paths: Vec<PathBuf>,
    pub mcp_servers: Vec<(String, Value)>,
    pub system_prompt: Option<String>,
    #[allow(dead_code)]
    pub permission_mode: Option<String>,
}

/// Scan Claude Code config directories for importable items
pub fn scan_claude_code(config_dirs: &[PathBuf], project_dir: Option<&Path>) -> ClaudeCodeConfig {
    let mut result = ClaudeCodeConfig {
        settings_path: None,
        claude_md_paths: Vec::new(),
        mcp_servers: Vec::new(),
        system_prompt: None,
        permission_mode: None,
    };

    for dir in config_dirs {
        let settings_file = dir.join("settings.json");
        if settings_file.exists() {
            result.settings_path = Some(settings_file.clone());
            if let Ok(contents) = std::fs::read_to_string(&settings_file)
                && let Ok(json) = serde_json::from_str::<Value>(&contents) {
                    if let Some(servers) = json.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, config) in servers {
                            result.mcp_servers.push((name.clone(), config.clone()));
                        }
                    }
                    if let Some(prompt) = json.get("systemPrompt").and_then(|v| v.as_str()) {
                        result.system_prompt = Some(prompt.to_string());
                    }
                }
        }
    }

    if let Some(proj) = project_dir {
        let claude_md = proj.join("CLAUDE.md");
        if claude_md.exists() {
            result.claude_md_paths.push(claude_md);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global_claude_md = home.join(".claude").join("CLAUDE.md");
        if global_claude_md.exists() && !result.claude_md_paths.contains(&global_claude_md) {
            result.claude_md_paths.push(global_claude_md);
        }
    }

    result
}

/// Summary of what's available for import
#[allow(dead_code)]
pub fn importable_items(config: &ClaudeCodeConfig) -> Vec<String> {
    let mut items = Vec::new();
    if !config.mcp_servers.is_empty() {
        items.push(format!("{} MCP server(s)", config.mcp_servers.len()));
    }
    if config.system_prompt.is_some() {
        items.push("System prompt".into());
    }
    for path in &config.claude_md_paths {
        items.push(format!("CLAUDE.md ({})", path.display()));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_nonexistent_dirs() {
        let config = scan_claude_code(&[PathBuf::from("/nonexistent")], None);
        assert!(config.settings_path.is_none());
        assert!(config.mcp_servers.is_empty());
        assert!(config.system_prompt.is_none());
    }

    #[test]
    fn test_importable_items_empty() {
        let config = ClaudeCodeConfig {
            settings_path: None,
            claude_md_paths: Vec::new(),
            mcp_servers: Vec::new(),
            system_prompt: None,
            permission_mode: None,
        };
        assert!(importable_items(&config).is_empty());
    }

    #[test]
    fn test_importable_items_with_data() {
        let config = ClaudeCodeConfig {
            settings_path: None,
            claude_md_paths: vec![PathBuf::from("/proj/CLAUDE.md")],
            mcp_servers: vec![("server1".into(), serde_json::json!({}))],
            system_prompt: Some("test prompt".into()),
            permission_mode: None,
        };
        let items = importable_items(&config);
        assert_eq!(items.len(), 3);
    }
}
