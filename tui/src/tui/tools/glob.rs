//! Glob tool renderer.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::ToolMessage;
use crate::tui::theme::Colors;

pub struct GlobRenderer;

impl ToolRenderer for GlobRenderer {
    fn handles(&self) -> &[&str] {
        &["glob"]
    }

    fn render(
        &self,
        tool: &ToolMessage,
        colors: &Colors,
        tick: u64,
        _diff_expanded: bool,
        _diff_scroll: usize,
    ) -> Vec<Line<'static>> {
        render(tool, colors, tick)
    }
}

pub fn render(tool: &ToolMessage, colors: &Colors, tick: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let icon = super::status_icon(&tool.status, colors, tick);
    let _pattern = tool
        .args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    // Extract filenames (just the basename) for inline display
    let files: Vec<&str> = tool
        .output
        .as_ref()
        .map(|o| o.lines().filter(|l| !l.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    let file_count = files.len();

    // Show first few filenames inline, e.g. "CLAUDE.md, README.md, ... (12 files)"
    let preview: String = if files.is_empty() {
        format!("{file_count} files")
    } else {
        let names: Vec<&str> = files
            .iter()
            .take(3)
            .map(|f| f.rsplit('/').next().unwrap_or(f))
            .collect();
        let joined = names.join(", ");
        if file_count > 3 {
            format!("{joined}, \u{2026} ({file_count} files)")
        } else {
            joined.to_string()
        }
    };

    lines.push(Line::from(vec![
        icon,
        Span::styled("Glob", Style::default().fg(colors.text)),
        Span::styled(
            format!("  {preview}"),
            Style::default()
                .fg(colors.text_dim)
                .add_modifier(Modifier::DIM),
        ),
    ]));

    if tool.expanded
        && let Some(ref output) = tool.output
    {
        for line in output.lines().filter(|l| !l.is_empty()).take(15) {
            lines.push(Line::from(Span::styled(
                format!("    {line}"),
                Style::default().fg(colors.text_dim),
            )));
        }
        let total = output.lines().filter(|l| !l.is_empty()).count();
        if total > 15 {
            lines.push(Line::from(Span::styled(
                format!("    ...({} more)", total - 15),
                Style::default().fg(colors.text_muted),
            )));
        }
    }

    lines.push(Line::from(""));
    lines
}
