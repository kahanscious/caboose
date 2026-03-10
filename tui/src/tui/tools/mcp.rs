//! MCP tool renderer — for tools namespaced as `server:tool_name`.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::ToolMessage;
use crate::tui::theme::Colors;

pub struct McpRenderer;

impl ToolRenderer for McpRenderer {
    fn handles(&self) -> &[&str] {
        &[]
    }

    fn matches(&self, tool_name: &str) -> bool {
        tool_name.contains(':')
    }

    fn render(&self, tool: &ToolMessage, colors: &Colors, tick: u64) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let icon = super::status_icon(&tool.status, colors, tick);

        let name_display: String = if tool.name.len() > 50 {
            format!("{}...", &tool.name[..47])
        } else {
            tool.name.clone()
        };

        lines.push(Line::from(vec![
            icon,
            Span::styled("MCP", Style::default().fg(colors.text)),
            Span::styled(
                format!("  {name_display}"),
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
}
