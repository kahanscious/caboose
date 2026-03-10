//! Fallback renderer for unknown tool types.

use ratatui::prelude::*;

use crate::app::ToolMessage;
use crate::tui::theme::Colors;

pub fn render(tool: &ToolMessage, colors: &Colors, tick: u64) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let icon = super::status_icon(&tool.status, colors, tick);

    lines.push(Line::from(vec![
        icon,
        Span::styled(tool.name.clone(), Style::default().fg(colors.text)),
    ]));

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
