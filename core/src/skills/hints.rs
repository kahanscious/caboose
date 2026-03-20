//! Auto-hint detection — suggest skills based on conversation context.

use super::types::SkillHint;
use crate::agent::conversation::{Content, ContentBlock, Message, Role};

/// Scan last `window_size` messages for contextual signals and suggest relevant skills.
pub fn detect_skill_hints(
    messages: &[Message],
    available_skills: &[String],
    window_size: usize,
) -> Vec<SkillHint> {
    let available_set: std::collections::HashSet<&str> =
        available_skills.iter().map(|s| s.as_str()).collect();
    let mut seen = std::collections::HashSet::new();
    let mut hints = Vec::new();

    let start = messages.len().saturating_sub(window_size);
    for msg in &messages[start..] {
        let text = extract_text(msg);
        if text.is_empty() {
            continue;
        }

        // Check each signal
        if available_set.contains("debug")
            && !seen.contains("debug")
            && is_tool_result(msg)
            && has_failure_signal(&text)
        {
            seen.insert("debug");
            hints.push(SkillHint {
                skill_name: "debug".into(),
                reason: "Test failures detected in tool output".into(),
            });
        }

        if available_set.contains("brainstorm")
            && !seen.contains("brainstorm")
            && msg.role == Role::User
            && has_build_signal(&text)
        {
            seen.insert("brainstorm");
            hints.push(SkillHint {
                skill_name: "brainstorm".into(),
                reason: "New feature/change request detected".into(),
            });
        }

        if available_set.contains("finish")
            && !seen.contains("finish")
            && msg.role == Role::Assistant
            && has_completion_signal(&text)
        {
            seen.insert("finish");
            hints.push(SkillHint {
                skill_name: "finish".into(),
                reason: "Implementation appears complete".into(),
            });
        }

        if available_set.contains("review")
            && !seen.contains("review")
            && msg.role == Role::User
            && has_review_signal(&text)
        {
            seen.insert("review");
            hints.push(SkillHint {
                skill_name: "review".into(),
                reason: "Code review requested".into(),
            });
        }
    }

    hints
}

fn extract_text(msg: &Message) -> String {
    match &msg.content {
        Content::Text(t) => t.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn is_tool_result(msg: &Message) -> bool {
    matches!(&msg.content, Content::Blocks(blocks) if blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. })))
}

fn has_failure_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("fail")
        || lower.contains("error:")
        || lower.contains("assertionerror")
        || lower.contains("typeerror")
        || lower.contains("referenceerror")
        || regex::Regex::new(r"exit code [1-9]")
            .unwrap()
            .is_match(&lower)
}

fn has_build_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    let trimmed = lower.trim_start();
    regex::Regex::new(r"^(build|add|create|implement|design|make)\b")
        .unwrap()
        .is_match(trimmed)
}

fn has_completion_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("implementation is complete")
        || lower.contains("all tests pass")
        || lower.contains("ready to merge")
        || lower.contains("ready for review")
}

fn has_review_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("review")
        || lower.contains("check this")
        || lower.contains("look at this code")
        || lower.contains("code quality")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::conversation::{Content, Message, Role};

    fn user_msg(text: &str) -> Message {
        Message {
            role: Role::User,
            content: Content::Text(text.into()),
            tool_call_id: None,
        }
    }

    fn assistant_msg(text: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: Content::Text(text.into()),
            tool_call_id: None,
        }
    }

    fn tool_result(text: &str) -> Message {
        Message {
            role: Role::User,
            content: Content::Blocks(vec![crate::agent::conversation::ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: text.into(),
                is_error: false,
            }]),
            tool_call_id: Some("t1".into()),
        }
    }

    #[test]
    fn detects_debug_on_test_failure() {
        let messages = vec![tool_result("FAIL src/test.rs - expected 1, got 2")];
        let available = vec!["debug".to_string()];
        let hints = detect_skill_hints(&messages, &available, 5);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].skill_name, "debug");
    }

    #[test]
    fn detects_brainstorm_on_build_request() {
        let messages = vec![user_msg("build a new authentication system")];
        let available = vec!["brainstorm".to_string()];
        let hints = detect_skill_hints(&messages, &available, 5);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].skill_name, "brainstorm");
    }

    #[test]
    fn detects_finish_on_completion() {
        let messages = vec![assistant_msg(
            "The implementation is complete. All tests pass.",
        )];
        let available = vec!["finish".to_string()];
        let hints = detect_skill_hints(&messages, &available, 5);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].skill_name, "finish");
    }

    #[test]
    fn detects_review_request() {
        let messages = vec![user_msg("can you review this code?")];
        let available = vec!["review".to_string()];
        let hints = detect_skill_hints(&messages, &available, 5);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].skill_name, "review");
    }

    #[test]
    fn respects_window_size() {
        let mut messages = Vec::new();
        for _ in 0..10 {
            messages.push(user_msg("hello"));
        }
        messages.push(user_msg("build something new"));
        let available = vec!["brainstorm".to_string()];
        // Window of 5 should still see the "build" message (it's the last one)
        let hints = detect_skill_hints(&messages, &available, 5);
        assert_eq!(hints.len(), 1);
    }

    #[test]
    fn only_suggests_available_skills() {
        let messages = vec![tool_result("FAIL test")];
        let available = vec!["brainstorm".to_string()]; // debug not available
        let hints = detect_skill_hints(&messages, &available, 5);
        assert!(hints.is_empty());
    }

    #[test]
    fn deduplicates_hints() {
        let messages = vec![
            tool_result("FAIL test 1"),
            tool_result("Error: connection refused"),
        ];
        let available = vec!["debug".to_string()];
        let hints = detect_skill_hints(&messages, &available, 5);
        assert_eq!(hints.len(), 1); // one hint for debug, not two
    }

    #[test]
    fn empty_messages_no_hints() {
        let hints = detect_skill_hints(&[], &["debug".to_string()], 5);
        assert!(hints.is_empty());
    }
}
