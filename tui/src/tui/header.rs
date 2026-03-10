//! Session header bar — colored separator strip.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::tui::theme::Colors;

/// Render the session header bar (empty colored strip).
pub fn render(frame: &mut Frame, area: Rect) {
    let colors = Colors::default();
    let header = Paragraph::new("").style(Style::default().bg(colors.bg_elevated));
    frame.render_widget(header, area);
}
