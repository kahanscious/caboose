//! Write/Edit file tool renderer.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::{ToolMessage, ToolStatus};
use crate::tui::theme::Colors;

pub struct WriteRenderer;

impl ToolRenderer for WriteRenderer {
    fn handles(&self) -> &[&str] {
        &["write_file", "edit_file", "apply_patch"]
    }

    fn render(&self, tool: &ToolMessage, colors: &Colors, tick: u64) -> Vec<Line<'static>> {
        render(tool, colors, tick)
    }
}

pub fn render(tool: &ToolMessage, colors: &Colors, tick: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let icon = super::status_icon(&tool.status, colors, tick);
    let label = match tool.name.as_str() {
        "edit_file" => "Edit",
        "apply_patch" => "Patch",
        _ => "Write",
    };

    // For apply_patch, extract file paths from the diff content
    let detail = if tool.name == "apply_patch" {
        extract_patch_files(&tool.args)
    } else {
        tool.file_path.clone().unwrap_or_else(|| "?".to_string())
    };

    lines.push(Line::from(vec![
        icon,
        Span::styled(label, Style::default().fg(colors.text)),
        Span::styled(
            format!("  {detail}"),
            Style::default()
                .fg(colors.text_dim)
                .add_modifier(Modifier::DIM),
        ),
    ]));

    // Show inline diff for edit_file (old_string → new_string)
    if tool.name == "edit_file"
        && tool.status == ToolStatus::Success
        && let (Some(old), Some(new)) = (
            tool.args.get("old_string").and_then(|v| v.as_str()),
            tool.args.get("new_string").and_then(|v| v.as_str()),
        )
    {
        render_inline_diff(&mut lines, old, new, colors);
    }

    // Show diff preview for apply_patch
    if tool.name == "apply_patch"
        && let Some(diff_text) = tool.args.get("diff").and_then(|v| v.as_str())
    {
        let max_preview_lines = 30;
        let diff_lines: Vec<&str> = diff_text.lines().collect();
        let show_count = diff_lines.len().min(max_preview_lines);

        for line in &diff_lines[..show_count] {
            let style = if line.starts_with('+') && !line.starts_with("+++") {
                Style::default().fg(colors.success)
            } else if line.starts_with('-') && !line.starts_with("---") {
                Style::default().fg(colors.error)
            } else if line.starts_with("@@") {
                Style::default().fg(colors.info)
            } else if line.starts_with("---") || line.starts_with("+++") {
                Style::default()
                    .fg(colors.text)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.text_dim)
            };
            lines.push(Line::from(Span::styled(format!("    {line}"), style)));
        }

        if diff_lines.len() > max_preview_lines {
            lines.push(Line::from(Span::styled(
                format!(
                    "    ... {} more lines",
                    diff_lines.len() - max_preview_lines
                ),
                Style::default().fg(colors.text_dim),
            )));
        }
    }

    if tool.expanded
        && let Some(ref output) = tool.output
    {
        for line in output.lines().take(10) {
            lines.push(Line::from(Span::styled(
                format!("    {line}"),
                Style::default().fg(colors.text_dim),
            )));
        }
    }

    lines.push(Line::from(""));
    lines
}

/// Render an inline diff showing removed lines (red) and added lines (green).
fn render_inline_diff(lines: &mut Vec<Line<'static>>, old: &str, new: &str, colors: &Colors) {
    let max_lines = 20;
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let total = old_lines.len() + new_lines.len();

    let mut count = 0;
    for line in &old_lines {
        if count >= max_lines {
            break;
        }
        lines.push(Line::from(Span::styled(
            format!("    - {line}"),
            Style::default().fg(colors.error),
        )));
        count += 1;
    }
    for line in &new_lines {
        if count >= max_lines {
            break;
        }
        lines.push(Line::from(Span::styled(
            format!("    + {line}"),
            Style::default().fg(colors.success),
        )));
        count += 1;
    }

    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!("    ... {} more lines", total - max_lines),
            Style::default().fg(colors.text_dim),
        )));
    }
}

/// Extract file paths from a unified diff for compact display.
fn extract_patch_files(args: &serde_json::Value) -> String {
    let diff = args.get("diff").and_then(|v| v.as_str()).unwrap_or("");
    let mut paths: Vec<&str> = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let p = rest.trim();
            let p = p.strip_prefix("b/").unwrap_or(p);
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
