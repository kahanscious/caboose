//! Input history — stores previous inputs with Up/Down browsing and JSON persistence.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const MAX_HISTORY: usize = 100;

#[derive(Debug, Serialize, Deserialize)]
pub struct InputHistory {
    pub entries: Vec<String>,
    #[serde(skip)]
    pub position: Option<usize>,
    #[serde(skip)]
    draft: String,
}

impl InputHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            position: None,
            draft: String::new(),
        }
    }

    /// Push a new entry. Skips empty strings and consecutive duplicates.
    pub fn push(&mut self, entry: String) {
        if entry.is_empty() {
            return;
        }
        if self.entries.last() == Some(&entry) {
            return;
        }
        self.entries.push(entry);
        if self.entries.len() > MAX_HISTORY {
            self.entries.remove(0);
        }
        self.reset();
    }

    /// Browse to the previous (older) entry. Saves draft on first call.
    /// Returns the entry to display, or None if already at oldest.
    pub fn browse_up(&mut self, current_input: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        match self.position {
            None => {
                self.draft = current_input.to_string();
                let idx = self.entries.len() - 1;
                self.position = Some(idx);
                Some(self.entries[idx].clone())
            }
            Some(pos) if pos > 0 => {
                let idx = pos - 1;
                self.position = Some(idx);
                Some(self.entries[idx].clone())
            }
            Some(_) => None,
        }
    }

    /// Browse to the next (newer) entry.
    /// Returns the entry, or the draft when moving past newest.
    pub fn browse_down(&mut self) -> Option<String> {
        match self.position {
            None => None,
            Some(pos) => {
                if pos + 1 < self.entries.len() {
                    let idx = pos + 1;
                    self.position = Some(idx);
                    Some(self.entries[idx].clone())
                } else {
                    self.position = None;
                    Some(self.draft.clone())
                }
            }
        }
    }

    /// Reset browse state without clearing entries.
    pub fn reset(&mut self) {
        self.position = None;
        self.draft.clear();
    }

    /// Load from disk.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(Self::new)
    }

    /// Save to disk.
    pub fn save(&self) {
        if let Some(path) = Self::path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, serde_json::to_string(&self).unwrap_or_default());
        }
    }

    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("caboose").join("tui_history.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_history_is_empty() {
        let h = InputHistory::new();
        assert!(h.entries.is_empty());
        assert!(h.position.is_none());
    }

    #[test]
    fn push_adds_entry() {
        let mut h = InputHistory::new();
        h.push("hello".to_string());
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.entries[0], "hello");
    }

    #[test]
    fn push_caps_at_max() {
        let mut h = InputHistory::new();
        for i in 0..150 {
            h.push(format!("entry {i}"));
        }
        assert_eq!(h.entries.len(), MAX_HISTORY);
        assert_eq!(h.entries[0], "entry 50");
    }

    #[test]
    fn browse_up_returns_latest() {
        let mut h = InputHistory::new();
        h.push("first".to_string());
        h.push("second".to_string());
        let result = h.browse_up("current draft");
        assert_eq!(result, Some("second".to_string()));
    }

    #[test]
    fn browse_up_then_down_restores_draft() {
        let mut h = InputHistory::new();
        h.push("old".to_string());
        h.browse_up("my draft");
        let result = h.browse_down();
        assert_eq!(result, Some("my draft".to_string()));
        assert!(h.position.is_none());
    }

    #[test]
    fn browse_up_past_oldest_stays_at_oldest() {
        let mut h = InputHistory::new();
        h.push("only".to_string());
        h.browse_up("");
        let result = h.browse_up("");
        assert_eq!(result, None);
    }

    #[test]
    fn browse_down_without_browsing_returns_none() {
        let mut h = InputHistory::new();
        h.push("something".to_string());
        assert_eq!(h.browse_down(), None);
    }

    #[test]
    fn reset_clears_browse_state() {
        let mut h = InputHistory::new();
        h.push("item".to_string());
        h.browse_up("draft");
        h.reset();
        assert!(h.position.is_none());
    }

    #[test]
    fn empty_strings_not_pushed() {
        let mut h = InputHistory::new();
        h.push("".to_string());
        assert!(h.entries.is_empty());
    }

    #[test]
    fn duplicate_consecutive_not_pushed() {
        let mut h = InputHistory::new();
        h.push("same".to_string());
        h.push("same".to_string());
        assert_eq!(h.entries.len(), 1);
    }
}
