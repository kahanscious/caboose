use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    Project,
    Global,
}

#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub name: String,
    pub description: String,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub worktree: Option<bool>,
    pub source: AgentSource,
    pub file_path: PathBuf,
    pub system_prompt: String,
}

/// Raw frontmatter for serde_yml deserialization.
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    name: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tools: Option<Vec<String>>,
    denied_tools: Option<Vec<String>>,
    worktree: Option<bool>,
}

/// Parse a markdown agent file into an AgentDefinition.
/// Returns None if the file is invalid (missing required fields, bad YAML, etc.).
pub fn parse_agent_file(
    content: &str,
    file_path: &std::path::Path,
    source: AgentSource,
) -> Option<AgentDefinition> {
    let content = content.trim_start_matches('\u{feff}'); // strip BOM
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let end_idx = after_first.find("\n---")?;
    let yaml_str = &after_first[..end_idx];
    let body_start = end_idx + 4; // skip \n---
    let body = after_first[body_start..].trim_start_matches('\n');

    let fm: AgentFrontmatter = serde_yml::from_str(yaml_str).ok()?;

    let name = fm.name?;
    let description = fm.description?;

    if !is_valid_name(&name) {
        return None;
    }

    // tools and denied_tools are mutually exclusive
    if fm.tools.is_some() && fm.denied_tools.is_some() {
        return None;
    }

    Some(AgentDefinition {
        name,
        description,
        model: fm.model,
        tools: fm.tools,
        denied_tools: fm.denied_tools,
        worktree: fm.worktree,
        source,
        file_path: file_path.to_path_buf(),
        system_prompt: body.to_string(),
    })
}

