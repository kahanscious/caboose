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

    // Show line range (e.g. "lines 165–364") when offset/limit are present,
    // otherwise just show total line count.
    let range_info = if tool.name == "read_file" {
        let offset = tool.args.get("offset").and_then(|v| v.as_u64());
        let limit = tool.args.get("limit").and_then(|v| v.as_u64());
        match (offset, limit) {
            (Some(off), Some(lim)) => {
                let start = off + 1; // offset is 0-based, display as 1-based
                let end = off + lim.min(line_count as u64);
                format!("lines {start}–{end}")
            }
            (Some(off), None) => {
                let start = off + 1;
                let end = off + line_count as u64;
                format!("lines {start}–{end}")
            }
            (None, Some(lim)) => {
                let end = lim.min(line_count as u64);
                format!("lines 1–{end}")
            }
            (None, None) => format!("{line_count} lines"),
        }
    } else {
        format!("{line_count} lines")
    };

    lines.push(Line::from(vec![
        icon,
        Span::styled(label, Style::default().fg(colors.text)),
        Span::styled(
            format!("  {path} ({range_info})"),
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
