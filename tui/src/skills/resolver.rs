//! 3-tier slash name resolution: client commands > skills > unknown.

use super::types::{Skill, SlashResolution};

/// Resolve a `/name` to a command, skill, or unknown.
pub fn resolve_slash_name(
    name: &str,
    client_commands: &[&str],
    skills: &[Skill],
) -> SlashResolution {
    let normalized = name.trim().to_lowercase();

    // Tier 1: client commands
    if client_commands
        .iter()
        .any(|c| c.to_lowercase() == normalized)
    {
        return SlashResolution::Command(normalized);
    }

    // Tier 2: skills (first match)
    if let Some(skill) = skills.iter().find(|s| s.name.to_lowercase() == normalized) {
        return SlashResolution::Skill(skill.clone());
    }

    // Tier 3: unknown
    SlashResolution::Unknown(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::*;

    fn make_skill(name: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("{name} skill"),
            template: format!("Template for {name}: $ARGS"),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        }
    }

    #[test]
    fn command_wins_over_skill() {
        let commands = vec!["model", "connect"];
        let skills = vec![make_skill("model")];
        let result = resolve_slash_name("model", &commands, &skills);
        assert!(matches!(result, SlashResolution::Command(_)));
    }

    #[test]
    fn skill_found_when_no_command() {
        let commands = vec!["model"];
        let skills = vec![make_skill("brainstorm")];
        let result = resolve_slash_name("brainstorm", &commands, &skills);
        assert!(matches!(result, SlashResolution::Skill(_)));
    }

    #[test]
    fn unknown_when_no_match() {
        let commands = vec!["model"];
        let skills = vec![make_skill("brainstorm")];
        let result = resolve_slash_name("nonexistent", &commands, &skills);
        assert!(matches!(result, SlashResolution::Unknown(_)));
    }

    #[test]
    fn case_insensitive_match() {
        let commands: Vec<&str> = vec![];
        let skills = vec![make_skill("brainstorm")];
        let result = resolve_slash_name("BRAINSTORM", &commands, &skills);
        assert!(matches!(result, SlashResolution::Skill(_)));
    }

    #[test]
    fn whitespace_trimmed() {
        let commands: Vec<&str> = vec![];
        let skills = vec![make_skill("debug")];
        let result = resolve_slash_name("  debug  ", &commands, &skills);
        assert!(matches!(result, SlashResolution::Skill(_)));
    }

    #[test]
    fn create_skill_resolves_as_command() {
        let skills = vec![];
        let commands = vec!["model", "create-skill", "quit"];
        let result = resolve_slash_name("create-skill", &commands, &skills);
        assert!(matches!(result, SlashResolution::Command(_)));
    }
}
