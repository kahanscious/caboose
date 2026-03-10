#![allow(dead_code)]
use std::path::PathBuf;

/// Items discoverable from a Codex installation
#[derive(Debug, Clone, Default)]
pub struct CodexConfig {
    pub config_path: Option<PathBuf>,
    pub instructions: Option<String>,
    pub instructions_md: Option<String>,
}

/// Scan Codex config directories
pub fn scan_codex(config_dirs: &[PathBuf]) -> CodexConfig {
    let mut result = CodexConfig::default();

    for dir in config_dirs {
        if !dir.exists() {
            continue;
        }

        // Check config.json, config.yaml, config.yml in order
        for filename in &["config.json", "config.yaml", "config.yml"] {
            let config_file = dir.join(filename);
            if config_file.exists() {
                result.config_path = Some(config_file.clone());
                if let Ok(text) = std::fs::read_to_string(&config_file) {
                    // Try JSON first
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(instructions) =
                            parsed.get("instructions").and_then(|v| v.as_str())
                        {
                            result.instructions = Some(instructions.to_string());
                        }
                    }
                }
                break;
            }
        }

        // Check instructions.md
        let instructions_file = dir.join("instructions.md");
        if instructions_file.exists() {
            if let Ok(text) = std::fs::read_to_string(&instructions_file) {
                result.instructions_md = Some(text);
            }
        }

        // Check AGENTS.md as a fallback for instructions_md
        if result.instructions_md.is_none() {
            let agents_file = dir.join("AGENTS.md");
            if agents_file.exists() {
                if let Ok(text) = std::fs::read_to_string(&agents_file) {
                    result.instructions_md = Some(text);
                }
            }
        }
    }

    result
}

/// Summary of what's available for import
pub fn importable_items(config: &CodexConfig) -> Vec<String> {
    let mut items = Vec::new();
    if config.instructions.is_some() {
        items.push("Configuration instructions".to_string());
    }
    if config.instructions_md.is_some() {
        items.push("Instructions file".to_string());
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_scan_nonexistent_dirs() {
        let config = scan_codex(&[PathBuf::from("/nonexistent/path")]);
        assert!(config.instructions.is_none());
        assert!(config.instructions_md.is_none());
        assert!(config.config_path.is_none());
    }

    #[test]
    fn test_scan_empty_dir() {
        let dir = tempdir().unwrap();
        let config = scan_codex(&[dir.path().to_path_buf()]);
        assert!(config.instructions.is_none());
        assert!(config.instructions_md.is_none());
    }

    #[test]
    fn test_scan_config_json_with_instructions() {
        let dir = tempdir().unwrap();
        let config_json = r#"{"instructions": "Always write tests"}"#;
        std::fs::write(dir.path().join("config.json"), config_json).unwrap();

        let config = scan_codex(&[dir.path().to_path_buf()]);
        assert_eq!(config.instructions.as_deref(), Some("Always write tests"));
        assert!(config.config_path.is_some());
    }

    #[test]
    fn test_scan_instructions_md() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("instructions.md"), "Use idiomatic Rust").unwrap();

        let config = scan_codex(&[dir.path().to_path_buf()]);
        assert_eq!(
            config.instructions_md.as_deref(),
            Some("Use idiomatic Rust")
        );
    }

    #[test]
    fn test_scan_agents_md_fallback() {
        let dir = tempdir().unwrap();
        // No instructions.md — only AGENTS.md
        std::fs::write(dir.path().join("AGENTS.md"), "Agent guidelines here").unwrap();

        let config = scan_codex(&[dir.path().to_path_buf()]);
        assert_eq!(
            config.instructions_md.as_deref(),
            Some("Agent guidelines here")
        );
    }

    #[test]
    fn test_instructions_md_takes_precedence_over_agents_md() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("instructions.md"), "From instructions").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "From agents").unwrap();

        let config = scan_codex(&[dir.path().to_path_buf()]);
        assert_eq!(
            config.instructions_md.as_deref(),
            Some("From instructions")
        );
    }

    #[test]
    fn test_scan_config_json_and_instructions_md_together() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"instructions": "Be brief"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("instructions.md"), "Extended notes").unwrap();

        let config = scan_codex(&[dir.path().to_path_buf()]);
        assert_eq!(config.instructions.as_deref(), Some("Be brief"));
        assert_eq!(config.instructions_md.as_deref(), Some("Extended notes"));
    }

    #[test]
    fn test_importable_items_empty() {
        let config = CodexConfig::default();
        assert!(importable_items(&config).is_empty());
    }

    #[test]
    fn test_importable_items_with_data() {
        let config = CodexConfig {
            config_path: None,
            instructions: Some("cfg instructions".into()),
            instructions_md: Some("md content".into()),
        };
        let items = importable_items(&config);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], "Configuration instructions");
        assert_eq!(items[1], "Instructions file");
    }

    #[test]
    fn test_importable_items_instructions_only() {
        let config = CodexConfig {
            config_path: None,
            instructions: Some("only cfg".into()),
            instructions_md: None,
        };
        let items = importable_items(&config);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0], "Configuration instructions");
    }
}
