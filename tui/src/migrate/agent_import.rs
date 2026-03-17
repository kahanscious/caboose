use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ImportedAgent {
    pub name: String,
    pub description: String,
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub worktree: Option<bool>,
    pub system_prompt: String,
    pub source_path: PathBuf,
    pub warnings: Vec<String>,
}

impl ImportedAgent {
    pub fn preview_label(&self) -> String {
        let model = self
            .model
            .as_deref()
            .map(|m| format!(" model={m}"))
            .unwrap_or_default();
        let worktree = self
            .worktree
            .map(|w| if w { " worktree" } else { " no-worktree" })
            .unwrap_or("");
        format!("{}{}{}", self.description, model, worktree)
    }
}

pub fn normalize_agent_name(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.chars().flat_map(|c| c.to_lowercase()) {
        let mapped = if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            Some(ch)
        } else if matches!(ch, '-' | '_' | ' ' | '.') {
            Some('-')
        } else {
            None
        };
        match mapped {
            Some('-') if !out.is_empty() && !prev_dash => {
                out.push('-');
                prev_dash = true;
            }
            Some(c) => {
                out.push(c);
                prev_dash = false;
            }
            None => {}
        }
    }
    let out = out.trim_matches('-');
    let mut name = if out.is_empty() {
        "agent".to_string()
    } else {
        out.to_string()
    };
    if !name
        .chars()
        .next()
        .map(|c| c.is_ascii_lowercase())
        .unwrap_or(false)
    {
        name = format!("agent-{name}");
    }
    if name.len() > 40 {
        name.truncate(40);
        name = name.trim_matches('-').to_string();
    }
    if name.is_empty() {
        "agent".to_string()
    } else {
        name
    }
}

pub fn render_caboose_agent_markdown(agent: &ImportedAgent) -> String {
    let mut lines = vec![
        "---".to_string(),
        format!("name: {}", agent.name),
        format!("description: {}", yaml_quote(&agent.description)),
    ];
    if let Some(model) = &agent.model {
        lines.push(format!("model: {}", yaml_quote(model)));
    }
    if let Some(tools) = &agent.tools {
        lines.push(format!("tools: [{}]", join_yaml_list(tools)));
    }
    if let Some(denied_tools) = &agent.denied_tools {
        lines.push(format!("denied_tools: [{}]", join_yaml_list(denied_tools)));
    }
    if let Some(worktree) = agent.worktree {
        lines.push(format!("worktree: {worktree}"));
    }
    lines.push("---".to_string());
    lines.push(agent.system_prompt.trim().to_string());
    lines.join("\n") + "\n"
}

pub fn unique_agent_path(base_dir: &Path, preferred_name: &str, content: &str) -> (PathBuf, bool) {
    let preferred = base_dir.join(format!("{preferred_name}.md"));
    if !preferred.exists() {
        return (preferred, false);
    }
    if std::fs::read_to_string(&preferred).ok().as_deref() == Some(content) {
        return (preferred, false);
    }
    for i in 2..1000 {
        let candidate = base_dir.join(format!("{preferred_name}-{i}.md"));
        if !candidate.exists() {
            return (candidate, true);
        }
    }
    (preferred, true)
}

pub fn tool_allow_list_from_names(names: &[String]) -> (Option<Vec<String>>, Vec<String>) {
    let mut mapped = BTreeSet::new();
    let mut warnings = Vec::new();
    for name in names {
        let tool_names = map_tool_name(name);
        if tool_names.is_empty() {
            warnings.push(format!("Skipped unsupported tool '{name}'"));
            continue;
        }
        mapped.extend(tool_names);
    }
    if mapped.is_empty() {
        (None, warnings)
    } else {
        (Some(mapped.into_iter().collect()), warnings)
    }
}

pub fn tool_deny_list_from_names(names: &[String]) -> (Option<Vec<String>>, Vec<String>) {
    let mut mapped = BTreeSet::new();
    let mut warnings = Vec::new();
    for name in names {
        let tool_names = map_tool_name(name);
        if tool_names.is_empty() {
            warnings.push(format!("Skipped unsupported tool '{name}'"));
            continue;
        }
        mapped.extend(tool_names);
    }
    if mapped.is_empty() {
        (None, warnings)
    } else {
        (Some(mapped.into_iter().collect()), warnings)
    }
}

pub fn map_tool_name(name: &str) -> Vec<String> {
    let normalized = name
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_lowercase()
        .replace(' ', "_");
    match normalized.as_str() {
        "read" | "read_file" => vec!["read_file".into(), "list_directory".into()],
        "write" | "write_file" => {
            vec![
                "write_file".into(),
                "edit_file".into(),
                "apply_patch".into(),
            ]
        }
        "edit" | "edit_file" => vec!["edit_file".into(), "apply_patch".into()],
        "patch" | "apply_patch" => vec!["apply_patch".into()],
        "glob" => vec!["glob".into()],
        "grep" => vec!["grep".into()],
        "bash" | "run_command" | "command" => vec!["run_command".into()],
        "fetch" | "webfetch" => vec!["fetch".into(), "web_search".into()],
        "diagnostics" => vec!["diagnostics".into()],
        "lsp" => vec!["lsp".into()],
        "todo" | "todo_write" => vec!["todo_write".into(), "todo_read".into()],
        _ => Vec::new(),
    }
}

fn join_yaml_list(values: &[String]) -> String {
    values
        .iter()
        .map(|v| yaml_quote(v))
        .collect::<Vec<_>>()
        .join(", ")
}

fn yaml_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_agent_name_sanitizes() {
        assert_eq!(normalize_agent_name("Code Reviewer"), "code-reviewer");
        assert_eq!(normalize_agent_name("123helper"), "agent-123helper");
    }

    #[test]
    fn tool_mapper_translates_common_names() {
        assert_eq!(
            map_tool_name("Write"),
            vec!["write_file", "edit_file", "apply_patch"]
        );
        assert_eq!(map_tool_name("Bash"), vec!["run_command"]);
    }

    #[test]
    fn render_agent_markdown_includes_frontmatter() {
        let agent = ImportedAgent {
            name: "reviewer".into(),
            description: "Review code".into(),
            model: Some("anthropic/claude-sonnet-4-6".into()),
            tools: Some(vec!["read_file".into()]),
            denied_tools: None,
            worktree: Some(false),
            system_prompt: "You are a reviewer.".into(),
            source_path: PathBuf::from("src.md"),
            warnings: vec![],
        };
        let rendered = render_caboose_agent_markdown(&agent);
        assert!(rendered.contains("name: reviewer"));
        assert!(rendered.contains("tools: [\"read_file\"]"));
        assert!(rendered.contains("worktree: false"));
    }
}
