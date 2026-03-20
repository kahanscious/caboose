use caboose_core::config::schema::McpServerConfig;
use crate::migrate::agent_import::{render_caboose_agent_markdown, unique_agent_path};
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
            command: Some(command),
            url: None,
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
    pub agents_imported: Vec<String>,
    #[allow(dead_code)]
    pub warnings: Vec<String>,
}

/// Apply toggled migration items: write MCP servers to config and content to CABOOSE.md.
pub fn apply_migration(items: &[crate::tui::dialog::MigrationItem]) -> MigrationResult {
    use crate::tui::dialog::MigrationItemKind;

    let mut result = MigrationResult::default();

    for item in items {
        if !item.toggled {
            continue;
        }

        match &item.kind {
            MigrationItemKind::McpServer { name, config } => {
                if let Some((caboose_name, server_config)) = convert_mcp_server(name, config) {
                    caboose_core::config::save_mcp_server_toggle(&caboose_name, &server_config);
                    result.mcp_servers_added.push(caboose_name);
                }
            }
            MigrationItemKind::SystemPrompt(prompt) => {
                let converted = convert_system_prompt(prompt);
                let caboose_md = std::env::current_dir()
                    .unwrap_or_default()
                    .join("CABOOSE.md");
                let existing = std::fs::read_to_string(&caboose_md).unwrap_or_default();
                let new_content = if existing.is_empty() {
                    converted
                } else {
                    format!("{}\n\n{}", existing.trim_end(), converted)
                };
                let _ = std::fs::write(&caboose_md, new_content);
                result.system_prompt_migrated = true;
            }
            MigrationItemKind::ClaudeMd(path) => {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let caboose_md = std::env::current_dir()
                        .unwrap_or_default()
                        .join("CABOOSE.md");
                    let existing = std::fs::read_to_string(&caboose_md).unwrap_or_default();
                    let header = "\n\n## Imported from CLAUDE.md\n\n";
                    let new_content = if existing.is_empty() {
                        format!("## Imported from CLAUDE.md\n\n{}", content)
                    } else {
                        format!("{}{}{}", existing.trim_end(), header, content)
                    };
                    let _ = std::fs::write(&caboose_md, new_content);
                    result.claude_md_converted.push(path.display().to_string());
                }
            }
            MigrationItemKind::Agent(agent) => {
                let agents_dir = std::env::current_dir()
                    .unwrap_or_default()
                    .join(".caboose")
                    .join("agents");
                let _ = std::fs::create_dir_all(&agents_dir);
                let content = render_caboose_agent_markdown(agent);
                let (target_path, renamed) = unique_agent_path(&agents_dir, &agent.name, &content);
                if renamed {
                    result.warnings.push(format!(
                        "Agent '{}' already existed; imported copy as {}",
                        agent.name,
                        target_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&agent.name)
                    ));
                }
                if let Err(e) = std::fs::write(&target_path, content) {
                    result.warnings.push(format!(
                        "Failed to import agent '{}' from {}: {e}",
                        agent.name,
                        agent.source_path.display()
                    ));
                    continue;
                }
                result.agents_imported.push(
                    target_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&agent.name)
                        .to_string(),
                );
                result.warnings.extend(agent.warnings.clone());
            }
        }
    }

    result
}

impl MigrationResult {
    /// Format a human-readable summary of what was applied.
    pub fn format_summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.mcp_servers_added.is_empty() {
            parts.push(format!(
                "{} MCP server(s) added",
                self.mcp_servers_added.len()
            ));
        }
        if self.system_prompt_migrated {
            parts.push("system prompt written to CABOOSE.md".to_string());
        }
        if !self.claude_md_converted.is_empty() {
            parts.push("CLAUDE.md imported to CABOOSE.md".to_string());
        }
        if !self.agents_imported.is_empty() {
            parts.push(format!("{} agent(s) imported", self.agents_imported.len()));
        }
        if parts.is_empty() {
            "No changes applied.".to_string()
        } else {
            let warning_suffix = if self.warnings.is_empty() {
                String::new()
            } else {
                format!(" ({} warning(s))", self.warnings.len())
            };
            format!("Migration complete: {}{}", parts.join(", "), warning_suffix)
        }
    }
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
        assert_eq!(server.command.as_deref(), Some("npx"));
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

    #[test]
    fn test_migration_result_summary_empty() {
        let result = MigrationResult::default();
        assert_eq!(result.format_summary(), "No changes applied.");
    }

    #[test]
    fn test_migration_result_summary_full() {
        let result = MigrationResult {
            mcp_servers_added: vec!["server1".into(), "server2".into()],
            system_prompt_migrated: true,
            claude_md_converted: vec!["path".into()],
            agents_imported: vec![],
            warnings: vec![],
        };
        let summary = result.format_summary();
        assert!(summary.contains("2 MCP server(s)"));
        assert!(summary.contains("system prompt"));
        assert!(summary.contains("CLAUDE.md"));
    }

    #[test]
    fn test_migration_result_summary_mcp_only() {
        let result = MigrationResult {
            mcp_servers_added: vec!["s1".into()],
            system_prompt_migrated: false,
            claude_md_converted: vec![],
            agents_imported: vec![],
            warnings: vec![],
        };
        let summary = result.format_summary();
        assert!(summary.contains("1 MCP server(s)"));
        assert!(!summary.contains("system prompt"));
    }
}
