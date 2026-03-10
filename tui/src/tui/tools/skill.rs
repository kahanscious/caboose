//! Skill invocation renderer — shows inline when a skill is activated.

use crate::tui::theme::Colors;
use ratatui::prelude::*;

/// Render a skill invocation inline.
pub fn render(name: &str, description: &str, colors: &Colors) -> Vec<Line<'static>> {
    vec![Line::from(vec![
        Span::styled("\u{2713} ", Style::default().fg(colors.success)),
        Span::styled("Skill  ", Style::default().fg(colors.text_muted).bold()),
        Span::styled(name.to_string(), Style::default().fg(colors.text)),
        Span::styled(
            format!(" — {description}"),
            Style::default().fg(colors.text_muted),
        ),
    ])]
}
