//! Fullscreen fuzzy file picker for image attachment.

use std::path::PathBuf;

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::tui::theme::Colors;

/// State for the file browser dialog.
#[derive(Debug)]
pub struct FileBrowserState {
    pub cwd: PathBuf,
    pub filter: String,
    pub entries: Vec<FileEntry>,
    pub filtered: Vec<usize>, // indices into entries
    pub selected: usize,
}

/// A single entry in the file browser.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub display: String,
}

impl FileBrowserState {
    /// Create a new file browser rooted at the given directory.
    pub fn new(cwd: PathBuf) -> Self {
        let mut state = Self {
            cwd,
            filter: String::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
        };
        state.scan_directory();
        state.apply_filter();
        state
    }

    /// Scan the current directory and populate `entries`.
    /// Respects `.gitignore` when present; skips hidden files by default.
    pub fn scan_directory(&mut self) {
        self.entries.clear();

        // Add parent directory entry if not at root
        if let Some(parent) = self.cwd.parent() {
            self.entries.push(FileEntry {
                path: parent.to_path_buf(),
                is_dir: true,
                display: "..".to_string(),
            });
        }

        let mut dirs = Vec::new();
        let mut files = Vec::new();

        // Use ignore::WalkBuilder with depth=1 to list immediate children
        // while respecting .gitignore. Hidden files are skipped by default.
        let walker = ignore::WalkBuilder::new(&self.cwd)
            .max_depth(Some(1))
            .require_git(false)
            .build();

        for result in walker.flatten() {
            let path = result.path().to_path_buf();
            // Skip the root directory itself
            if path == self.cwd {
                continue;
            }
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            let is_dir = path.is_dir();

            let fe = FileEntry {
                path,
                is_dir,
                display: name,
            };

            if is_dir {
                dirs.push(fe);
            } else {
                files.push(fe);
            }
        }

        // Sort alphabetically, dirs first
        dirs.sort_by(|a, b| a.display.to_lowercase().cmp(&b.display.to_lowercase()));
        files.sort_by(|a, b| a.display.to_lowercase().cmp(&b.display.to_lowercase()));

        self.entries.extend(dirs);
        self.entries.extend(files);
    }

    /// Apply the current filter to produce `filtered` indices.
    pub fn apply_filter(&mut self) {
        let query = self.filter.to_lowercase();
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if query.is_empty() {
                    return true;
                }
                e.display.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        // Reset selection if out of bounds
        if self.selected >= self.filtered.len() {
            self.selected = 0;
        }
    }

    /// Get the currently selected entry, if any.
    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.entries.get(idx))
    }

    /// Navigate into a directory: update cwd, rescan, clear filter.
    pub fn navigate_into(&mut self, dir: PathBuf) {
        self.cwd = dir;
        self.filter.clear();
        self.selected = 0;
        self.scan_directory();
        self.apply_filter();
    }

    /// Move selection up.
    pub fn select_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection down.
    pub fn select_down(&mut self) {
        if !self.filtered.is_empty() && self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    /// Push a character to the filter and reapply.
    pub fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.apply_filter();
    }

    /// Pop a character from the filter and reapply.
    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.apply_filter();
    }
}

