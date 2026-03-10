//! Skill system types.

use std::path::PathBuf;

/// A skill loaded from disk or built-in.
#[derive(Debug, Clone)]
pub struct Skill {
    /// Lowercase name, e.g., "brainstorm". Used as `/brainstorm`.
    pub name: String,
    /// Short description (first non-empty line of body, headers stripped).
    pub description: String,
    /// Full markdown template (frontmatter stripped).
    pub template: String,
    /// Where this skill came from.
    pub source: SkillSource,
    /// How the response should be rendered.
    #[allow(dead_code)]
    pub response_format: ResponseFormat,
}

/// Origin of a skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillSource {
    /// Shipped with the binary.
    Builtin,
    /// Loaded from a file on disk.
    File(PathBuf),
}

/// How the LLM response should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseFormat {
    /// Conversational prose (chat mode).
    #[default]
    Prose,
    /// Structured plan steps (create mode).
    Plan,
}

/// Result of resolving a `/slash` name.
#[derive(Debug)]
#[allow(dead_code)]
pub enum SlashResolution {
    /// Matches a built-in client command (e.g., `/model`, `/connect`).
    Command(String),
    /// Matches a skill (user or built-in).
    Skill(Skill),
    /// No match found.
    Unknown(String),
}

/// A context-based skill suggestion.
#[derive(Debug, Clone)]
pub struct SkillHint {
    /// Skill name to suggest, e.g., "debug".
    pub skill_name: String,
    /// Why it was suggested, e.g., "Test failures detected".
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_source_equality() {
        assert_eq!(SkillSource::Builtin, SkillSource::Builtin);
        assert_ne!(SkillSource::Builtin, SkillSource::File(PathBuf::from("x")));
    }

    #[test]
    fn response_format_default_is_prose() {
        assert_eq!(ResponseFormat::default(), ResponseFormat::Prose);
    }
}
