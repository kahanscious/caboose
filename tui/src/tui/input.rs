//! Input handling — keybinding definitions and input mode management.

use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::style::Color;
use ratatui::widgets::Paragraph;
use std::time::SystemTime;

use crate::tui::theme;

/// Whether the cursor should be visible right now (500ms on, 500ms off).
fn cursor_visible() -> bool {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    (millis / 500).is_multiple_of(2)
}

/// Actions the user can trigger via keyboard input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Action {
    /// Submit the current input
    Submit,
    /// Insert a character
    InsertChar(char),
    /// Delete character before cursor
    Backspace,
    /// Quit the application
    Quit,
    /// Scroll chat up
    ScrollUp,
    /// Scroll chat down
    ScrollDown,
    /// Open session picker
    SessionPicker,
    /// Approve a tool call
    ApproveTool,
    /// Deny a tool call
    DenyTool,
    /// No-op
    None,
}

/// Map a key code to an action in chat mode.
#[allow(dead_code)]
pub fn map_chat_key(key: KeyCode) -> Action {
    match key {
        KeyCode::Enter => Action::Submit,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Up => Action::ScrollUp,
        KeyCode::Down => Action::ScrollDown,
        KeyCode::Char(c) => Action::InsertChar(c),
        _ => Action::None,
    }
}

/// Placeholder text shown when input is empty.
const PLACEHOLDER: &str = "Chugga chugga choo choo";

/// Left accent character (left half block).
const ACCENT_CHAR: &str = "\u{258c}";

/// Cursor block character.
const CURSOR_BLOCK: &str = "\u{2588}";

/// Render the input field with multi-line support.
///
/// Layout: top padding, input lines (1+), bottom padding, info line.
/// Single-line inputs render as before. Multi-line inputs show each line,
/// the first with an accent bar and subsequent lines with a continuation bar.
pub fn render_input_field(
    frame: &mut ratatui::Frame,
    area: Rect,
    input: &crate::tui::input_buffer::InputBuffer,
    accent_color: Color,
    info_left: Vec<Span>,
    info_right: Vec<Span>,
    colors: &theme::Colors,
) {
    if area.height < 4 || area.width < 4 {
        return;
    }

    // Usable text width after accent bar + space prefix + cursor block
    let text_width = (area.width as usize).saturating_sub(3).max(1);

    let visual_lines = input.visual_line_count(text_width as u16);
    // How many rows for input lines (at least 1, cap to available space minus 2 for padding/info)
    let max_input_rows = (area.height as usize).saturating_sub(3).max(1);
    let visible_rows = visual_lines.min(max_input_rows);

    // Visual cursor position for scroll tracking
    let (vcursor_row, vcursor_col) = input.visual_cursor(text_width as u16);

    // Scroll offset: keep visual cursor visible within the viewport
    let scroll_offset = if vcursor_row >= max_input_rows {
        vcursor_row - max_input_rows + 1
    } else {
        0
    };

    // Build all visual rows: (logical_line_idx, wrap_idx, text_chunk)
    let mut visual_rows: Vec<(usize, usize, String)> = Vec::new();
    for (line_idx, line_text) in input.lines().enumerate() {
        if line_text.is_empty() {
            visual_rows.push((line_idx, 0, String::new()));
        } else {
            let offsets = crate::tui::input_buffer::word_wrap_offsets(line_text, text_width);
            for (wrap_idx, &start) in offsets.iter().enumerate() {
                let end = offsets
                    .get(wrap_idx + 1)
                    .copied()
                    .unwrap_or(line_text.len());
                visual_rows.push((line_idx, wrap_idx, line_text[start..end].to_string()));
            }
        }
    }

    // --- Row 0: top padding ---
    let row0 = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(colors.bg_secondary)),
        row0,
    );

    // --- Input rows ---
    for vrow_idx in 0..visible_rows {
        let src_idx = vrow_idx + scroll_offset;
        if src_idx >= visual_rows.len() {
            break;
        }
        let y = area.y + 1 + vrow_idx as u16;
        if y >= area.y + area.height.saturating_sub(1) {
            break;
        }
        let row_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };

        // Fill background
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(colors.bg_secondary)),
            row_area,
        );

        let (line_idx, wrap_idx, ref chunk) = visual_rows[src_idx];
        let is_first_visual_row = src_idx == 0;
        let is_cursor_vrow = src_idx == vcursor_row;

        let bar = if is_first_visual_row {
            Span::styled(
                ACCENT_CHAR,
                Style::default().fg(accent_color).bg(colors.bg_secondary),
            )
        } else if wrap_idx == 0 {
            // First wrap of a new logical line (not the first line)
            Span::styled(
                "\u{2502}",
                Style::default().fg(colors.border).bg(colors.bg_secondary),
            )
        } else {
            // Continuation wrap of the same logical line
            Span::styled(" ", Style::default().bg(colors.bg_secondary))
        };
        let space = Span::styled(" ", Style::default().bg(colors.bg_secondary));

        let text_spans = if input.is_empty() && line_idx == 0 {
            // Placeholder with blinking cursor
            let cursor_span = if cursor_visible() {
                Span::styled(
                    CURSOR_BLOCK,
                    Style::default().fg(accent_color).bg(colors.bg_secondary),
                )
            } else {
                Span::styled(
                    " ",
                    Style::default()
                        .fg(colors.bg_secondary)
                        .bg(colors.bg_secondary),
                )
            };
            vec![
                bar,
                space,
                cursor_span,
                Span::styled(
                    PLACEHOLDER,
                    Style::default().fg(colors.text_dim).bg(colors.bg_secondary),
                ),
            ]
        } else if is_cursor_vrow {
            // Split chunk at visual cursor column for cursor rendering
            let (before, after) = if vcursor_col <= chunk.len() {
                (&chunk[..vcursor_col], &chunk[vcursor_col..])
            } else {
                (chunk.as_str(), "")
            };
            let cursor_span = if cursor_visible() {
                Span::styled(
                    CURSOR_BLOCK,
                    Style::default().fg(accent_color).bg(colors.bg_secondary),
                )
            } else {
                Span::styled(
                    " ",
                    Style::default()
                        .fg(colors.bg_secondary)
                        .bg(colors.bg_secondary),
                )
            };
            let mut spans = vec![
                bar,
                space,
                Span::styled(
                    before.to_string(),
                    Style::default().fg(colors.text).bg(colors.bg_secondary),
                ),
                cursor_span,
            ];
            if !after.is_empty() {
                spans.push(Span::styled(
                    after.to_string(),
                    Style::default().fg(colors.text).bg(colors.bg_secondary),
                ));
            }
            spans
        } else {
            vec![
                bar,
                space,
                Span::styled(
                    chunk.to_string(),
                    Style::default().fg(colors.text).bg(colors.bg_secondary),
                ),
            ]
        };

        let line_widget = Paragraph::new(Line::from(text_spans));
        frame.render_widget(line_widget, row_area);
    }

    // --- Padding row (between input and info line) ---
    let pad_y = area.y + 1 + visible_rows as u16;
    if pad_y < area.y + area.height.saturating_sub(1) {
        let pad_area = Rect {
            x: area.x,
            y: pad_y,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(colors.bg_secondary)),
            pad_area,
        );
    }

    // --- Info line (always last row) ---
    let info_y = area.y + area.height - 1;
    let row3 = Rect {
        x: area.x,
        y: info_y,
        width: area.width,
        height: 1,
    };

    let left_width: usize = info_left.iter().map(|s| s.width()).sum();
    let right_width: usize = info_right.iter().map(|s| s.width()).sum();
    let padding = (area.width as usize).saturating_sub(left_width + right_width + 2);

    let mut info_spans = vec![Span::raw("  ")];
    info_spans.extend(info_left);
    info_spans.push(Span::raw(" ".repeat(padding)));
    info_spans.extend(info_right);

    let info_line = Paragraph::new(Line::from(info_spans))
        .style(Style::default().bg(colors.bg_primary).fg(colors.text_dim));
    frame.render_widget(info_line, row3);
}

