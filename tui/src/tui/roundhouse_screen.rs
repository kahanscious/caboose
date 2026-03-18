//! Dedicated Roundhouse screen — model viewer + navigator + gate bar.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::State;
use crate::tui::theme;

/// Render the full Roundhouse screen.
pub fn render(frame: &mut Frame, _state: &State) {
    let colors = theme::Colors::default();
    let area = frame.area();

    let text = Paragraph::new("Roundhouse v2 — screen coming soon")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors.roundhouse)),
        );
    frame.render_widget(text, area);
}
