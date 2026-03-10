//! Command policy — allow/deny list for shell command execution.

/// Policy decision for a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    RequireApproval,
}

/// Evaluate whether a command should be allowed.
pub fn check(command: &str, allow_list: &[String], deny_list: &[String]) -> Decision {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let Some(cmd) = parts.first() else {
        return Decision::Deny("empty command".to_string());
    };

    // Check deny list first
    for pattern in deny_list {
        if cmd == pattern || command.contains(pattern) {
            return Decision::Deny(format!("command matches deny pattern: {pattern}"));
        }
    }

    // Check allow list
    for pattern in allow_list {
        if cmd == pattern {
            return Decision::Allow;
        }
    }

    // Default: require approval
    Decision::RequireApproval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_is_denied() {
        let result = check("", &[], &[]);
        assert!(matches!(result, Decision::Deny(_)));
    }

    #[test]
    fn whitespace_only_command_is_denied() {
        let result = check("   ", &[], &[]);
        assert!(matches!(result, Decision::Deny(_)));
    }

    #[test]
    fn deny_list_blocks_command() {
        let result = check("rm -rf /", &[], &["rm".into()]);
        assert!(matches!(result, Decision::Deny(_)));
    }

    #[test]
    fn allow_list_permits_command() {
        let result = check("ls -la", &["ls".into()], &[]);
        assert!(matches!(result, Decision::Allow));
    }

    #[test]
    fn unknown_command_requires_approval() {
        let result = check("curl https://example.com", &[], &[]);
        assert!(matches!(result, Decision::RequireApproval));
    }

    #[test]
    fn deny_list_checked_before_allow_list() {
        let result = check("rm file.txt", &["rm".into()], &["rm".into()]);
        assert!(matches!(result, Decision::Deny(_)));
    }

    #[test]
    fn chained_command_with_denied_segment() {
        let result = check("echo hello && rm -rf /", &[], &["rm".into()]);
        assert!(matches!(result, Decision::Deny(_)));
    }
}
