//! Inline tool approval bar — renders above the input area.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::tui::theme;

/// Height of the approval bar (top border + 2 content lines + bottom border).
pub const APPROVAL_BAR_HEIGHT: u16 = 4;

/// Render the inline approval bar into the given area.
pub fn render(
    frame: &mut ratatui::Frame,
    area: Rect,
    tool_name: &str,
    args: &serde_json::Value,
    has_diff: bool,
) {
    let colors = theme::Colors::default();

    let summary = format_tool_summary(tool_name, args, area.width.saturating_sub(4) as usize);

    let content = vec![
        Line::from(Span::styled(summary, Style::default().fg(colors.text))),
        build_action_hints(&colors, has_diff),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.warning))
        .title(Span::styled(
            " Tool Approval ",
            Style::default().fg(colors.warning),
        ));

    let paragraph = Paragraph::new(content).block(block);
    frame.render_widget(paragraph, area);
}

/// Build the `[y]es  [n]o  [a]lways` hints line with highlighted key letters.
fn build_action_hints(colors: &theme::Colors, has_diff: bool) -> Line<'static> {
    let key_style = Style::default()
        .fg(colors.warning)
        .add_modifier(Modifier::BOLD);
    let dim_style = Style::default().fg(colors.text_dim);

    let mut spans = vec![
        Span::styled("[", dim_style),
        Span::styled("y", key_style),
        Span::styled("]es  ", dim_style),
        Span::styled("[", dim_style),
        Span::styled("n", key_style),
        Span::styled("]o  ", dim_style),
        Span::styled("[", dim_style),
        Span::styled("a", key_style),
        Span::styled("]lways", dim_style),
    ];

    if has_diff {
        spans.push(Span::styled("  [", dim_style));
        spans.push(Span::styled("d", key_style));
        spans.push(Span::styled("] diff", dim_style));
    }

    Line::from(spans)
}

/// Public summary for use in rejection messages (no width limit).
pub fn format_tool_summary_pub(tool_name: &str, args: &serde_json::Value) -> String {
    format_tool_summary(tool_name, args, usize::MAX)
}

/// Format a human-readable one-line summary for a tool call.
fn format_tool_summary(tool_name: &str, args: &serde_json::Value, max_width: usize) -> String {
    let detail = match tool_name {
        "apply_patch" => summarize_patch(args),
        "write_file" => summarize_write(args),
        "edit_file" => summarize_edit(args),
        "run_command" => summarize_command(args),
        _ => summarize_generic(args),
    };

    let full = format!("{tool_name} \u{2014} {detail}");
    let char_count: usize = full.chars().count();
    if char_count > max_width && max_width > 3 {
        let truncated: String = full.chars().take(max_width - 1).collect();
        format!("{truncated}\u{2026}")
    } else {
        full
    }
}

fn summarize_patch(args: &serde_json::Value) -> String {
    let diff = args.get("diff").and_then(|v| v.as_str()).unwrap_or("");
    let mut paths: Vec<&str> = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let p = rest.trim();
            let p = if let Some(stripped) = p.strip_prefix("b/") {
                stripped
            } else {
                p
            };
            if p != "/dev/null" && !paths.contains(&p) {
                paths.push(p);
            }
        }
    }
    if paths.is_empty() {
        "unified diff".to_string()
    } else if paths.len() == 1 {
        paths[0].to_string()
    } else {
        format!("{} (+{} more)", paths[0], paths.len() - 1)
    }
}

fn summarize_write(args: &serde_json::Value) -> String {
    let path = args
        .get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let len = args
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.len());
    if let Some(n) = len {
        format!("{path} ({n} bytes)")
    } else {
        path.to_string()
    }
}

fn summarize_edit(args: &serde_json::Value) -> String {
    args.get("path")
        .or_else(|| args.get("file_path"))
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string()
}

fn summarize_command(args: &serde_json::Value) -> String {
    args.get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string()
}

fn summarize_generic(args: &serde_json::Value) -> String {
    let s = format!("{args}");
    let char_count = s.chars().count();
    if char_count > 60 {
        let truncated: String = s.chars().take(59).collect();
        format!("{truncated}\u{2026}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_summary_single_file() {
        let args = json!({"diff": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new"});
        assert_eq!(summarize_patch(&args), "src/main.rs");
    }

    #[test]
    fn patch_summary_multi_file() {
        let args = json!({"diff": "--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-x\n+y"});
        let result = summarize_patch(&args);
        assert!(result.contains("a.rs"));
        assert!(result.contains("+1 more"));
    }

    #[test]
    fn write_summary_with_size() {
        let args = json!({"path": "foo.txt", "content": "hello"});
        assert_eq!(summarize_write(&args), "foo.txt (5 bytes)");
    }

    #[test]
    fn edit_summary() {
        let args = json!({"path": "bar.rs", "old_string": "x", "new_string": "y"});
        assert_eq!(summarize_edit(&args), "bar.rs");
    }

    #[test]
    fn command_summary() {
        let args = json!({"command": "cargo test"});
        assert_eq!(summarize_command(&args), "cargo test");
    }

    #[test]
    fn generic_summary_truncates() {
        let long = "x".repeat(100);
        let args = json!({"data": long});
        let result = summarize_generic(&args);
        assert!(result.chars().count() <= 61);
        assert!(result.ends_with('\u{2026}'));
    }

    #[test]
    fn format_tool_summary_truncates_to_width() {
        let args = json!({"command": "a".repeat(100)});
        let result = format_tool_summary("run_command", &args, 40);
        assert!(result.chars().count() <= 40);
    }
}