/// Build the info-line left spans.
///
/// Returns `(accent_color, left_spans)`.
pub fn build_info_left<'a>(
    agent_state_label: Option<&str>,
    quit_confirm: bool,
    mode: crate::agent::permission::Mode,
    model: &str,
    provider: &str,
    thinking_mode: crate::provider::ThinkingMode,
    model_supports_thinking: bool,
    colors: &theme::Colors,
) -> (Color, Vec<Span<'a>>) {
    if quit_confirm {
        return (
            colors.error,
            vec![Span::styled(
                "press ctrl+c again to quit".to_string(),
                Style::default().fg(colors.warning),
            )],
        );
    }

    if let Some(label) = agent_state_label {
        return (
            colors.roundhouse,
            vec![Span::styled(
                label.to_string(),
                Style::default().fg(colors.roundhouse),
            )],
        );
    }

    let mode_color = match mode {
        crate::agent::permission::Mode::Plan => colors.info,
        crate::agent::permission::Mode::Create => colors.brand,
        crate::agent::permission::Mode::Chug => colors.warning,
    };

    let mut spans = vec![
        Span::styled(
            mode.label().to_string(),
            Style::default().fg(mode_color).bold(),
        ),
        Span::styled("  ".to_string(), Style::default().fg(colors.text_dim)),
        Span::styled(model.to_string(), Style::default().fg(colors.text_dim)),
        Span::styled(
            " \u{00b7} ".to_string(),
            Style::default().fg(colors.text_muted),
        ),
        Span::styled(provider.to_string(), Style::default().fg(colors.text_dim)),
    ];

    // Show thinking indicator when enabled
    if model_supports_thinking && thinking_mode.is_on() {
        spans.push(Span::styled(
            " \u{00b7} ".to_string(),
            Style::default().fg(colors.text_muted),
        ));
        spans.push(Span::styled(
            "thinking".to_string(),
            Style::default().fg(colors.info),
        ));
    }

    (mode_color, spans)
}

/// Build the info-line right spans (keybind hints).
pub fn build_info_right(
    model_supports_thinking: bool,
    colors: &theme::Colors,
) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled("tab ", Style::default().fg(colors.text_muted)),
        Span::styled("mode", Style::default().fg(colors.text_dim)),
    ];

    if model_supports_thinking {
        spans.push(Span::styled("  ", Style::default().fg(colors.text_muted)));
        spans.push(Span::styled("ctrl+t ", Style::default().fg(colors.text_muted)));
        spans.push(Span::styled("thinking", Style::default().fg(colors.text_dim)));
    }

    spans.push(Span::styled("  ", Style::default().fg(colors.text_muted)));
    spans.push(Span::styled("ctrl+k ", Style::default().fg(colors.text_muted)));
    spans.push(Span::styled("commands", Style::default().fg(colors.text_dim)));

    spans
}
