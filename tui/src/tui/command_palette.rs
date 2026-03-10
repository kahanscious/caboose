//! Command palette — Ctrl+K fuzzy search over all available commands.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::app::State;
use crate::tui::dialog::CommandPaletteState;
use crate::tui::theme::Colors;

/// Render the command palette overlay.
pub fn render(frame: &mut Frame, palette: &CommandPaletteState, state: &State, colors: &Colors) {
    let area = frame.area();

    // Centered popup: 60 wide, up to 18 tall
    let width = 60.min(area.width.saturating_sub(4));
    let height = 18.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    // Split: search bar (3) + list (rest)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup_area);

    // Search bar
    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.border_active))
        .title(" Commands ")
        .title_style(Style::default().fg(colors.text).bold())
        .style(Style::default().bg(colors.bg_elevated));

    let search_text = if palette.filter.is_empty() {
        Paragraph::new(Span::styled(
            "Type to filter...",
            Style::default().fg(colors.text_muted),
        ))
    } else {
        Paragraph::new(Span::styled(
            format!("> {}", palette.filter),
            Style::default().fg(colors.text),
        ))
    };
    frame.render_widget(search_text.block(search_block), chunks[0]);

    // Build command items
    let mut items: Vec<ListItem> = Vec::new();
    let mut selectable_indices: Vec<usize> = Vec::new();
    let mut item_idx = 0;

    let grouped = state.commands.available_by_category(state);
    for (category, cmds) in &grouped {
        // Filter by palette filter
        let filtered: Vec<_> = cmds
            .iter()
            .filter(|c| {
                if palette.filter.is_empty() {
                    return true;
                }
                let f = palette.filter.to_lowercase();
                c.name.to_lowercase().contains(&f)
                    || c.slash.map(|s| s.contains(&f)).unwrap_or(false)
            })
            .collect();

        if filtered.is_empty() {
            continue;
        }

        // Category header
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {}", category.label()),
            Style::default().fg(colors.text_dim).bold(),
        ))));
        item_idx += 1;

        for cmd in filtered {
            let slash_str = cmd.slash.map(|s| format!("/{s}")).unwrap_or_default();
            let keybind_str = cmd.keybind.map(|kb| kb.display()).unwrap_or_default();

            // Right-aligned keybind + slash
            let name_width = cmd.name.len();
            let right = format!("{slash_str:>10} {keybind_str:>8}");
            let pad = (width as usize)
                .saturating_sub(4) // borders + indent
                .saturating_sub(name_width + 2)
                .saturating_sub(right.len());

            let line = Line::from(vec![
                Span::raw("  "),
                Span::styled(cmd.name.to_string(), Style::default().fg(colors.text)),
                Span::raw(" ".repeat(pad)),
                Span::styled(slash_str, Style::default().fg(colors.text_dim)),
                Span::raw(" "),
                Span::styled(keybind_str, Style::default().fg(colors.text_muted)),
            ]);

            items.push(ListItem::new(line));
            selectable_indices.push(item_idx);
            item_idx += 1;
        }
    }

    // Highlight selected
    let selected_item_idx = selectable_indices.get(palette.selected).copied();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                .border_style(Style::default().fg(colors.border_active))
                .style(Style::default().bg(colors.bg_elevated)),
        )
        .highlight_style(Style::default().bg(colors.bg_hover).fg(colors.text))
        .highlight_symbol("\u{25b8} "); // ▸

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(selected_item_idx);
    frame.render_stateful_widget(list, chunks[1], &mut list_state);
}

/// Count the number of selectable (non-header) commands matching the current filter.
pub fn filtered_count(palette: &CommandPaletteState, state: &State) -> usize {
    let grouped = state.commands.available_by_category(state);
    let mut count = 0;
    for (_category, cmds) in &grouped {
        for cmd in cmds {
            if palette.filter.is_empty()
                || cmd
                    .name
                    .to_lowercase()
                    .contains(&palette.filter.to_lowercase())
                || cmd
                    .slash
                    .map(|s| s.contains(palette.filter.to_lowercase().as_str()))
                    .unwrap_or(false)
            {
                count += 1;
            }
        }
    }
    count
}

/// Given a mouse row, return the selectable index if it falls on a command item.
/// Uses the same centering math as `render` to locate the list area.
pub fn hit_test(
    palette: &CommandPaletteState,
    state: &State,
    mouse_row: u16,
    terminal_height: u16,
    terminal_width: u16,
) -> Option<usize> {
    // Replicate centering math from render
    let _width = 60.min(terminal_width.saturating_sub(4));
    let height = 18.min(terminal_height.saturating_sub(4));
    let y = (terminal_height.saturating_sub(height)) / 2;

    // List area starts after 3-row search bar
    let list_y = y + 3;
    // List block has LEFT/RIGHT/BOTTOM borders — no top border, so items start at list_y
    let list_inner_y = list_y;
    let list_inner_height = (y + height).saturating_sub(list_y).saturating_sub(1); // -1 for bottom border

    if mouse_row < list_inner_y || mouse_row >= list_inner_y + list_inner_height {
        return None;
    }

    let item_row = (mouse_row - list_inner_y) as usize;

    // Walk items to find which selectable index this row maps to
    let grouped = state.commands.available_by_category(state);
    let mut row = 0;
    let mut selectable_idx = 0;
    for (_category, cmds) in &grouped {
        let filtered: Vec<_> = cmds
            .iter()
            .filter(|c| {
                if palette.filter.is_empty() {
                    return true;
                }
                let f = palette.filter.to_lowercase();
                c.name.to_lowercase().contains(&f)
                    || c.slash.map(|s| s.contains(&f)).unwrap_or(false)
            })
            .collect();

        if filtered.is_empty() {
            continue;
        }

        // Category header row
        if row == item_row {
            return None; // Header — not selectable
        }
        row += 1;

        for _ in filtered {
            if row == item_row {
                return Some(selectable_idx);
            }
            row += 1;
            selectable_idx += 1;
        }
    }
    None
}

/// Get the command ID at the selected index, if any.
pub fn selected_command_id(palette: &CommandPaletteState, state: &State) -> Option<&'static str> {
    let grouped = state.commands.available_by_category(state);
    let mut idx = 0;
    for (_category, cmds) in &grouped {
        for cmd in cmds {
            let matches = palette.filter.is_empty()
                || cmd
                    .name
                    .to_lowercase()
                    .contains(&palette.filter.to_lowercase())
                || cmd
                    .slash
                    .map(|s| s.contains(palette.filter.to_lowercase().as_str()))
                    .unwrap_or(false);
            if matches {
                if idx == palette.selected {
                    return Some(cmd.id);
                }
                idx += 1;
            }
        }
    }
    None
}
