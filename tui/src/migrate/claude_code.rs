use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::migrate::agent_import::{
    ImportedAgent, normalize_agent_name, tool_allow_list_from_names,
};

/// Items discoverable from a Claude Code installation
#[derive(Debug, Clone)]
pub struct ClaudeCodeConfig {
    pub settings_path: Option<PathBuf>,
    pub claude_md_paths: Vec<PathBuf>,
    pub mcp_servers: Vec<(String, Value)>,
    pub system_prompt: Option<String>,
    pub agents: Vec<ImportedAgent>,
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
        agents: Vec::new(),
        permission_mode: None,
    };

    let mut agent_map: HashMap<String, ImportedAgent> = HashMap::new();

    for dir in config_dirs {
        let settings_file = dir.join("settings.json");
        if settings_file.exists() {
            result.settings_path = Some(settings_file.clone());
            if let Ok(contents) = std::fs::read_to_string(&settings_file)
                && let Ok(json) = serde_json::from_str::<Value>(&contents)
            {
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

        let agents_dir = dir.join("agents");
        for agent in scan_agent_dir(&agents_dir) {
            agent_map.insert(agent.name.clone(), agent);
        }
    }

    if let Some(proj) = project_dir {
        let claude_md = proj.join("CLAUDE.md");
        if claude_md.exists() {
            result.claude_md_paths.push(claude_md);
        }

        let project_agents_dir = proj.join(".claude").join("agents");
        for agent in scan_agent_dir(&project_agents_dir) {
            agent_map.insert(agent.name.clone(), agent);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let global_claude_md = home.join(".claude").join("CLAUDE.md");
        if global_claude_md.exists() && !result.claude_md_paths.contains(&global_claude_md) {
            result.claude_md_paths.push(global_claude_md);
        }
    }

    let mut agents: Vec<_> = agent_map.into_values().collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    result.agents = agents;

    result
}

fn scan_agent_dir(dir: &Path) -> Vec<ImportedAgent> {
    let mut agents = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return agents,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(agent) = parse_claude_agent_file(&content, &path) {
            agents.push(agent);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

fn parse_claude_agent_file(content: &str, path: &Path) -> Option<ImportedAgent> {
    let content = content.trim_start_matches('\u{feff}');
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let end_idx = after_first.find("\n---")?;
    let yaml_str = &after_first[..end_idx];
    let body_start = end_idx + 4;
    let body = after_first[body_start..]
        .trim_start_matches('\n')
        .trim()
        .to_string();
    if body.is_empty() {
        return None;
    }

    let fm: serde_yml::Value = serde_yml::from_str(yaml_str).ok()?;
    let name_raw = fm.get("name")?.as_str()?.trim();
    let description = fm
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("Migrated Claude Code agent")
        .trim()
        .to_string();
    let model = fm
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let tool_names = fm.get("tools").map(parse_string_list).unwrap_or_default();
    let (tools, mut warnings) = tool_allow_list_from_names(&tool_names);

    let normalized_name = normalize_agent_name(name_raw);
    if normalized_name != name_raw {
        warnings.push(format!(
            "Renamed imported agent '{}' to '{}'",
            name_raw, normalized_name
        ));
    }

    Some(ImportedAgent {
        name: normalized_name,
        description,
        model,
        tools,
        denied_tools: None,
        worktree: None,
        system_prompt: body,
        source_path: path.to_path_buf(),
        warnings,
    })
}

fn parse_string_list(value: &serde_yml::Value) -> Vec<String> {
    match value {
        serde_yml::Value::Sequence(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        serde_yml::Value::String(s) => s
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
        _ => Vec::new(),
    }
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
        assert!(config.agents.is_empty());
    }

    #[test]
    fn parse_claude_agent_from_markdown() {
        let content = "---\nname: Reviewer\ndescription: Reviews code\ntools: [Read, Grep, Bash]\n---\nYou review code.";
        let agent = parse_claude_agent_file(content, Path::new("reviewer.md")).unwrap();
        assert_eq!(agent.name, "reviewer");
        assert_eq!(agent.description, "Reviews code");
        assert_eq!(
            agent.tools,
            Some(vec![
                "grep".into(),
                "list_directory".into(),
                "read_file".into(),
                "run_command".into()
            ])
        );
    }
}