/// Validate an agent name: must match ^[a-z][a-z0-9-]{0,39}$
fn is_valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 40 {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Scan a directory for agent `.md` files and parse them.
pub fn scan_directory(dir: &std::path::Path, source: AgentSource) -> Vec<AgentDefinition> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut agents = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path)
            && let Some(def) = parse_agent_file(&content, &path, source.clone())
        {
            agents.push(def);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

/// Load all agents from project and global directories.
/// Project agents override global agents with the same name.
pub fn load_agents(
    project_dir: Option<&std::path::Path>,
    global_dir: Option<&std::path::Path>,
) -> Vec<AgentDefinition> {
    let mut map = std::collections::HashMap::<String, AgentDefinition>::new();

    // Global first (lower priority)
    if let Some(dir) = global_dir {
        for def in scan_directory(dir, AgentSource::Global) {
            map.insert(def.name.clone(), def);
        }
    }

    // Project second (overrides global)
    if let Some(dir) = project_dir {
        for def in scan_directory(dir, AgentSource::Project) {
            map.insert(def.name.clone(), def);
        }
    }

    let mut agents: Vec<AgentDefinition> = map.into_values().collect();
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

/// Load agents and filter out any whose names collide with built-in client commands.
pub fn load_agents_validated(
    project_dir: Option<&std::path::Path>,
    global_dir: Option<&std::path::Path>,
    client_commands: &[&str],
) -> Vec<AgentDefinition> {
    let mut agents = load_agents(project_dir, global_dir);
    agents.retain(|a| {
        let collides = client_commands
            .iter()
            .any(|cmd| cmd.to_lowercase() == a.name);
        if collides {
            eprintln!(
                "warning: agent '{}' skipped — collides with built-in command",
                a.name
            );
        }
        !collides
    });
    agents
}

/// Build the agent awareness block for system prompt injection.
pub fn build_agent_awareness_block(agents: &[AgentDefinition]) -> String {
    if agents.is_empty() {
        return String::new();
    }

    let mut block = String::from(
        "\n\n## Available agents\n\n\
         You can spawn specialized agents using the spawn_agent tool with the `agent` parameter.\n\
         Use these when the task matches an agent's specialty.\n\n",
    );

    for agent in agents {
        block.push_str(&format!("- {}: {}\n", agent.name, agent.description));
    }

    block
}

/// Resolve a model shorthand to a full model ID.
/// Returns None for unknown shorthands (caller should fall back to current model).
pub fn resolve_model_shorthand(model: &str) -> Option<&'static str> {
    match model {
        "sonnet" => Some("claude-sonnet-4-6"),
        "opus" => Some("claude-opus-4-6"),
        "haiku" => Some("claude-haiku-4-5-20251001"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_agent() {
        let content = "---\nname: code-reviewer\ndescription: Reviews code for bugs\nmodel: sonnet\ntools: [read, grep, glob]\nworktree: false\n---\nYou are a code reviewer.";
        let def = parse_agent_file(
            content,
            std::path::Path::new("test.md"),
            AgentSource::Project,
        )
        .unwrap();
        assert_eq!(def.name, "code-reviewer");
        assert_eq!(def.description, "Reviews code for bugs");
        assert_eq!(def.model, Some("sonnet".into()));
        assert_eq!(
            def.tools,
            Some(vec!["read".into(), "grep".into(), "glob".into()])
        );
        assert_eq!(def.worktree, Some(false));
        assert_eq!(def.system_prompt, "You are a code reviewer.");
        assert_eq!(def.source, AgentSource::Project);
    }

    #[test]
    fn parse_minimal_agent() {
        let content = "---\nname: helper\ndescription: General helper\n---\nDo helpful things.";
        let def = parse_agent_file(
            content,
            std::path::Path::new("helper.md"),
            AgentSource::Global,
        )
        .unwrap();
        assert_eq!(def.name, "helper");
        assert!(def.model.is_none());
        assert!(def.tools.is_none());
        assert!(def.denied_tools.is_none());
        assert!(def.worktree.is_none());
        assert_eq!(def.system_prompt, "Do helpful things.");
    }

    #[test]
    fn parse_missing_name_returns_none() {
        let content = "---\ndescription: No name\n---\nBody.";
        assert!(parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .is_none());
    }

    #[test]
    fn parse_missing_description_returns_none() {
        let content = "---\nname: foo\n---\nBody.";
        assert!(parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .is_none());
    }

    #[test]
    fn parse_both_tools_and_denied_tools_returns_none() {
        let content =
            "---\nname: foo\ndescription: bar\ntools: [read]\ndenied_tools: [write]\n---\nBody.";
        assert!(parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .is_none());
    }

    #[test]
    fn parse_invalid_name_returns_none() {
        let content = "---\nname: Code-Reviewer\ndescription: bad name\n---\nBody.";
        assert!(parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .is_none());
    }

    #[test]
    fn parse_no_frontmatter_returns_none() {
        let content = "Just a plain markdown file.";
        assert!(parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .is_none());
    }

    #[test]
    fn parse_bad_yaml_returns_none() {
        let content = "---\n: invalid yaml [[\n---\nBody.";
        assert!(parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .is_none());
    }

    #[test]
    fn valid_names() {
        assert!(is_valid_name("code-reviewer"));
        assert!(is_valid_name("a"));
        assert!(is_valid_name("test-agent-2"));
        assert!(is_valid_name("a1b2c3"));
    }

    #[test]
    fn invalid_names() {
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("Code-Reviewer"));
        assert!(!is_valid_name("2fast"));
        assert!(!is_valid_name("-leading"));
        assert!(!is_valid_name("has spaces"));
        assert!(!is_valid_name("has_underscores"));
        assert!(!is_valid_name(&"a".repeat(41)));
    }

    #[test]
    fn parse_denied_tools_field() {
        let content = "---\nname: safe-agent\ndescription: No shell\ndenied_tools: [shell, write]\n---\nBe safe.";
        let def = parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .unwrap();
        assert_eq!(
            def.denied_tools,
            Some(vec!["shell".into(), "write".into()])
        );
        assert!(def.tools.is_none());
    }

    #[test]
    fn parse_multiline_body() {
        let content =
            "---\nname: writer\ndescription: Writes code\n---\nLine 1.\n\nLine 2.\n\nLine 3.";
        let def = parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .unwrap();
        assert!(def.system_prompt.contains("Line 1."));
        assert!(def.system_prompt.contains("Line 3."));
    }

    #[test]
    fn parse_empty_body_allowed() {
        let content = "---\nname: empty\ndescription: No prompt\n---\n";
        let def = parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .unwrap();
        assert_eq!(def.name, "empty");
        assert!(def.system_prompt.is_empty() || def.system_prompt.trim().is_empty());
    }

    #[test]
    fn parse_worktree_false() {
        let content =
            "---\nname: reader\ndescription: Read only\nworktree: false\n---\nRead stuff.";
        let def = parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .unwrap();
        assert_eq!(def.worktree, Some(false));
    }

    #[test]
    fn parse_worktree_default_is_none() {
        let content = "---\nname: default\ndescription: Default worktree\n---\nBody.";
        let def = parse_agent_file(content, std::path::Path::new("a.md"), AgentSource::Project)
            .unwrap();
        assert!(def.worktree.is_none());
    }

    #[test]
    fn scan_directory_nonexistent_returns_empty() {
        let agents = scan_directory(
            std::path::Path::new("/nonexistent/path"),
            AgentSource::Global,
        );
        assert!(agents.is_empty());
    }

    #[test]
    fn load_agents_no_dirs_returns_empty() {
        let agents = load_agents(None, None);
        assert!(agents.is_empty());
    }

    #[test]
    fn scan_directory_with_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let agent_file = dir.path().join("reviewer.md");
        std::fs::write(
            &agent_file,
            "---\nname: reviewer\ndescription: Reviews code\n---\nReview it.",
        )
        .unwrap();
        let agents = scan_directory(dir.path(), AgentSource::Project);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "reviewer");
    }

    #[test]
    fn scan_directory_skips_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("good.md"),
            "---\nname: good\ndescription: Good agent\n---\nGood.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("bad.md"),
            "---\ndescription: Bad agent\n---\nBad.",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not an agent").unwrap();
        let agents = scan_directory(dir.path(), AgentSource::Project);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "good");
    }

    #[test]
    fn load_agents_project_overrides_global() {
        let global = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::write(
            global.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: Global reviewer\n---\nGlobal.",
        )
        .unwrap();
        std::fs::write(
            project.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: Project reviewer\n---\nProject.",
        )
        .unwrap();
        let agents = load_agents(Some(project.path()), Some(global.path()));
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].description, "Project reviewer");
        assert_eq!(agents[0].source, AgentSource::Project);
    }

    #[test]
    fn load_agents_validated_filters_command_collisions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("model.md"),
            "---\nname: model\ndescription: Collides\n---\nBody.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("reviewer.md"),
            "---\nname: reviewer\ndescription: OK\n---\nBody.",
        )
        .unwrap();
        let agents = load_agents_validated(Some(dir.path()), None, &["model", "help", "quit"]);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "reviewer");
    }

    #[test]
    fn awareness_block_with_agents() {
        let agents = vec![
            AgentDefinition {
                name: "reviewer".into(),
                description: "Reviews code".into(),
                model: None,
                tools: None,
                denied_tools: None,
                worktree: None,
                source: AgentSource::Project,
                file_path: PathBuf::new(),
                system_prompt: String::new(),
            },
            AgentDefinition {
                name: "writer".into(),
                description: "Writes tests".into(),
                model: None,
                tools: None,
                denied_tools: None,
                worktree: None,
                source: AgentSource::Global,
                file_path: PathBuf::new(),
                system_prompt: String::new(),
            },
        ];
        let block = build_agent_awareness_block(&agents);
        assert!(block.contains("Available agents"));
        assert!(block.contains("spawn_agent"));
        assert!(block.contains("reviewer: Reviews code"));
        assert!(block.contains("writer: Writes tests"));
    }

    #[test]
    fn awareness_block_empty() {
        assert!(build_agent_awareness_block(&[]).is_empty());
    }

    #[test]
    fn resolve_model_shorthands() {
        assert_eq!(
            resolve_model_shorthand("sonnet"),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(resolve_model_shorthand("opus"), Some("claude-opus-4-6"));
        assert_eq!(
            resolve_model_shorthand("haiku"),
            Some("claude-haiku-4-5-20251001")
        );
    }

    #[test]
    fn resolve_model_unknown_returns_none() {
        assert!(resolve_model_shorthand("gpt-5").is_none());
        assert!(resolve_model_shorthand("unknown").is_none());
    }
}
