//! Generate awareness block for system prompt injection.

use super::types::{Skill, SkillHint};

/// Build the skill awareness block for injection into the system prompt.
/// Returns empty string if no skills are available.
pub fn build_awareness_block(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let mut block = String::from(
        "\n\n## Available skills\n\n\
         You have access to workflow skills that can be invoked with /command-name.\n\
         When you recognize a matching situation, suggest the relevant skill to the user.\n\n",
    );

    for skill in skills {
        block.push_str(&format!("- /{} — {}\n", skill.name, skill.description));
    }

    block
}

/// Check if context usage is high enough to suggest a handoff.
/// Returns a hint if estimated tokens exceed 90% of the context window.
pub fn detect_handoff_hint(estimated_tokens: u32, context_window: u32) -> Option<SkillHint> {
    if context_window == 0 {
        return None;
    }
    let usage_pct = (estimated_tokens as f64 / context_window as f64) * 100.0;
    if usage_pct >= 90.0 {
        Some(SkillHint {
            skill_name: "handoff".into(),
            reason: format!(
                "Context is {:.0}% full — consider /handoff to continue in a fresh session",
                usage_pct
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::*;

    fn make_skill(name: &str, desc: &str) -> Skill {
        Skill {
            name: name.into(),
            description: desc.into(),
            template: String::new(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        }
    }

    #[test]
    fn generates_block_with_skills() {
        let skills = vec![
            make_skill("brainstorm", "Design exploration"),
            make_skill("debug", "Isolate faults"),
        ];
        let block = build_awareness_block(&skills);
        assert!(block.contains("## Available skills"));
        assert!(block.contains("/brainstorm"));
        assert!(block.contains("/debug"));
        assert!(block.contains("Design exploration"));
    }

    #[test]
    fn empty_skills_returns_empty() {
        let block = build_awareness_block(&[]);
        assert!(block.is_empty());
    }

    #[test]
    fn no_hint_for_low_context() {
        assert!(detect_handoff_hint(1000, 200_000).is_none());
    }

    #[test]
    fn hint_at_high_context_usage() {
        // 190k / 200k = 95%
        let hint = detect_handoff_hint(190_000, 200_000).unwrap();
        assert_eq!(hint.skill_name, "handoff");
        assert!(hint.reason.contains("95%"));
    }

    #[test]
    fn hint_at_exactly_90_pct() {
        assert!(detect_handoff_hint(180_000, 200_000).is_some());
    }

    #[test]
    fn no_hint_below_90_pct() {
        // 89% should not trigger
        assert!(detect_handoff_hint(178_000, 200_000).is_none());
    }

    #[test]
    fn no_hint_zero_context_window() {
        assert!(detect_handoff_hint(50_000, 0).is_none());
    }
}
