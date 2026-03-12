//! Shell command tool renderer.

use ratatui::prelude::*;

use super::ToolRenderer;
use crate::app::{ToolMessage, ToolStatus};
use crate::tui::theme::Colors;

pub struct BashRenderer;

impl ToolRenderer for BashRenderer {
    fn handles(&self) -> &[&str] {
        &["run_command"]
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

    let command = tool
        .args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("?");

    let cmd_display: String = if command.len() > 50 {
        format!("{}...", &command[..47])
    } else {
        command.to_string()
    };

    let exit_hint = if tool.status == ToolStatus::Failed {
        " (error)".to_string()
    } else {
        String::new()
    };

    lines.push(Line::from(vec![
        icon,
        Span::styled("Bash", Style::default().fg(colors.text)),
        Span::styled(
            format!("  {cmd_display}{exit_hint}"),
            Style::default()
                .fg(colors.text_dim)
                .add_modifier(Modifier::DIM),
        ),
    ]));

    if tool.expanded
        && let Some(ref output) = tool.output
    {
        for line in output.lines().take(15) {
            lines.push(Line::from(Span::styled(
                format!("    \u{2502} {line}"),
                Style::default().fg(colors.code_text).bg(colors.code_bg),
            )));
        }
        let total = output.lines().count();
        if total > 15 {
            lines.push(Line::from(Span::styled(
                format!("    ...({} more lines)", total - 15),
                Style::default().fg(colors.text_muted),
            )));
        }
    }

    lines.push(Line::from(""));
    lines
}
