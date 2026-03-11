//! WorkspaceList dialog renderer.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    prelude::Stylize,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use crate::tui::dialog::WorkspaceListState;
use crate::tui::theme::Colors;

pub fn render(f: &mut Frame, area: Rect, state: &WorkspaceListState) {
    let colors = Colors::active();
    let popup = centered_rect(50, 14, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" workspaces ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.border_active))
        .title_style(Style::default().fg(colors.text).bold())
        .style(Style::default().bg(colors.bg_elevated));

    f.render_widget(block, popup);

    let inner = Rect {
        x: popup.x + 1,
        y: popup.y + 1,
        width: popup.width.saturating_sub(2),
        height: popup.height.saturating_sub(2),
    };

    if state.workspaces.is_empty() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // spacer
                Constraint::Length(1), // "no workspaces"
                Constraint::Length(1), // "press a to add"
                Constraint::Min(0),    // spacer
                Constraint::Length(1), // hints
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new("  no workspaces configured")
                .style(Style::default().fg(colors.text_secondary)),
            chunks[1],
        );
        f.render_widget(
            Paragraph::new("  press a to add one")
                .style(Style::default().fg(colors.text_dim)),
            chunks[2],
        );
        f.render_widget(
            Paragraph::new(" a add · esc close")
                .style(Style::default().fg(colors.text_dim)),
            chunks[4],
        );
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // list
                Constraint::Length(1), // hints
            ])
            .split(inner);

        let items: Vec<ListItem> = state
            .workspaces
            .iter()
            .map(|(name, cfg, available)| {
                let avail_style = if *available {
                    Style::default().fg(colors.success)
                } else {
                    Style::default().fg(colors.error)
                };
                let avail_marker = if *available { "●" } else { "✗" };
                let mode_str = match cfg.mode {
                    crate::config::schema::WorkspaceMode::Proactive => "proactive",
                    crate::config::schema::WorkspaceMode::Explicit => "explicit ",
                };
                let display_path = truncate_path(&cfg.path, 12);
                let line = Line::from(vec![
                    Span::raw(format!("  {name:<16} ")),
                    Span::styled(format!("{mode_str} "), Style::default().fg(colors.text_dim)),
                    Span::styled(format!("{avail_marker} "), avail_style),
                    Span::styled(display_path, Style::default().fg(colors.text_secondary)),
                ]);
                ListItem::new(line)
            })
            .collect();

        let mut list_state = ListState::default();
        list_state.select(Some(state.selected));

        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .fg(colors.brand)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, chunks[0], &mut list_state);

        f.render_widget(
            Paragraph::new(" a add · d remove · esc close")
                .style(Style::default().fg(colors.text_dim)),
            chunks[1],
        );
    }
}

fn truncate_path(path: &str, max_len: usize) -> String {
    let char_count = path.chars().count();
    if char_count <= max_len {
        return path.to_string();
    }
    let skip = char_count - max_len;
    let byte_offset = path.char_indices().nth(skip).map(|(i, _)| i).unwrap_or(0);
    format!("…{}", &path[byte_offset..])
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
    use crate::config::schema::{WorkspaceConfig, WorkspaceMode};

    #[test]
    fn render_does_not_panic_with_empty_list() {
        let state = WorkspaceListState {
            workspaces: vec![],
            selected: 0,
        };
        assert_eq!(state.workspaces.len(), 0);
    }

    #[test]
    fn render_does_not_panic_with_entries() {
        let state = WorkspaceListState {
            workspaces: vec![
                ("caboose-web".to_string(), WorkspaceConfig {
                    path: "/home/alex/caboose-web".to_string(),
                    mode: WorkspaceMode::Proactive,
                }, true),
                ("caboose-docs".to_string(), WorkspaceConfig {
                    path: "/home/alex/caboose-docs".to_string(),
                    mode: WorkspaceMode::Explicit,
                }, false),
            ],
            selected: 0,
        };
        assert_eq!(state.workspaces.len(), 2);
    }

    #[test]
    fn truncate_path_short_path_unchanged() {
        assert_eq!(truncate_path("/tmp/x", 12), "/tmp/x");
    }

    #[test]
    fn truncate_path_long_path_truncated() {
        let result = truncate_path("/home/alex/very/long/path/here", 12);
        assert!(result.starts_with('…'));
        let char_count = result.chars().count();
        assert!(char_count <= 13);
    }
}
