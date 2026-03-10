//! Home screen — centered branding, logo, and input field.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::app::State;
use crate::tui::theme;

/// Pixel art caboose icon — displayed above the text logo.
const ICON: &[&str] = &[
    "       ▄████████▄       ",
    "       █        █       ",
    "▄▄████████████████████▄▄",
    "  █    █        █    █  ",
    "  ████████████████████  ",
    "  ▀ ▄██▄        ▄██▄ ▀  ",
    "    ▀██▀        ▀██▀    ",
];

/// ASCII art logo for "CABOOSE" in block characters.
/// Each glyph is 4 chars wide with 1-space gaps.
///  C    A    B    O    O    S    E
const LOGO: &[&str] = &[
    r"▄▀▀▀ ▄▀▀▄ █▀▀▄ ▄▀▀▄ ▄▀▀▄ ▄▀▀▀ █▀▀▀",
    r"█    █▀▀█ █▀▀▄ █  █ █  █ ▀▀▀█ █▀▀ ",
    r" ▀▀▀ ▀  ▀ ▀▀▀   ▀▀   ▀▀  ▀▀▀  ▀▀▀▀",
];

/// Tips shown on the home screen — one is randomly selected per session.
pub const TIPS: &[&str] = &[
    "Ctrl+K  open the command palette",
    "Ctrl+N  start a new session",
    "/plan  create an implementation plan",
    "/debug  systematic debugging workflow",
    "/brainstorm  explore ideas before building",
    "/tdd  test-driven development workflow",
    "/review  code review a pull request",
    "/handoff  summarize session for handoff",
    "@file  reference files in your prompt",
    "/terminal  toggle the terminal panel",
    "/connect  add an API key",
    "Tab (empty input)  cycle permission modes",
    "/mcp  manage MCP server connections",
    "/init  generate a CABOOSE.md project file",
    "Pipe input:  echo 'explain this' | caboose",
    "--prompt  run non-interactively from the CLI",
    "/create-skill  make a custom slash command",
    "Ctrl+C twice  quit caboose",
];

