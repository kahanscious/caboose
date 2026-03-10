//! Multi-line input buffer with cursor tracking.

/// Word-wrap a line into chunks that fit within `width` characters.
/// Returns byte-offset break points. Words stay together when possible;
/// only forced-breaks mid-word when a single word exceeds `width`.
pub fn word_wrap_offsets(line: &str, width: usize) -> Vec<usize> {
    if line.is_empty() || width == 0 {
        return vec![0];
    }
    let mut breaks = vec![0usize];
    let mut col = 0; // current column position
    let mut last_space = None; // byte offset of last space seen on this visual line
    for (i, c) in line.char_indices() {
        if c == ' ' {
            last_space = Some(i);
        }
        col += c.len_utf8();
        if col > width {
            if let Some(sp) = last_space {
                // Break after the space (skip the space itself)
                let next = sp + 1;
                if next > *breaks.last().unwrap() {
                    breaks.push(next);
                    // Recompute col from the break point
                    col = line[next..=i].len();
                    last_space = None;
                    continue;
                }
            }
            // No space found — forced break at current position
            breaks.push(i);
            col = c.len_utf8();
            last_space = None;
        }
    }
    breaks
}

/// Count visual rows for a single line when word-wrapped at `width`.
pub fn word_wrap_count(line: &str, width: usize) -> usize {
    word_wrap_offsets(line, width).len()
}

/// A multi-line text buffer with cursor position.
#[derive(Debug)]
pub struct InputBuffer {
    lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    /// Get the full content, joining lines with newlines.
    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Whether the buffer is empty (single empty line).
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Number of lines in the buffer.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Get the current line under the cursor.
    #[allow(dead_code)]
    pub fn current_line(&self) -> &str {
        &self.lines[self.cursor_row]
    }

