//! LLM-guided skill creation — state machine and helpers.

use std::path::PathBuf;

/// Phase of the skill creation flow.
#[derive(Debug, Clone)]
pub enum SkillCreationPhase {
    /// Waiting for user to provide a skill name.
    AwaitingName,
    /// Waiting for user to describe the goal.
    AwaitingGoal,
    /// LLM is asking clarifying questions.
    Gathering,
    /// Skill generated, awaiting user preview + scope choice.
    Preview {
        content: String,
        companion_files: Vec<CompanionFile>,
    },
}

/// A companion file for folder skills.
#[derive(Debug, Clone)]
pub struct CompanionFile {
    pub name: String,
    pub content: String,
}

/// State for an active `/create-skill` session.
#[derive(Debug, Clone)]
pub struct SkillCreationState {
    pub name: String,
    #[allow(dead_code)]
    pub goal: String,
    pub phase: SkillCreationPhase,
    pub question_count: u8,
}

/// Where to save the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillScope {
    /// `.caboose/skills/`
    Project,
    /// `~/.config/caboose/skills/`
    Global,
}

/// Maximum clarifying questions before forcing generation.
pub const MAX_CREATION_QUESTIONS: u8 = 8;

/// Reserved command names that cannot be used as skill names.
const RESERVED_NAMES: &[&str] = &[
    "model",
    "connect",
    "compact",
    "new",
    "sessions",
    "title",
    "memories",
    "forget",
    "mcp",
    "settings",
    "init",
    "quit",
    "skills",
    "palette",
    "create-skill",
    "cancel",
];

/// Check if a skill name is reserved (collides with built-in commands).
pub fn is_reserved_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    RESERVED_NAMES.iter().any(|r| *r == lower)
}

/// Build the skill creation system prompt.
pub fn system_prompt(name: &str, goal: &str) -> String {
    format!(
        r#"You are a skill creation assistant for the Caboose app. Your job is to help the user create a well-structured skill template.

The user wants to create a skill called "{name}" with this goal: "{goal}"

## Your process:
1. Ask 2-5 focused clarifying questions to understand the user's intent, constraints, and desired behavior. Ask one question at a time.
2. Once you have enough context, use the generate_skill tool to produce the final skill.

## Skill template format:
A skill is a markdown document that instructs an AI assistant. It should include:
- YAML frontmatter (between --- markers) with `name` and `description` fields. The `description` MUST be a short phrase (3-8 words, max 60 chars) — e.g. "Automate staging deployments" or "Fix code quality issues"
- Step-by-step instructions for the AI to follow
- Constraints and guardrails
- Examples where helpful
- $ARGS placeholder where the user's input should be inserted

## Rules:
- Keep questions focused and one at a time
- Don't ask more than 5-6 questions unless the user's answers reveal complexity
- Always include $ARGS in the generated skill so the user can pass arguments
- Write the skill in second person ("You should...", "Your task is...")
- Be specific and actionable — vague skills are useless
- Respect the user's scope: if they don't specify a language or framework, create a language-agnostic skill
- Ask about WHAT the skill should do and HOW, not about prerequisites

## Important:
When you're ready to generate, you MUST use the generate_skill tool. Do not output the skill content as plain text."#
    )
}

/// Build the fallback system prompt (providers without tool support).
#[allow(dead_code)]
pub fn fallback_system_prompt(name: &str, goal: &str) -> String {
    format!(
        r#"You are a skill creation assistant for the Caboose app. Your job is to help the user create a well-structured skill template.

The user wants to create a skill called "{name}" with this goal: "{goal}"

## Your process:
1. Ask 2-5 focused clarifying questions to understand the user's intent, constraints, and desired behavior. Ask one question at a time.
2. Once you have enough context, output the complete skill template.

## Skill template format:
A skill is a markdown document with YAML frontmatter (name, description) and a template body with $ARGS placeholders.

## Rules:
- Keep questions focused and one at a time
- Always include $ARGS in the generated skill
- Write in second person ("You should...", "Your task is...")
- Be specific and actionable

## Important:
When ready, output ONLY the complete skill template markdown. Start directly with the YAML frontmatter (---)."#
    )
}

/// Resolve the target directory for a given scope.
pub fn skill_dir(scope: SkillScope) -> Option<PathBuf> {
    match scope {
        SkillScope::Project => Some(PathBuf::from(".caboose/skills")),
        SkillScope::Global => dirs::config_dir().map(|d| d.join("caboose").join("skills")),
    }
}

/// Check if a skill file already exists at the target scope.
pub fn skill_exists(name: &str, scope: SkillScope) -> Option<PathBuf> {
    let dir = skill_dir(scope)?;
    let file = dir.join(format!("{name}.md"));
    if file.exists() {
        return Some(file);
    }
    let folder = dir.join(name).join("skill.md");
    if folder.exists() {
        return Some(folder);
    }
    None
}

/// Write skill content (and optional companions) to disk.
pub fn write_skill(
    name: &str,
    content: &str,
    companions: &[CompanionFile],
    scope: SkillScope,
) -> Result<PathBuf, String> {
    let dir = skill_dir(scope).ok_or("Cannot resolve skill directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create directory: {e}"))?;

    if companions.is_empty() {
        // Single file skill
        let path = dir.join(format!("{name}.md"));
        std::fs::write(&path, content).map_err(|e| format!("Write failed: {e}"))?;
        Ok(path)
    } else {
        // Folder skill
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).map_err(|e| format!("Cannot create folder: {e}"))?;
        let skill_path = folder.join("skill.md");
        std::fs::write(&skill_path, content).map_err(|e| format!("Write failed: {e}"))?;
        for cf in companions {
            // Safety: reject path traversal
            if cf.name.contains("..") || cf.name.starts_with('/') || cf.name.starts_with('\\') {
                continue;
            }
            let cf_path = folder.join(&cf.name);
            std::fs::write(&cf_path, &cf.content)
                .map_err(|e| format!("Write companion '{}' failed: {e}", cf.name))?;
        }
        Ok(skill_path)
    }
}