/// Render the file browser overlay.
pub fn render(frame: &mut Frame, state: &FileBrowserState, colors: &Colors) {
    let area = frame.area();

    // Centered popup: 70 wide, up to 24 tall
    let width = 70.min(area.width.saturating_sub(4));
    let height = 24.min(area.height.saturating_sub(2));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    // Split: title/path (3) + filter (3) + list (rest)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(popup_area);

    // Title bar with current path
    let cwd_display = state.cwd.to_string_lossy();
    let title_block = Block::default()
        .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(colors.border_active))
        .title(" Attach Image ")
        .title_style(Style::default().fg(colors.text).bold())
        .style(Style::default().bg(colors.bg_elevated));
    let path_text = Paragraph::new(Span::styled(
        format!(" {cwd_display}"),
        Style::default().fg(colors.text_dim),
    ));
    frame.render_widget(path_text.block(title_block), chunks[0]);

    // Filter input
    let filter_block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(colors.border_active))
        .style(Style::default().bg(colors.bg_elevated));
    let filter_text = if state.filter.is_empty() {
        Paragraph::new(Span::styled(
            " Type to filter...",
            Style::default().fg(colors.text_muted),
        ))
    } else {
        Paragraph::new(Span::styled(
            format!(" > {}", state.filter),
            Style::default().fg(colors.text),
        ))
    };
    frame.render_widget(filter_text.block(filter_block), chunks[1]);

    // File list
    let items: Vec<ListItem> = state
        .filtered
        .iter()
        .map(|&idx| {
            let entry = &state.entries[idx];
            let icon = if entry.display == ".." {
                "\u{2190} " // ← arrow
            } else if entry.is_dir {
                "/ "
            } else {
                "  "
            };
            let style = if entry.is_dir {
                Style::default().fg(colors.info)
            } else if crate::attachment::is_image_path(&entry.path) {
                Style::default().fg(colors.success)
            } else {
                Style::default().fg(colors.text_dim)
            };
            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::raw(icon),
                Span::styled(&entry.display, style),
            ]))
        })
        .collect();

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
    list_state.select(Some(state.selected));
    frame.render_stateful_widget(list, chunks[2], &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_browser_state_new_scans_cwd() {
        let dir = tempfile::tempdir().unwrap();
        // Create a .gitignore to filter node_modules
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        // Create some test files and dirs
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("file.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("image.png"), "fake").unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();

        let state = FileBrowserState::new(dir.path().to_path_buf());

        // Should have ".." + subdir + file.txt + image.png
        // Hidden files skipped by default, node_modules by .gitignore
        let names: Vec<&str> = state.entries.iter().map(|e| e.display.as_str()).collect();
        assert!(names.contains(&".."));
        assert!(names.contains(&"subdir"));
        assert!(names.contains(&"file.txt"));
        assert!(names.contains(&"image.png"));
        assert!(!names.contains(&".hidden"));
        assert!(!names.contains(&"node_modules"));
    }

    #[test]
    fn file_browser_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), "a").unwrap();
        std::fs::write(dir.path().join("beta.txt"), "b").unwrap();
        std::fs::write(dir.path().join("gamma.png"), "c").unwrap();

        let mut state = FileBrowserState::new(dir.path().to_path_buf());
        state.push_filter('p');
        state.push_filter('h');

        // Only "alpha" contains "ph"
        let filtered_names: Vec<&str> = state
            .filtered
            .iter()
            .map(|&i| state.entries[i].display.as_str())
            .collect();
        assert!(filtered_names.contains(&"alpha.txt"));
        assert!(!filtered_names.contains(&"beta.txt"));
        assert!(!filtered_names.contains(&"gamma.png"));
    }

    #[test]
    fn file_browser_navigate_into() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("child");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("inner.txt"), "x").unwrap();

        let mut state = FileBrowserState::new(dir.path().to_path_buf());
        state.navigate_into(sub.clone());

        assert_eq!(state.cwd, sub);
        let names: Vec<&str> = state.entries.iter().map(|e| e.display.as_str()).collect();
        assert!(names.contains(&"inner.txt"));
        assert!(names.contains(&".."));
    }

    #[test]
    fn file_browser_select_up_down() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();

        let mut state = FileBrowserState::new(dir.path().to_path_buf());
        assert_eq!(state.selected, 0);

        state.select_down();
        assert_eq!(state.selected, 1);

        state.select_up();
        assert_eq!(state.selected, 0);

        // Can't go below 0
        state.select_up();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn file_browser_selected_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.txt"), "").unwrap();

        let state = FileBrowserState::new(dir.path().to_path_buf());
        let entry = state.selected_entry();
        assert!(entry.is_some());
    }

    #[test]
    fn file_browser_dirs_sorted_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z_file.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("a_dir")).unwrap();
        std::fs::write(dir.path().join("a_file.txt"), "").unwrap();
        std::fs::create_dir(dir.path().join("z_dir")).unwrap();

        let state = FileBrowserState::new(dir.path().to_path_buf());
        // After "..", dirs should come first (a_dir, z_dir), then files (a_file, z_file)
        let names: Vec<&str> = state.entries.iter().map(|e| e.display.as_str()).collect();
        let dir_end = names.iter().position(|n| *n == "a_file.txt").unwrap();
        // All entries before a_file.txt should be dirs or ".."
        for name in &names[..dir_end] {
            let entry = state.entries.iter().find(|e| e.display == *name).unwrap();
            assert!(entry.is_dir, "{name} should be a dir");
        }
    }
}
