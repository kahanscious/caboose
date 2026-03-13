//! Read file / list directory tool renderer.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::ToolMessage;
use crate::tui::theme::Colors;

pub struct ReadRenderer;

impl ToolRenderer for ReadRenderer {
    fn handles(&self) -> &[&str] {
        &["read_file", "list_directory"]
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
    let path = tool.file_path.clone().unwrap_or_else(|| "?".to_string());

    let line_count = tool.output.as_ref().map(|o| o.lines().count()).unwrap_or(0);

    let label = if tool.name == "list_directory" {
        "List"
    } else {
        "Read"
    };

    lines.push(Line::from(vec![
        icon,
        Span::styled(label, Style::default().fg(colors.text)),
        Span::styled(
            format!("  {path} ({line_count} lines)"),
            Style::default()
                .fg(colors.text_dim)
                .add_modifier(Modifier::DIM),
        ),
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
        let total = output.lines().count();
        if total > 10 {
            lines.push(Line::from(Span::styled(
                format!("    ...({} more lines)", total - 10),
                Style::default().fg(colors.text_muted),
            )));
        }
    }

    lines.push(Line::from(""));
    lines
}
