//! Fetch URL tool renderer.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::ToolMessage;
use crate::tui::theme::Colors;

pub struct FetchRenderer;

impl ToolRenderer for FetchRenderer {
    fn handles(&self) -> &[&str] {
        &["fetch"]
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
    let url = tool.args.get("url").and_then(|v| v.as_str()).unwrap_or("?");

    let url_display: String = if url.len() > 50 {
        format!("{}...", &url[..47])
    } else {
        url.to_string()
    };

    lines.push(Line::from(vec![
        icon,
        Span::styled("Fetch", Style::default().fg(colors.text)),
        Span::styled(
            format!("  {url_display}"),
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
    }

    lines.push(Line::from(""));
    lines
}
