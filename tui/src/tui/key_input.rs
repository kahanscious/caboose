//! API key input modal — text input for entering a provider API key.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme;

/// State for the API key input modal.
pub struct KeyInputState {
    /// The provider ID we're entering a key for
    pub provider_id: String,
    /// Current input text
    pub input: String,
    /// Whether a key already exists for this provider
    pub has_existing: bool,
}

impl KeyInputState {
    pub fn new(provider_id: String, has_existing: bool) -> Self {
        Self {
            provider_id,
            input: String::new(),
            has_existing,
        }
    }
}

/// Render the API key input as a centered overlay.
pub fn render(frame: &mut Frame, state: &KeyInputState) {
    let colors = theme::Colors::default();
    let area = frame.area();

    // Center a popup: 60% width, fixed height
    let popup_width = (area.width * 60 / 100).max(40).min(area.width - 4);
    let popup_height = 5u16.min(area.height - 2);
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Hint (directly below input)
            Constraint::Min(0),
        ])
        .split(popup_area);

    // Title block with input
    let title = format!(" API key \u{2014} {} ", state.provider_id);
    let input_block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(Alignment::Left)
        .border_style(Style::default().fg(colors.border_active))
        .style(Style::default().bg(colors.bg_elevated));

    let input_text = if state.input.is_empty() {
        let placeholder = if state.has_existing {
            "paste new key or enter to clear"
        } else {
            "API key"
        };
        Span::styled(placeholder, Style::default().fg(colors.text_muted))
    } else {
        Span::styled(
            "\u{2022}".repeat(state.input.len()),
            Style::default().fg(colors.text),
        )
    };
    let input = Paragraph::new(Line::from(input_text)).block(input_block);
    frame.render_widget(input, chunks[0]);

    // "esc" hint in title bar
    let esc_hint = Span::styled("esc", Style::default().fg(colors.text_dim));
    let esc_area = Rect {
        x: chunks[0].x + chunks[0].width.saturating_sub(5),
        y: chunks[0].y,
        width: 4,
        height: 1,
    };
    frame.render_widget(Paragraph::new(Line::from(esc_hint)), esc_area);

    // Hint line
    let hint = if state.has_existing {
        Line::from(vec![
            Span::styled("  enter ", Style::default().fg(colors.text).bold()),
            Span::styled("clear key  ", Style::default().fg(colors.text_dim)),
            Span::styled("esc ", Style::default().fg(colors.text).bold()),
            Span::styled("cancel", Style::default().fg(colors.text_dim)),
        ])
    } else {
        Line::from(vec![
            Span::styled("  enter ", Style::default().fg(colors.text).bold()),
            Span::styled("submit", Style::default().fg(colors.text_dim)),
        ])
    };
    let hint_bg = Block::default().style(Style::default().bg(colors.bg_elevated));
    frame.render_widget(Paragraph::new(hint).block(hint_bg), chunks[1]);
}
