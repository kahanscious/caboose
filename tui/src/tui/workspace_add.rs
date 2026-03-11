//! WorkspaceAdd dialog renderer — 3-phase add flow: Path → Name → Mode.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use crate::tui::dialog::{WorkspaceAddPhase, WorkspaceAddState};

pub fn render(f: &mut Frame, area: Rect, state: &WorkspaceAddState) {
    let popup = centered_rect(50, 14, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" add workspace ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(block, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    match state.phase {
        WorkspaceAddPhase::Path => render_path_phase(f, inner, state),
        WorkspaceAddPhase::Name => render_name_phase(f, inner, state),
        WorkspaceAddPhase::Mode => render_mode_phase(f, inner, state),
    }
}

fn render_path_phase(f: &mut Frame, area: Rect, state: &WorkspaceAddState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(1), // input
            Constraint::Length(1), // spacer
            Constraint::Min(1),    // suggestions list
            Constraint::Length(1), // error or hints
        ])
        .split(area);

    f.render_widget(
        Paragraph::new("path").style(Style::default().fg(Color::DarkGray)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!("> {}█", state.path_input)),
        chunks[1],
    );

    if !state.path_matches.is_empty() {
        let items: Vec<ListItem> = state
            .path_matches
            .iter()
            .map(|p| ListItem::new(Line::from(format!("  {p}/"))))
            .collect();
        let mut list_state = ListState::default();
        list_state.select(if state.path_matches.is_empty() { None } else { Some(state.path_selected) });
        let list = List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol("▸ ");
        f.render_stateful_widget(list, chunks[3], &mut list_state);
    }

    let hint_or_error = if let Some(ref err) = state.error {
        Paragraph::new(Span::styled(err.as_str(), Style::default().fg(Color::Red)))
    } else {
        Paragraph::new(" type to filter · ↑↓ select · ↵ confirm")
            .style(Style::default().fg(Color::DarkGray))
    };
    f.render_widget(hint_or_error, chunks[4]);
}

fn render_name_phase(f: &mut Frame, area: Rect, state: &WorkspaceAddState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(1), // input
            Constraint::Min(0),    // spacer
            Constraint::Length(1), // error or hints
        ])
        .split(area);

    f.render_widget(
        Paragraph::new("name").style(Style::default().fg(Color::DarkGray)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(format!("> {}█", state.name_input)),
        chunks[1],
    );

    let hint_or_error = if let Some(ref err) = state.error {
        Paragraph::new(Span::styled(err.as_str(), Style::default().fg(Color::Red)))
    } else {
        Paragraph::new(" ↵ confirm · esc back")
            .style(Style::default().fg(Color::DarkGray))
    };
    f.render_widget(hint_or_error, chunks[3]);
}

fn render_mode_phase(f: &mut Frame, area: Rect, state: &WorkspaceAddState) {
    let options = [
        ("proactive", "agent searches this repo automatically"),
        ("explicit", "only when you reference it directly"),
    ];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(1), // spacer
            Constraint::Min(1),    // list
            Constraint::Length(1), // hints
        ])
        .split(area);

    f.render_widget(
        Paragraph::new("search mode").style(Style::default().fg(Color::DarkGray)),
        chunks[0],
    );

    let items: Vec<ListItem> = options
        .iter()
        .map(|(name, desc)| ListItem::new(Line::from(vec![
            Span::raw(format!("  {name:<12} ")),
            Span::styled(*desc, Style::default().fg(Color::DarkGray)),
        ])))
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.mode_selected));

    let list = List::new(items)
        .highlight_style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, chunks[2], &mut list_state);

    f.render_widget(
        Paragraph::new(" ↑↓ select · ↵ confirm · esc back")
            .style(Style::default().fg(Color::DarkGray)),
        chunks[3],
    );
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let width = (area.width * percent_x / 100).min(area.width);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height: height.min(area.height) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_path_phase_stub() {
        let state = WorkspaceAddState::default();
        assert!(matches!(state.phase, WorkspaceAddPhase::Path));
    }

    #[test]
    fn render_name_phase_stub() {
        let mut state = WorkspaceAddState::default();
        state.phase = WorkspaceAddPhase::Name;
        state.name_input = "caboose-web".to_string();
        assert_eq!(state.name_input, "caboose-web");
    }

    #[test]
    fn render_mode_phase_stub() {
        let mut state = WorkspaceAddState::default();
        state.phase = WorkspaceAddPhase::Mode;
        state.mode_selected = 1;
        assert_eq!(state.mode_selected, 1);
    }
}