/// Render the centered home screen.
pub fn render(frame: &mut Frame, state: &State) {
    let colors = theme::Colors::default();
    let area = frame.area();

    let terminal_visible = state
        .terminal_panel
        .as_ref()
        .map(|p| p.visible)
        .unwrap_or(false);
    let terminal_height = if terminal_visible {
        (area.height * 25 / 100).max(6)
    } else {
        0
    };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if terminal_visible {
            vec![
                Constraint::Min(1),                  // Main content area
                Constraint::Length(4),               // Footer
                Constraint::Length(terminal_height), // Terminal panel (bottommost)
            ]
        } else {
            vec![
                Constraint::Min(1),    // Main content area
                Constraint::Length(4), // Footer
            ]
        })
        .split(area);

    // Show icon only if terminal is tall enough
    let show_icon = outer[0].height >= 19;

    // Build the centered content lines (icon + logo only)
    let mut lines: Vec<Line> = Vec::new();

    // Icon (pixel art caboose)
    if show_icon {
        let brand = Style::default().fg(colors.brand).bold();
        for icon_line in ICON.iter() {
            lines.push(Line::from(Span::styled(*icon_line, brand)).alignment(Alignment::Center));
        }
        // Blank line between icon and text
        lines.push(Line::from(""));
    }

    // Logo lines
    for logo_line in LOGO {
        lines.push(
            Line::from(Span::styled(
                *logo_line,
                Style::default().fg(colors.brand).bold(),
            ))
            .alignment(Alignment::Center),
        );
    }

    // Blank line after logo
    lines.push(Line::from(""));

    // Content height: lines above + dynamic input field height
    let lines_above_input = if show_icon {
        ICON.len() + 1 + LOGO.len() + 1
    } else {
        LOGO.len() + 1
    };
    let input_width = (area.width * 7 / 10)
        .max(40)
        .min(area.width.saturating_sub(4));
    let home_text_width = (input_width as usize).saturating_sub(3).max(1);
    let extra_input_lines = state
        .input
        .visual_line_count(home_text_width as u16)
        .saturating_sub(1)
        .min(4) as u16;
    let input_field_height = 4 + extra_input_lines;
    let content_height = lines_above_input as u16 + input_field_height;
    let vertical_pad = outer[0].height.saturating_sub(content_height) / 2;

    let centered_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(vertical_pad),
            Constraint::Length(content_height),
            Constraint::Min(0),
        ])
        .split(outer[0]);

    // Render the logo/icon lines
    let logo_content = Paragraph::new(lines);
    frame.render_widget(logo_content, centered_area[1]);

    // --- Input field (dynamic rows, centered, 70% terminal width) ---
    let input_x = area.x + (area.width.saturating_sub(input_width)) / 2;
    let input_y = centered_area[1].y + lines_above_input as u16;
    let input_area = Rect::new(input_x, input_y, input_width, input_field_height);

    let quit_confirm = state.quit_first_press.is_some();
    let (accent_color, info_left) = crate::tui::input::build_info_left(
        None,
        quit_confirm,
        state.mode,
        &state.active_model_name,
        &state.active_provider_name,
        &colors,
    );
    let info_right = crate::tui::input::build_info_right(&colors);

    crate::tui::input::render_input_field(
        frame,
        input_area,
        &state.input,
        accent_color,
        info_left,
        info_right,
        &colors,
    );

    // --- Tip (centered between input bottom and footer top) ---
    let input_bottom = input_y + input_field_height;
    let footer_top = outer[1].y;
    if footer_top > input_bottom + 1 {
        let tip_y = input_bottom + (footer_top - input_bottom) / 2;
        let tip_text = TIPS[state.home_tip_index % TIPS.len()];
        let tip_line = Line::from(Span::styled(
            format!("tip: {tip_text}"),
            Style::default().fg(colors.text_dim),
        ))
        .alignment(Alignment::Center);
        let tip_area = Rect::new(area.x, tip_y, area.width, 1);
        frame.render_widget(Paragraph::new(tip_line), tip_area);
    }

    // Footer renders BEFORE dropdowns so dropdowns paint on top when overlapping
    let budget = state
        .config
        .behavior
        .as_ref()
        .and_then(|b| b.max_session_cost)
        .map(|max| crate::tui::footer::BudgetInfo {
            session_cost: state.session_cost,
            max_cost: max,
        });
    let is_active = state.init_rx.is_some();
    crate::tui::footer::render(
        frame,
        outer[1],
        state.mode,
        state.caboose_pos,
        is_active,
        budget,
        state.update_available.as_deref(),
    );

    // Terminal panel (bottommost, below footer)
    if terminal_visible && let Some(panel) = &state.terminal_panel {
        let terminal_area = outer[2];
        state.terminal_area.set(Some(terminal_area));
        let widget = crate::terminal::widget::TerminalWidget {
            panel,
            focused: state.terminal_focused,
            colors: &colors,
        };
        frame.render_widget(widget, terminal_area);
    }

    // Slash autocomplete dropdown (renders below input on home screen)
    if let Some(auto) = &state.slash_auto {
        let anchor = Rect::new(input_x, input_y + input_field_height, input_width, 1);
        let input_text = state.input.content();
        crate::tui::slash_auto::render_slash_autocomplete(
            frame,
            anchor,
            auto,
            &input_text,
            &state.commands,
            &state.skills,
            &colors,
            false,
            state.current_session_id.as_deref(),
            &state.discovered_locals,
        );
    }

    // File autocomplete dropdown (renders below input, attached to input border)
    if let Some(ref auto) = state.file_auto {
        let visible = auto.matches.len().min(8);
        if visible > 0 {
            let dropdown_height = visible as u16 + 2; // +2 for border
            // Position so top border overlaps input bottom border (connected look)
            let dropdown_area = Rect {
                x: input_x,
                y: input_y + input_field_height - 1,
                width: input_width.min(60),
                height: dropdown_height,
            };

            let items: Vec<Line> = auto
                .matches
                .iter()
                .enumerate()
                .take(visible)
                .map(|(i, path)| {
                    let style = if i == auto.selected {
                        Style::default().fg(colors.text).bg(colors.bg_hover)
                    } else {
                        Style::default()
                            .fg(colors.text_secondary)
                            .bg(colors.bg_elevated)
                    };
                    Line::from(Span::styled(format!(" {path} "), style))
                })
                .collect();

            let block = ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(Style::default().fg(colors.border_active))
                .title(" files ")
                .title_style(Style::default().fg(colors.text_dim))
                .style(Style::default().bg(colors.bg_elevated));
            let paragraph = Paragraph::new(items).block(block);
            frame.render_widget(ratatui::widgets::Clear, dropdown_area);
            frame.render_widget(paragraph, dropdown_area);
        }
    }
}
