use ratatui::prelude::*;
use ratatui::widgets::*;
use crate::app::State;
use crate::roundhouse::types::RoundhousePhase;

/// Render the Roundhouse provider selection dialog
pub fn render_roundhouse_picker(f: &mut Frame, area: Rect, state: &State) {
    let session = match &state.roundhouse_session {
        Some(s) if s.phase == RoundhousePhase::SelectingProviders => s,
        _ => return,
    };

    let block = Block::default()
        .title(" Roundhouse — Select Secondary Providers ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Primary: {} / {}", session.primary_provider, session.primary_model),
            Style::default().fg(Color::Green),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Available providers (Enter to toggle, Tab to confirm):",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
    ];

    // List configured providers (excluding primary)
    // This will be populated from state.providers registry
    for secondary in &session.secondaries {
        lines.push(Line::from(format!(
            "  [x] {} / {}",
            secondary.provider_name, secondary.model_name
        )));
    }

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}
