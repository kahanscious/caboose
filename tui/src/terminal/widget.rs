//! Ratatui widget that renders a vt100 screen into a Buffer.

use ratatui::prelude::*;
use ratatui::widgets::Widget;

use super::colors;
use super::panel::TerminalPanel;

/// Read the current git branch by walking up from CWD to find `.git/HEAD`.
/// Returns `None` if not in a git repo or on a detached HEAD.
fn git_branch() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let head_path = dir.join(".git/HEAD");
        if let Ok(head) = std::fs::read_to_string(&head_path) {
            let trimmed = head.trim();
            return trimmed
                .strip_prefix("ref: refs/heads/")
                .map(|b| b.to_string());
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Renders the terminal panel: a 1-row header + the vt100 screen grid.
pub struct TerminalWidget<'a> {
    pub panel: &'a TerminalPanel,
    pub focused: bool,
    pub colors: &'a crate::tui::theme::Colors,
}

impl TerminalWidget<'_> {
    /// Render a thin header bar: branch on the left, [x] on the right.
    fn render_header(&self, area: Rect, buf: &mut Buffer) {
        let bg = Color::Black;

        // Fill background.
        for x in area.x..area.x + area.width {
            buf[(x, area.y)].set_char(' ').set_bg(bg);
        }

        // Left side: git branch (if available).
        if let Some(branch) = git_branch() {
            let label = format!(" {branch}");
            for (i, ch) in label.chars().enumerate() {
                let x = area.x + i as u16;
                if x >= area.x + area.width.saturating_sub(5) {
                    break;
                }
                buf[(x, area.y)]
                    .set_char(ch)
                    .set_fg(Color::White)
                    .set_bg(bg);
            }
        }

        // Right side: close button.
        let close = " [x] ";
        let close_len = close.len() as u16;
        if area.width > close_len {
            let close_x = area.x + area.width - close_len;
            for (i, ch) in close.chars().enumerate() {
                let x = close_x + i as u16;
                buf[(x, area.y)]
                    .set_char(ch)
                    .set_fg(Color::White)
                    .set_bg(bg);
            }
        }
    }

    /// Render the vt100 screen grid into the body area.
    fn render_screen(&self, area: Rect, buf: &mut Buffer) {
        let screen = self.panel.screen();
        let (screen_rows, screen_cols) = screen.size();

        // Fill background with bg_primary.
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                buf[(x, y)].set_char(' ').set_bg(self.colors.bg_primary);
            }
        }

        // Render each cell from the vt100 screen.
        // Note: `set_scrollback()` on the parser already shifts what `screen.cell()`
        // returns, so we always read from (row, col) directly regardless of
        // scroll_offset.
        let render_rows = area.height.min(screen_rows);
        let render_cols = area.width.min(screen_cols);

        for row in 0..render_rows {
            for col in 0..render_cols {
                if let Some(cell) = screen.cell(row, col) {
                    // Skip the second half of wide characters — ratatui handles
                    // the width from the first cell.
                    if cell.is_wide_continuation() {
                        continue;
                    }

                    let x = area.x + col;
                    let y = area.y + row;

                    let style = colors::cell_style(cell);
                    let contents = cell.contents();

                    let buf_cell = &mut buf[(x, y)];
                    if contents.is_empty() {
                        buf_cell.set_char(' ');
                    } else {
                        buf_cell.set_symbol(&contents);
                    }
                    buf_cell.set_style(style);

                    // If the vt100 cell has default bg, apply our theme bg.
                    if cell.bgcolor() == vt100::Color::Default {
                        buf_cell.set_bg(self.colors.bg_primary);
                    }

                    // Wide characters: set width so ratatui skips the next cell.
                    if cell.is_wide() && col + 1 < render_cols {
                        let next = &mut buf[(x + 1, y)];
                        next.set_symbol("");
                        next.set_style(style);
                        if cell.bgcolor() == vt100::Color::Default {
                            next.set_bg(self.colors.bg_primary);
                        }
                    }
                }
            }
        }

        // Cursor: show with REVERSED modifier if focused and not scrolled back.
        if self.focused && self.panel.scroll_offset == 0 {
            let (cursor_row, cursor_col) = screen.cursor_position();
            if cursor_row < render_rows && cursor_col < render_cols {
                let x = area.x + cursor_col;
                let y = area.y + cursor_row;
                let current_style = buf[(x, y)].style();
                buf[(x, y)].set_style(current_style.add_modifier(Modifier::REVERSED));
            }
        }

        // Scrollback indicator: show line count when scrolled up.
        if self.panel.scroll_offset > 0 {
            let indicator = format!(" [{} lines above] ", self.panel.scroll_offset);
            let indicator_len = indicator.len() as u16;
            if indicator_len <= area.width {
                let start_x = area.x + area.width - indicator_len;
                for (i, ch) in indicator.chars().enumerate() {
                    let x = start_x + i as u16;
                    buf[(x, area.y)]
                        .set_char(ch)
                        .set_fg(self.colors.text_muted)
                        .set_bg(self.colors.bg_elevated);
                }
            }
        }
    }
}

impl Widget for TerminalWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 2 {
            return; // Need at least header + 1 row of content.
        }

        let header_area = Rect { height: 1, ..area };
        let body_area = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };

        self.render_header(header_area, buf);
        self.render_screen(body_area, buf);
    }
}
