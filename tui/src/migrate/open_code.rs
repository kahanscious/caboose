use serde_json::Value;
use std::path::PathBuf;

/// Items discoverable from an Open Code installation
#[derive(Debug, Clone, Default)]
pub struct OpenCodeConfig {
    pub config_path: Option<PathBuf>,
    pub mcp_servers: Vec<(String, Value)>,
    pub system_prompt: Option<String>,
}

/// Scan Open Code config directories
pub fn scan_open_code(config_dirs: &[PathBuf]) -> OpenCodeConfig {
    let mut result = OpenCodeConfig::default();

    for dir in config_dirs {
        if !dir.exists() {
            continue;
        }

        let config_file = dir.join("config.json");
        if config_file.exists() {
            result.config_path = Some(config_file.clone());
            if let Ok(text) = std::fs::read_to_string(&config_file)
                && let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    // Extract MCP servers
                    if let Some(servers) = parsed.get("mcpServers").and_then(|v| v.as_object()) {
                        for (name, cfg) in servers {
                            result.mcp_servers.push((name.clone(), cfg.clone()));
                        }
                    }
                    // Extract custom instructions / system prompt
                    if let Some(instructions) =
                        parsed.get("customInstructions").and_then(|v| v.as_str())
                    {
                        result.system_prompt = Some(instructions.to_string());
                    }
                    if result.system_prompt.is_none()
                        && let Some(prompt) =
                            parsed.get("systemPrompt").and_then(|v| v.as_str())
                        {
                            result.system_prompt = Some(prompt.to_string());
                        }
                }
        }

        // Fall back to instructions.md if no inline prompt found
        if result.system_prompt.is_none() {
            let instructions_file = dir.join("instructions.md");
            if instructions_file.exists()
                && let Ok(text) = std::fs::read_to_string(&instructions_file) {
                    result.system_prompt = Some(text);
                }
        }
    }

    result
}

/// Summary of what's available for import
#[allow(dead_code)]
pub fn importable_items(config: &OpenCodeConfig) -> Vec<String> {
    let mut items = Vec::new();
    if !config.mcp_servers.is_empty() {
        items.push(format!("{} MCP server(s)", config.mcp_servers.len()));
    }
    if config.system_prompt.is_some() {
        items.push("Custom instructions".to_string());
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_scan_nonexistent_dirs() {
        let config = scan_open_code(&[PathBuf::from("/nonexistent/path")]);
        assert!(config.mcp_servers.is_empty());
        assert!(config.system_prompt.is_none());
        assert!(config.config_path.is_none());
    }

    #[test]
    fn test_scan_empty_dir() {
        let dir = tempdir().unwrap();
        let config = scan_open_code(&[dir.path().to_path_buf()]);
        assert!(config.mcp_servers.is_empty());
        assert!(config.system_prompt.is_none());
    }

    #[test]
    fn test_scan_with_mcp_servers_and_custom_instructions() {
        let dir = tempdir().unwrap();
        let config_json = r#"{
            "mcpServers": {
                "test-server": {"command": "npx", "args": ["-y", "@test/mcp"]},
                "another": {"command": "node", "args": ["server.js"]}
            },
            "customInstructions": "Be concise and helpful"
        }"#;
        std::fs::write(dir.path().join("config.json"), config_json).unwrap();

        let config = scan_open_code(&[dir.path().to_path_buf()]);
        assert_eq!(config.mcp_servers.len(), 2);
        assert_eq!(
            config.system_prompt.as_deref(),
            Some("Be concise and helpful")
        );
        assert!(config.config_path.is_some());
    }

    #[test]
    fn test_scan_with_system_prompt_key() {
        let dir = tempdir().unwrap();
        let config_json = r#"{"systemPrompt": "Always use Rust"}"#;
        std::fs::write(dir.path().join("config.json"), config_json).unwrap();

        let config = scan_open_code(&[dir.path().to_path_buf()]);
        assert_eq!(config.system_prompt.as_deref(), Some("Always use Rust"));
    }

    #[test]
    fn test_scan_instructions_md_fallback() {
        let dir = tempdir().unwrap();
        // No config.json — only instructions.md
        std::fs::write(dir.path().join("instructions.md"), "Use snake_case").unwrap();

        let config = scan_open_code(&[dir.path().to_path_buf()]);
        assert_eq!(config.system_prompt.as_deref(), Some("Use snake_case"));
    }

    #[test]
    fn test_custom_instructions_takes_precedence_over_instructions_md() {
        let dir = tempdir().unwrap();
        let config_json = r#"{"customInstructions": "From config"}"#;
        std::fs::write(dir.path().join("config.json"), config_json).unwrap();
        std::fs::write(dir.path().join("instructions.md"), "From file").unwrap();

        let config = scan_open_code(&[dir.path().to_path_buf()]);
        assert_eq!(config.system_prompt.as_deref(), Some("From config"));
    }

    #[test]
    fn test_importable_items_empty() {
        let config = OpenCodeConfig::default();
        assert!(importable_items(&config).is_empty());
    }

    #[test]
    fn test_importable_items_with_data() {
        let config = OpenCodeConfig {
            config_path: None,
            mcp_servers: vec![
                ("s1".into(), serde_json::json!({})),
                ("s2".into(), serde_json::json!({})),
            ],
            system_prompt: Some("prompt".into()),
        };
        let items = importable_items(&config);
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("2 MCP server(s)"));
        assert_eq!(items[1], "Custom instructions");
    }
}
