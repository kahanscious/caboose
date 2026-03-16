//! 4-tier slash name resolution: client commands > agents > skills > unknown.

use super::types::{Skill, SlashResolution};
use crate::agents::AgentDefinition;

/// Resolve a `/name` to a command, agent, skill, or unknown.
pub fn resolve_slash_name(
    name: &str,
    client_commands: &[&str],
    agents: &[AgentDefinition],
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

    // Tier 2: custom agents
    if let Some(agent) = agents.iter().find(|a| a.name == normalized) {
        return SlashResolution::Agent(agent.clone());
    }

    // Tier 3: skills (first match)
    if let Some(skill) = skills.iter().find(|s| s.name.to_lowercase() == normalized) {
        return SlashResolution::Skill(skill.clone());
    }

    // Tier 4: unknown
    SlashResolution::Unknown(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentSource;
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

    fn make_agent(name: &str) -> AgentDefinition {
        AgentDefinition {
            name: name.to_string(),
            description: format!("{name} agent"),
            model: None,
            tools: None,
            denied_tools: None,
            worktree: None,
            source: AgentSource::Project,
            file_path: std::path::PathBuf::new(),
            system_prompt: String::new(),
        }
    }

    #[test]
    fn command_wins_over_skill() {
        let commands = vec!["model", "connect"];
        let skills = vec![make_skill("model")];
        let result = resolve_slash_name("model", &commands, &[], &skills);
        assert!(matches!(result, SlashResolution::Command(_)));
    }

    #[test]
    fn command_wins_over_agent() {
        let commands = vec!["model"];
        let agents = vec![make_agent("model")];
        let result = resolve_slash_name("model", &commands, &agents, &[]);
        assert!(matches!(result, SlashResolution::Command(_)));
    }

    #[test]
    fn agent_wins_over_skill() {
        let agents = vec![make_agent("reviewer")];
        let skills = vec![make_skill("reviewer")];
        let result = resolve_slash_name("reviewer", &[], &agents, &skills);
        assert!(matches!(result, SlashResolution::Agent(_)));
    }

    #[test]
    fn agent_found_when_no_command() {
        let agents = vec![make_agent("reviewer")];
        let result = resolve_slash_name("reviewer", &[], &agents, &[]);
        assert!(matches!(result, SlashResolution::Agent(_)));
    }

    #[test]
    fn skill_found_when_no_command_or_agent() {
        let commands = vec!["model"];
        let skills = vec![make_skill("brainstorm")];
        let result = resolve_slash_name("brainstorm", &commands, &[], &skills);
        assert!(matches!(result, SlashResolution::Skill(_)));
    }

    #[test]
    fn unknown_when_no_match() {
        let commands = vec!["model"];
        let skills = vec![make_skill("brainstorm")];
        let result = resolve_slash_name("nonexistent", &commands, &[], &skills);
        assert!(matches!(result, SlashResolution::Unknown(_)));
    }

    #[test]
    fn case_insensitive_match() {
        let commands: Vec<&str> = vec![];
        let skills = vec![make_skill("brainstorm")];
        let result = resolve_slash_name("BRAINSTORM", &commands, &[], &skills);
        assert!(matches!(result, SlashResolution::Skill(_)));
    }

    #[test]
    fn whitespace_trimmed() {
        let commands: Vec<&str> = vec![];
        let skills = vec![make_skill("debug")];
        let result = resolve_slash_name("  debug  ", &commands, &[], &skills);
        assert!(matches!(result, SlashResolution::Skill(_)));
    }

    #[test]
    fn create_skill_resolves_as_command() {
        let skills = vec![];
        let commands = vec!["model", "create-skill", "quit"];
        let result = resolve_slash_name("create-skill", &commands, &[], &skills);
        assert!(matches!(result, SlashResolution::Command(_)));
    }
}
