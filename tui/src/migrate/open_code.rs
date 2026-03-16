use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::migrate::agent_import::{
    ImportedAgent, map_tool_name, normalize_agent_name, tool_allow_list_from_names,
    tool_deny_list_from_names,
};

/// Items discoverable from an Open Code installation
#[derive(Debug, Clone, Default)]
pub struct OpenCodeConfig {
    pub config_path: Option<PathBuf>,
    pub mcp_servers: Vec<(String, Value)>,
    pub system_prompt: Option<String>,
    pub agents: Vec<ImportedAgent>,
}

/// Scan Open Code config directories
pub fn scan_open_code(config_dirs: &[PathBuf], project_dir: Option<&Path>) -> OpenCodeConfig {
    let mut result = OpenCodeConfig::default();
    let mut agent_map: HashMap<String, ImportedAgent> = HashMap::new();

    for dir in config_dirs {
        if !dir.exists() {
            continue;
        }

        for filename in ["opencode.json", "config.json"] {
            let config_file = dir.join(filename);
            if config_file.exists() {
                result.config_path = Some(config_file.clone());
                if let Ok(text) = std::fs::read_to_string(&config_file)
                    && let Ok(parsed) = serde_json::from_str::<Value>(&text)
                {
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
                        && let Some(prompt) = parsed.get("systemPrompt").and_then(|v| v.as_str())
                    {
                        result.system_prompt = Some(prompt.to_string());
                    }

                    for agent in parse_json_agents(&parsed, &config_file) {
                        agent_map.insert(agent.name.clone(), agent);
                    }
                }
                break;
            }
        }

        // Fall back to instructions.md if no inline prompt found
        if result.system_prompt.is_none() {
            let instructions_file = dir.join("instructions.md");
            if instructions_file.exists()
                && let Ok(text) = std::fs::read_to_string(&instructions_file)
            {
                result.system_prompt = Some(text);
            }
        }

        let agents_dir = dir.join("agents");
        for agent in scan_agent_dir(&agents_dir) {
            agent_map.insert(agent.name.clone(), agent);
        }
    }

    if let Some(project_dir) = project_dir {
        for dirname in [".opencode", ".open-code"] {
            let base = project_dir.join(dirname);
            for agent in scan_agent_dir(&base.join("agents")) {
                agent_map.insert(agent.name.clone(), agent);
            }
            for filename in ["opencode.json", "config.json"] {
                let config_file = base.join(filename);
                if config_file.exists()
                    && let Ok(text) = std::fs::read_to_string(&config_file)
                    && let Ok(parsed) = serde_json::from_str::<Value>(&text)
                {
                    for agent in parse_json_agents(&parsed, &config_file) {
                        agent_map.insert(agent.name.clone(), agent);
                    }
                }
            }
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
        if let Some(agent) = parse_markdown_agent_file(&content, &path) {
            agents.push(agent);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

fn parse_markdown_agent_file(content: &str, path: &Path) -> Option<ImportedAgent> {
    let content = content.trim_start_matches('\u{feff}');
    let (frontmatter, body) = if let Some(after_first) = content.strip_prefix("---") {
        let end_idx = after_first.find("\n---")?;
        let yaml_str = &after_first[..end_idx];
        let body_start = end_idx + 4;
        let body = after_first[body_start..].trim_start_matches('\n');
        (
            serde_yml::from_str::<serde_yml::Value>(yaml_str).ok(),
            body.trim().to_string(),
        )
    } else {
        (None, content.trim().to_string())
    };
    if body.is_empty() {
        return None;
    }

    let stem = path.file_stem()?.to_string_lossy().to_string();
    let raw_name = frontmatter
        .as_ref()
        .and_then(|fm| fm.get("name").and_then(|v| v.as_str()))
        .unwrap_or(&stem);
    let normalized_name = normalize_agent_name(raw_name);
    let mut warnings = Vec::new();
    if normalized_name != raw_name {
        warnings.push(format!(
            "Renamed imported agent '{}' to '{}'",
            raw_name, normalized_name
        ));
    }

    let description = frontmatter
        .as_ref()
        .and_then(|fm| fm.get("description").and_then(|v| v.as_str()))
        .unwrap_or("Migrated OpenCode agent")
        .trim()
        .to_string();

    let model = frontmatter
        .as_ref()
        .and_then(|fm| fm.get("model").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mode = frontmatter
        .as_ref()
        .and_then(|fm| fm.get("mode").and_then(|v| v.as_str()));
    let worktree = match mode {
        Some("primary") => {
            warnings.push("Imported primary-mode agent as worktree:false Caboose agent".into());
            Some(false)
        }
        Some("subagent") => Some(true),
        _ => None,
    };

    let (tools, denied_tools, tool_warnings) = frontmatter
        .as_ref()
        .map(parse_open_code_yaml_tools)
        .unwrap_or((None, None, Vec::new()));
    warnings.extend(tool_warnings);
    warnings.extend(
        frontmatter
            .as_ref()
            .map(parse_open_code_yaml_permissions)
            .unwrap_or_default(),
    );
    let denied_tools = merge_tool_lists(
        denied_tools,
        frontmatter
            .as_ref()
            .map(collect_permission_denies)
            .unwrap_or_default(),
    );

    Some(ImportedAgent {
        name: normalized_name,
        description,
        model,
        tools,
        denied_tools,
        worktree,
        system_prompt: body,
        source_path: path.to_path_buf(),
        warnings,
    })
}

fn parse_json_agents(parsed: &Value, source_path: &Path) -> Vec<ImportedAgent> {
    let mut agents = Vec::new();
    for key in ["agent", "agents"] {
        let Some(map) = parsed.get(key).and_then(|v| v.as_object()) else {
            continue;
        };
        for (name, value) in map {
            let Some(obj) = value.as_object() else {
                continue;
            };
            let raw_name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(name.as_str());
            let normalized_name = normalize_agent_name(raw_name);
            let mut warnings = Vec::new();
            if normalized_name != raw_name {
                warnings.push(format!(
                    "Renamed imported agent '{}' to '{}'",
                    raw_name, normalized_name
                ));
            }
            let description = obj
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("Migrated OpenCode agent")
                .trim()
                .to_string();
            let model = obj
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let worktree = match obj.get("mode").and_then(|v| v.as_str()) {
                Some("primary") => {
                    warnings
                        .push("Imported primary-mode agent as worktree:false Caboose agent".into());
                    Some(false)
                }
                Some("subagent") => Some(true),
                _ => None,
            };

            let (tools, denied_tools, tool_warnings) = parse_open_code_json_tools(obj.get("tools"));
            warnings.extend(tool_warnings);
            let (permission_denies, permission_warnings) =
                parse_open_code_json_permissions(obj.get("permission"));
            warnings.extend(permission_warnings);

            let prompt = obj
                .get("prompt")
                .or_else(|| obj.get("systemPrompt"))
                .or_else(|| obj.get("instructions"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if prompt.is_empty() {
                continue;
            }

            agents.push(ImportedAgent {
                name: normalized_name,
                description,
                model,
                tools,
                denied_tools: merge_tool_lists(denied_tools, permission_denies),
                worktree,
                system_prompt: prompt,
                source_path: source_path.to_path_buf(),
                warnings,
            });
        }
    }
    agents
}

fn parse_open_code_yaml_tools(
    fm: &serde_yml::Value,
) -> (Option<Vec<String>>, Option<Vec<String>>, Vec<String>) {
    parse_open_code_tools_value(fm.get("tools"))
}

fn parse_open_code_json_tools(
    value: Option<&Value>,
) -> (Option<Vec<String>>, Option<Vec<String>>, Vec<String>) {
    parse_open_code_tools_value_json(value)
}

fn parse_open_code_tools_value(
    value: Option<&serde_yml::Value>,
) -> (Option<Vec<String>>, Option<Vec<String>>, Vec<String>) {
    match value {
        Some(serde_yml::Value::Sequence(items)) => {
            let names = items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let (tools, warnings) = tool_allow_list_from_names(&names);
            (tools, None, warnings)
        }
        Some(serde_yml::Value::Mapping(map)) => {
            let mut denied = Vec::new();
            let mut allow = Vec::new();
            for (key, value) in map {
                let Some(key) = key.as_str() else {
                    continue;
                };
                match value.as_bool() {
                    Some(true) => allow.push(key.to_string()),
                    Some(false) => denied.push(key.to_string()),
                    None => {}
                }
            }
            let (allow_tools, mut warnings) = tool_allow_list_from_names(&allow);
            let (deny_tools, deny_warnings) = tool_deny_list_from_names(&denied);
            warnings.extend(deny_warnings);
            // Favor deny-only semantics for bool maps; explicit true entries are advisory.
            (allow_tools.filter(|_| false), deny_tools, warnings)
        }
        _ => (None, None, Vec::new()),
    }
}

fn parse_open_code_tools_value_json(
    value: Option<&Value>,
) -> (Option<Vec<String>>, Option<Vec<String>>, Vec<String>) {
    match value {
        Some(Value::Array(items)) => {
            let names = items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let (tools, warnings) = tool_allow_list_from_names(&names);
            (tools, None, warnings)
        }
        Some(Value::Object(map)) => {
            let mut denied = Vec::new();
            let mut allow = Vec::new();
            for (key, value) in map {
                match value.as_bool() {
                    Some(true) => allow.push(key.to_string()),
                    Some(false) => denied.push(key.to_string()),
                    None => {}
                }
            }
            let (_, mut warnings) = tool_allow_list_from_names(&allow);
            let (deny_tools, deny_warnings) = tool_deny_list_from_names(&denied);
            warnings.extend(deny_warnings);
            (None, deny_tools, warnings)
        }
        _ => (None, None, Vec::new()),
    }
}

fn parse_open_code_yaml_permissions(fm: &serde_yml::Value) -> Vec<String> {
    let Some(map) = fm.get("permission").and_then(|v| v.as_mapping()) else {
        return Vec::new();
    };
    let mut warnings = Vec::new();
    for (key, value) in map {
        let Some(key) = key.as_str() else {
            continue;
        };
        if let Some(mode) = value.as_str()
            && mode != "allow"
        {
            warnings.push(format!(
                "Permission '{key}: {mode}' was imported as a tool restriction where possible"
            ));
        }
    }
    warnings
}

fn collect_permission_denies(fm: &serde_yml::Value) -> Option<Vec<String>> {
    let map = fm.get("permission").and_then(|v| v.as_mapping())?;
    let mut names = Vec::new();
    for (key, value) in map {
        let Some(key) = key.as_str() else {
            continue;
        };
        let Some(mode) = value.as_str() else {
            continue;
        };
        if mode == "deny" {
            names.extend(permission_key_to_tools(key));
        }
    }
    dedupe_tool_vec(names)
}

fn parse_open_code_json_permissions(value: Option<&Value>) -> (Option<Vec<String>>, Vec<String>) {
    let Some(map) = value.and_then(|v| v.as_object()) else {
        return (None, Vec::new());
    };
    let mut names = Vec::new();
    let mut warnings = Vec::new();
    for (key, value) in map {
        let Some(mode) = value.as_str() else {
            continue;
        };
        if mode != "allow" {
            warnings.push(format!(
                "Permission '{key}: {mode}' was imported as a tool restriction where possible"
            ));
        }
        if mode == "deny" {
            names.extend(permission_key_to_tools(key));
        }
    }
    (dedupe_tool_vec(names), warnings)
}

fn permission_key_to_tools(key: &str) -> Vec<String> {
    let names: Vec<String> = match key {
        "edit" => vec!["write".to_string()],
        "bash" => vec!["bash".to_string()],
        "webfetch" => vec!["webfetch".to_string()],
        _ => Vec::new(),
    };
    names
        .into_iter()
        .flat_map(|name| map_tool_name(&name))
        .collect()
}

fn merge_tool_lists(base: Option<Vec<String>>, extra: Option<Vec<String>>) -> Option<Vec<String>> {
    let mut set = BTreeSet::new();
    if let Some(values) = base {
        set.extend(values);
    }
    if let Some(values) = extra {
        set.extend(values);
    }
    if set.is_empty() {
        None
    } else {
        Some(set.into_iter().collect())
    }
}

fn dedupe_tool_vec(values: Vec<String>) -> Option<Vec<String>> {
    let set: BTreeSet<_> = values.into_iter().collect();
    if set.is_empty() {
        None
    } else {
        Some(set.into_iter().collect())
    }
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
    for agent in &config.agents {
        items.push(format!("Agent ({})", agent.name));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_scan_nonexistent_dirs() {
        let config = scan_open_code(&[PathBuf::from("/nonexistent/path")], None);
        assert!(config.mcp_servers.is_empty());
        assert!(config.system_prompt.is_none());
        assert!(config.config_path.is_none());
        assert!(config.agents.is_empty());
    }

    #[test]
    fn test_scan_empty_dir() {
        let dir = tempdir().unwrap();
        let config = scan_open_code(&[dir.path().to_path_buf()], None);
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

        let config = scan_open_code(&[dir.path().to_path_buf()], None);
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

        let config = scan_open_code(&[dir.path().to_path_buf()], None);
        assert_eq!(config.system_prompt.as_deref(), Some("Always use Rust"));
    }

    #[test]
    fn test_scan_instructions_md_fallback() {
        let dir = tempdir().unwrap();
        // No config.json — only instructions.md
        std::fs::write(dir.path().join("instructions.md"), "Use snake_case").unwrap();

        let config = scan_open_code(&[dir.path().to_path_buf()], None);
        assert_eq!(config.system_prompt.as_deref(), Some("Use snake_case"));
    }

    #[test]
    fn test_custom_instructions_takes_precedence_over_instructions_md() {
        let dir = tempdir().unwrap();
        let config_json = r#"{"customInstructions": "From config"}"#;
        std::fs::write(dir.path().join("config.json"), config_json).unwrap();
        std::fs::write(dir.path().join("instructions.md"), "From file").unwrap();

        let config = scan_open_code(&[dir.path().to_path_buf()], None);
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
            agents: vec![],
        };
        let items = importable_items(&config);
        assert_eq!(items.len(), 2);
        assert!(items[0].contains("2 MCP server(s)"));
        assert_eq!(items[1], "Custom instructions");
    }

    #[test]
    fn parse_open_code_markdown_agent_uses_filename_and_denies_false_tools() {
        let content = "---\ndescription: Reviews PRs\nmode: subagent\ntools:\n  write: false\n  bash: false\n---\nReview the diff.";
        let agent = parse_markdown_agent_file(content, Path::new("code-reviewer.md")).unwrap();
        assert_eq!(agent.name, "code-reviewer");
        assert_eq!(agent.worktree, Some(true));
        assert_eq!(
            agent.denied_tools,
            Some(vec![
                "apply_patch".into(),
                "edit_file".into(),
                "run_command".into(),
                "write_file".into()
            ])
        );
    }

    #[test]
    fn scan_open_code_reads_agents_from_opencode_json() {
        let dir = tempdir().unwrap();
        let config_json = r#"{
            "agent": {
                "reviewer": {
                    "description": "Review changes",
                    "mode": "subagent",
                    "model": "anthropic/claude-sonnet-4-6",
                    "prompt": "Review the code",
                    "tools": { "write": false }
                }
            }
        }"#;
        std::fs::write(dir.path().join("opencode.json"), config_json).unwrap();
        let config = scan_open_code(&[dir.path().to_path_buf()], None);
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].name, "reviewer");
    }
}