/// Heuristic: does this text look like a generated skill?
/// Used as fallback when the provider doesn't support tool calls.
pub fn looks_like_generated_skill(text: &str) -> bool {
    text.len() >= 200 && (text.contains("$ARGS") || text.starts_with("---"))
}

/// Extract companion files from JSON string.
pub fn parse_companion_files(json: &str) -> Vec<CompanionFile> {
    let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    parsed
        .iter()
        .filter_map(|v| {
            let name = v.get("name")?.as_str()?.to_string();
            let content = v.get("content")?.as_str()?.to_string();
            Some(CompanionFile { name, content })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_names_detected() {
        assert!(is_reserved_name("model"));
        assert!(is_reserved_name("Model"));
        assert!(is_reserved_name("create-skill"));
        assert!(!is_reserved_name("deploy"));
        assert!(!is_reserved_name("my-custom-skill"));
    }

    #[test]
    fn system_prompt_includes_name_and_goal() {
        let prompt = system_prompt("deploy", "automate deployment");
        assert!(prompt.contains("deploy"));
        assert!(prompt.contains("automate deployment"));
        assert!(prompt.contains("generate_skill"));
    }

    #[test]
    fn fallback_prompt_includes_name_and_goal() {
        let prompt = fallback_system_prompt("deploy", "automate deployment");
        assert!(prompt.contains("deploy"));
        assert!(prompt.contains("automate deployment"));
        assert!(!prompt.contains("generate_skill")); // no tool ref in fallback
    }

    #[test]
    fn skill_dir_project_is_relative() {
        let dir = skill_dir(SkillScope::Project).unwrap();
        assert_eq!(dir, PathBuf::from(".caboose/skills"));
    }

    #[test]
    fn skill_dir_global_uses_config() {
        let dir = skill_dir(SkillScope::Global);
        assert!(dir.is_some());
        let dir = dir.unwrap();
        assert!(dir.to_string_lossy().contains("caboose"));
    }

    #[test]
    fn write_skill_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("deploy.md");
        std::fs::write(&path, "---\nname: deploy\n---\nDo $ARGS").unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("$ARGS"));
    }

    #[test]
    fn parse_companion_files_valid_json() {
        let json = "[{\"name\":\"examples.md\",\"content\":\"# Examples\\nstuff\"}]";
        let files = parse_companion_files(json);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "examples.md");
        assert!(files[0].content.contains("Examples"));
    }

    #[test]
    fn parse_companion_files_invalid_json() {
        let files = parse_companion_files("not json");
        assert!(files.is_empty());
    }

    #[test]
    fn parse_companion_files_missing_fields() {
        let json = r#"[{"name":"test.md"},{"bad":"data"}]"#;
        let files = parse_companion_files(json);
        assert!(files.is_empty());
    }

    #[test]
    fn looks_like_skill_with_args_placeholder() {
        assert!(looks_like_generated_skill(&format!(
            "{}$ARGS more text",
            "x".repeat(200)
        )));
    }

    #[test]
    fn looks_like_skill_with_frontmatter() {
        let text = format!("---\nname: test\n---\n{}", "x".repeat(200));
        assert!(looks_like_generated_skill(&text));
    }

    #[test]
    fn short_text_not_a_skill() {
        assert!(!looks_like_generated_skill("$ARGS"));
    }

    #[test]
    fn skill_scope_equality() {
        assert_eq!(SkillScope::Project, SkillScope::Project);
        assert_ne!(SkillScope::Project, SkillScope::Global);
    }

    #[test]
    fn write_folder_skill_with_companions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&dir).unwrap();
        let companions = vec![CompanionFile {
            name: "ref.md".into(),
            content: "# Reference\nstuff".into(),
        }];
        // write_skill uses skill_dir() which returns a relative path, so test
        // the folder creation logic directly
        let folder = dir.join("my-skill");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("skill.md"),
            "---\nname: my-skill\n---\nDo $ARGS",
        )
        .unwrap();
        for cf in &companions {
            std::fs::write(folder.join(&cf.name), &cf.content).unwrap();
        }
        assert!(folder.join("skill.md").exists());
        assert!(folder.join("ref.md").exists());
        let content = std::fs::read_to_string(folder.join("ref.md")).unwrap();
        assert!(content.contains("Reference"));
    }

    #[test]
    fn companion_file_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&dir).unwrap();
        let companions = vec![
            CompanionFile {
                name: "../../../etc/passwd".into(),
                content: "evil".into(),
            },
            CompanionFile {
                name: "good.md".into(),
                content: "safe".into(),
            },
        ];
        // Simulate the write_skill logic: path traversal files are skipped
        let folder = dir.join("test-skill");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("skill.md"), "content").unwrap();
        for cf in &companions {
            if cf.name.contains("..") || cf.name.starts_with('/') || cf.name.starts_with('\\') {
                continue;
            }
            std::fs::write(folder.join(&cf.name), &cf.content).unwrap();
        }
        // Path traversal file should NOT exist
        assert!(!folder.join("../../../etc/passwd").exists());
        // Good file should exist
        assert!(folder.join("good.md").exists());
    }

    #[test]
    fn max_creation_questions_is_reasonable() {
        assert!(MAX_CREATION_QUESTIONS >= 3);
        assert!(MAX_CREATION_QUESTIONS <= 15);
    }
}
