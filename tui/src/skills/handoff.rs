//! Programmatic handoff summary builder.

use std::collections::HashMap;

/// File modification stats for the handoff summary.
pub struct HandoffFileStats {
    pub additions: usize,
    pub deletions: usize,
}

/// Input data for building a handoff summary.
pub struct HandoffContext<'a> {
    pub session_id: Option<&'a str>,
    pub session_title: Option<&'a str>,
    pub provider_name: Option<&'a str>,
    pub model_name: Option<&'a str>,
    pub turn_count: u32,
    pub user_messages: Vec<&'a str>,
    pub modified_files: &'a HashMap<String, HandoffFileStats>,
    pub tool_counts: &'a HashMap<String, u32>,
    pub open_tasks: Vec<&'a str>,
    pub focus: Option<&'a str>,
}

/// Build a structured markdown handoff summary from session context.
pub fn build_handoff_summary(ctx: &HandoffContext<'_>) -> String {
    let mut out = String::from("## Handoff Summary\n");

    // Session metadata line
    let sid = ctx
        .session_id
        .map(|s| &s[..8.min(s.len())])
        .unwrap_or("unknown");
    let provider = ctx.provider_name.unwrap_or("unknown");
    let model = ctx.model_name.unwrap_or("unknown");
    out.push_str(&format!(
        "Session: {} | Provider: {}/{} | Turns: {}\n\n",
        sid, provider, model, ctx.turn_count
    ));

    // Title if present
    if let Some(title) = ctx.session_title {
        out.push_str(&format!("**{}**\n\n", title));
    }

    // What was done — user messages as bullet points
    if !ctx.user_messages.is_empty() {
        out.push_str("### What was done\n");
        for msg in &ctx.user_messages {
            // Take first line, truncate at 120 chars
            let first_line = msg.lines().next().unwrap_or(msg);
            let truncated = if first_line.len() > 120 {
                format!("{}...", &first_line[..117])
            } else {
                first_line.to_string()
            };
            out.push_str(&format!("- {}\n", truncated));
        }
        out.push('\n');
    }

    // Files modified
    if !ctx.modified_files.is_empty() {
        out.push_str("### Files modified\n");
        let mut files: Vec<_> = ctx.modified_files.iter().collect();
        files.sort_by_key(|(k, _)| k.as_str());
        for (path, stats) in &files {
            if stats.additions > 0 || stats.deletions > 0 {
                out.push_str(&format!(
                    "- {} (+{}/-{})\n",
                    path, stats.additions, stats.deletions
                ));
            } else {
                out.push_str(&format!("- {} (read)\n", path));
            }
        }
        out.push('\n');
    }

    // Tools used
    if !ctx.tool_counts.is_empty() {
        out.push_str("### Tools used\n");
        let mut tools: Vec<_> = ctx.tool_counts.iter().collect();
        tools.sort_by_key(|(k, _)| k.as_str());
        let parts: Vec<String> = tools
            .iter()
            .map(|(name, count)| format!("{} ({}x)", name, count))
            .collect();
        out.push_str(&format!("- {}\n\n", parts.join(", ")));
    }

    // Open tasks
    if !ctx.open_tasks.is_empty() {
        out.push_str("### Open tasks\n");
        for task in &ctx.open_tasks {
            out.push_str(&format!("- [ ] {}\n", task));
        }
        out.push('\n');
    }

    // Focus
    if let Some(focus) = ctx.focus
        && !focus.is_empty()
    {
        out.push_str(&format!("### Focus\n{}\n", focus));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates empty HashMaps and a default `HandoffContext` referencing them.
    ///
    /// Because `modified_files` and `tool_counts` are borrowed references,
    /// the HashMaps must outlive the context. Each test creates its own
    /// empty maps via `let` bindings, then calls this helper.
    fn empty_ctx<'a>(
        files: &'a HashMap<String, HandoffFileStats>,
        tools: &'a HashMap<String, u32>,
    ) -> HandoffContext<'a> {
        HandoffContext {
            session_id: None,
            session_title: None,
            provider_name: None,
            model_name: None,
            turn_count: 0,
            user_messages: vec![],
            modified_files: files,
            tool_counts: tools,
            open_tasks: vec![],
            focus: None,
        }
    }

    #[test]
    fn empty_context_produces_header() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let summary = build_handoff_summary(&empty_ctx(&files, &tools));
        assert!(summary.starts_with("## Handoff Summary"));
        assert!(summary.contains("Session: unknown"));
        assert!(summary.contains("Turns: 0"));
    }

    #[test]
    fn includes_session_metadata() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let ctx = HandoffContext {
            session_id: Some("abcdef12-3456-7890"),
            provider_name: Some("anthropic"),
            model_name: Some("claude-sonnet"),
            turn_count: 14,
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("Session: abcdef12"));
        assert!(summary.contains("anthropic/claude-sonnet"));
        assert!(summary.contains("Turns: 14"));
    }

    #[test]
    fn includes_title() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let ctx = HandoffContext {
            session_title: Some("Fix auth middleware"),
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("**Fix auth middleware**"));
    }

    #[test]
    fn includes_user_messages() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let ctx = HandoffContext {
            user_messages: vec!["Add login endpoint", "Fix the CORS issue"],
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("### What was done"));
        assert!(summary.contains("- Add login endpoint"));
        assert!(summary.contains("- Fix the CORS issue"));
    }

    #[test]
    fn truncates_long_messages() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let long_msg = "a".repeat(200);
        let ctx = HandoffContext {
            user_messages: vec![&long_msg],
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("..."));
        assert!(!summary.contains(&long_msg));
    }

    #[test]
    fn includes_modified_files() {
        let mut files = HashMap::new();
        files.insert(
            "src/auth.rs".into(),
            HandoffFileStats {
                additions: 45,
                deletions: 12,
            },
        );
        files.insert(
            "src/config.rs".into(),
            HandoffFileStats {
                additions: 0,
                deletions: 0,
            },
        );
        let tools = HashMap::new();
        let ctx = HandoffContext {
            modified_files: &files,
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("### Files modified"));
        assert!(summary.contains("src/auth.rs (+45/-12)"));
        assert!(summary.contains("src/config.rs (read)"));
    }

    #[test]
    fn includes_tool_counts() {
        let files = HashMap::new();
        let mut tools = HashMap::new();
        tools.insert("read_file".into(), 4);
        tools.insert("edit_file".into(), 3);
        let ctx = HandoffContext {
            tool_counts: &tools,
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("### Tools used"));
        assert!(summary.contains("edit_file (3x)"));
        assert!(summary.contains("read_file (4x)"));
    }

    #[test]
    fn includes_open_tasks() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let ctx = HandoffContext {
            open_tasks: vec!["Write integration tests", "Add rate limiting"],
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("### Open tasks"));
        assert!(summary.contains("- [ ] Write integration tests"));
    }

    #[test]
    fn includes_focus() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let ctx = HandoffContext {
            focus: Some("Focus on the authentication refactor"),
            ..empty_ctx(&files, &tools)
        };
        let summary = build_handoff_summary(&ctx);
        assert!(summary.contains("### Focus"));
        assert!(summary.contains("Focus on the authentication refactor"));
    }

    #[test]
    fn omits_empty_sections() {
        let files = HashMap::new();
        let tools = HashMap::new();
        let summary = build_handoff_summary(&empty_ctx(&files, &tools));
        assert!(!summary.contains("### What was done"));
        assert!(!summary.contains("### Files modified"));
        assert!(!summary.contains("### Tools used"));
        assert!(!summary.contains("### Open tasks"));
        assert!(!summary.contains("### Focus"));
    }
}
