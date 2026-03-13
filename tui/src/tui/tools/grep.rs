//! Grep tool renderer.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::ToolMessage;
use crate::tui::theme::Colors;

pub struct GrepRenderer;

impl ToolRenderer for GrepRenderer {
    fn handles(&self) -> &[&str] {
        &["grep"]
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
    let pattern = tool
        .args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    let match_count = tool
        .output
        .as_ref()
        .map(|o| o.lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0);

    lines.push(Line::from(vec![
        icon,
        Span::styled("Search", Style::default().fg(colors.text)),
        Span::styled(
            format!("  \"{pattern}\" \u{2192} {match_count} matches"),
            Style::default()
                .fg(colors.text_dim)
                .add_modifier(Modifier::DIM),
        ),
    ]));

    if tool.expanded
        && let Some(ref output) = tool.output
    {
        for line in output.lines().filter(|l| !l.is_empty()).take(10) {
            lines.push(Line::from(Span::styled(
                format!("    {line}"),
                Style::default().fg(colors.text_dim),
            )));
        }
        let total = output.lines().filter(|l| !l.is_empty()).count();
        if total > 10 {
            lines.push(Line::from(Span::styled(
                format!("    ...({} more)", total - 10),
                Style::default().fg(colors.text_muted),
            )));
        }
    }

    lines.push(Line::from(""));
    lines
}
