//! Template placeholder expansion — $ARGS and $FILE.

use super::types::Skill;

/// Expand a skill template, replacing $ARGS and $FILE placeholders.
pub fn expand_skill(skill: &Skill, args: &str, working_dir: &str) -> String {
    skill
        .template
        .replace("$ARGS", args)
        .replace("$FILE", working_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::types::*;

    fn make_skill(template: &str) -> Skill {
        Skill {
            name: "test".into(),
            description: "test".into(),
            template: template.into(),
            source: SkillSource::Builtin,
            response_format: ResponseFormat::Prose,
        }
    }

    #[test]
    fn expands_args() {
        let skill = make_skill("Do $ARGS now.");
        let result = expand_skill(&skill, "fix the bug", "/home/user/project");
        assert_eq!(result, "Do fix the bug now.");
    }

    #[test]
    fn expands_file() {
        let skill = make_skill("Working in $FILE.");
        let result = expand_skill(&skill, "", "/home/user/project");
        assert_eq!(result, "Working in /home/user/project.");
    }

    #[test]
    fn expands_multiple_occurrences() {
        let skill = make_skill("$ARGS then $ARGS again in $FILE");
        let result = expand_skill(&skill, "test", "/proj");
        assert_eq!(result, "test then test again in /proj");
    }

    #[test]
    fn no_placeholders_unchanged() {
        let skill = make_skill("Just plain text.");
        let result = expand_skill(&skill, "args", "/dir");
        assert_eq!(result, "Just plain text.");
    }
}