    /// Iterate over all lines.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|s| s.as_str())
    }

    /// Clear all content and reset cursor.
    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.lines[self.cursor_row].insert(self.cursor_col, c);
        self.cursor_col += c.len_utf8();
    }

    /// Insert a newline, splitting the current line at the cursor.
    pub fn insert_newline(&mut self) {
        let tail = self.lines[self.cursor_row][self.cursor_col..].to_string();
        self.lines[self.cursor_row].truncate(self.cursor_col);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, tail);
        self.cursor_col = 0;
    }

    /// Delete the character before the cursor, or merge lines at boundary.
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let line = &self.lines[self.cursor_row];
            let prev_char_boundary = line[..self.cursor_col]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.lines[self.cursor_row].remove(prev_char_boundary);
            self.cursor_col = prev_char_boundary;
        } else if self.cursor_row > 0 {
            let current_line = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current_line);
        }
    }

    /// Move cursor up. Returns false if already at top row.
    pub fn move_up(&mut self) -> bool {
        if self.cursor_row == 0 {
            return false;
        }
        self.cursor_row -= 1;
        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
        true
    }

    /// Move cursor down. Returns false if already at bottom row.
    pub fn move_down(&mut self) -> bool {
        if self.cursor_row >= self.lines.len() - 1 {
            return false;
        }
        self.cursor_row += 1;
        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
        true
    }

    /// Move cursor left, wrapping to previous line if at start.
    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            let line = &self.lines[self.cursor_row];
            self.cursor_col = line[..self.cursor_col]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
    }

    /// Move cursor right, wrapping to next line if at end.
    pub fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            let line = &self.lines[self.cursor_row];
            self.cursor_col = line[self.cursor_col..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor_col + i)
                .unwrap_or(line_len);
        } else if self.cursor_row < self.lines.len() - 1 {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    /// Append a string (possibly multi-line) at the cursor.
    pub fn push_str(&mut self, s: &str) {
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                self.insert_newline();
            }
            for ch in part.chars() {
                self.insert_char(ch);
            }
        }
    }

    /// Count total visual rows needed when word-wrapping at `width`.
    /// `width` is the usable character width (excluding accent bar + space + cursor).
    pub fn visual_line_count(&self, width: u16) -> usize {
        let w = (width as usize).max(1);
        self.lines
            .iter()
            .map(|line| {
                if line.is_empty() {
                    1
                } else {
                    word_wrap_count(line, w)
                }
            })
            .sum()
    }

    /// Get visual cursor (row, col) accounting for word-wrapping at `width`.
    pub fn visual_cursor(&self, width: u16) -> (usize, usize) {
        let w = (width as usize).max(1);
        let mut visual_row = 0;
        for (i, line) in self.lines.iter().enumerate() {
            if i == self.cursor_row {
                let offsets = word_wrap_offsets(line, w);
                // Find which visual row the cursor falls on
                let mut wrap_row = 0;
                for (j, &start) in offsets.iter().enumerate() {
                    let next = offsets.get(j + 1).copied().unwrap_or(line.len());
                    if self.cursor_col >= start && self.cursor_col <= next {
                        // If cursor is exactly at the start of the NEXT row,
                        // place it there (unless it's the last segment)
                        if self.cursor_col == next && j + 1 < offsets.len() {
                            wrap_row = j + 1;
                        } else {
                            wrap_row = j;
                        }
                        break;
                    }
                }
                let row_start = offsets[wrap_row];
                let wrap_col = self.cursor_col - row_start;
                return (visual_row + wrap_row, wrap_col);
            }
            visual_row += if line.is_empty() {
                1
            } else {
                word_wrap_count(line, w)
            };
        }
        (visual_row, 0)
    }

    /// Set content from a string, replacing everything.
    pub fn set(&mut self, s: &str) {
        self.clear();
        if !s.is_empty() {
            self.push_str(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_empty() {
        let buf = InputBuffer::new();
        assert!(buf.is_empty());
        assert_eq!(buf.content(), "");
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn insert_chars() {
        let mut buf = InputBuffer::new();
        buf.insert_char('h');
        buf.insert_char('i');
        assert_eq!(buf.content(), "hi");
        assert!(!buf.is_empty());
    }

    #[test]
    fn insert_newline_creates_second_line() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_newline();
        buf.insert_char('b');
        assert_eq!(buf.content(), "a\nb");
        assert_eq!(buf.line_count(), 2);
        assert_eq!(buf.cursor_row, 1);
        assert_eq!(buf.cursor_col, 1);
    }

    #[test]
    fn backspace_at_line_boundary_merges() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_newline();
        buf.insert_char('b');
        buf.cursor_col = 0;
        buf.backspace();
        assert_eq!(buf.content(), "ab");
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.cursor_col, 1);
    }

    #[test]
    fn backspace_deletes_char() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_char('b');
        buf.backspace();
        assert_eq!(buf.content(), "a");
    }

    #[test]
    fn backspace_on_empty_is_noop() {
        let mut buf = InputBuffer::new();
        buf.backspace();
        assert_eq!(buf.content(), "");
    }

    #[test]
    fn clear_resets() {
        let mut buf = InputBuffer::new();
        buf.insert_char('x');
        buf.insert_newline();
        buf.insert_char('y');
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.cursor_row, 0);
        assert_eq!(buf.cursor_col, 0);
    }

    #[test]
    fn push_str_multiline() {
        let mut buf = InputBuffer::new();
        buf.push_str("line1\nline2\nline3");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.content(), "line1\nline2\nline3");
    }

    #[test]
    fn move_up_returns_false_at_top() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        assert!(!buf.move_up());
    }

    #[test]
    fn move_down_returns_false_at_bottom() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        assert!(!buf.move_down());
    }

    #[test]
    fn move_up_down_between_lines() {
        let mut buf = InputBuffer::new();
        buf.insert_char('a');
        buf.insert_newline();
        buf.insert_char('b');
        assert!(buf.move_up());
        assert_eq!(buf.cursor_row, 0);
        assert!(buf.move_down());
        assert_eq!(buf.cursor_row, 1);
    }

    #[test]
    fn cursor_col_clamped_on_move_up() {
        let mut buf = InputBuffer::new();
        buf.push_str("ab\nxyzw");
        buf.move_up();
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn current_line_returns_correct_line() {
        let mut buf = InputBuffer::new();
        buf.push_str("hello\nworld");
        assert_eq!(buf.current_line(), "world");
        buf.move_up();
        assert_eq!(buf.current_line(), "hello");
    }

    #[test]
    fn lines_returns_all_lines() {
        let mut buf = InputBuffer::new();
        buf.push_str("a\nb\nc");
        let lines: Vec<&str> = buf.lines().collect();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn set_replaces_content() {
        let mut buf = InputBuffer::new();
        buf.insert_char('x');
        buf.set("hello\nworld");
        assert_eq!(buf.content(), "hello\nworld");
        assert_eq!(buf.line_count(), 2);
    }

    #[test]
    fn set_empty_string() {
        let mut buf = InputBuffer::new();
        buf.insert_char('x');
        buf.set("");
        assert!(buf.is_empty());
    }
}
