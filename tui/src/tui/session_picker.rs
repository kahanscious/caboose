//! Session picker — interactive dialog for browsing, filtering, and switching sessions.

use crate::session::Session;

/// State for the session picker overlay.
#[allow(dead_code)]
pub struct SessionPickerState {
    pub sessions: Vec<Session>,
    pub filter: String,
    pub selected: usize,
    pub confirm_delete: Option<usize>,
}

#[allow(dead_code)]
impl SessionPickerState {
    pub fn new(sessions: Vec<Session>) -> Self {
        Self {
            sessions,
            filter: String::new(),
            selected: 0,
            confirm_delete: None,
        }
    }

    /// Get the filtered list of sessions matching the current search.
    pub fn filtered(&self) -> Vec<&Session> {
        if self.filter.is_empty() {
            return self.sessions.iter().collect();
        }
        let needle = self.filter.to_lowercase();
        self.sessions
            .iter()
            .filter(|s| {
                // Match on title (case-insensitive contains)
                let title_match = s
                    .title
                    .as_ref()
                    .map(|t| t.to_lowercase().contains(&needle))
                    .unwrap_or(false);
                // Match on id prefix
                let id_match = s.id.starts_with(&self.filter);
                // Match on 8-char id prefix
                let short_id_match = s.id.len() >= 8 && s.id[..8].to_lowercase().contains(&needle);
                title_match || id_match || short_id_match
            })
            .collect()
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        let max = self.filtered().len().saturating_sub(1);
        if self.selected < max {
            self.selected += 1;
        }
    }

    /// Get the session ID of the currently selected entry.
    pub fn selected_id(&self) -> Option<String> {
        let filtered = self.filtered();
        filtered.get(self.selected).map(|s| s.id.clone())
    }

    /// Clamp selected index to valid range after filter change.
    pub fn clamp_selection(&mut self) {
        let max = self.filtered().len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }

    /// Remove a session from the cached list by ID.
    pub fn remove_session(&mut self, id: &str) {
        self.sessions.retain(|s| s.id != id);
        self.clamp_selection();
        self.confirm_delete = None;
    }
}

/// Format a timestamp as a human-readable relative time.
pub fn format_relative_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(dt);
    let seconds = duration.num_seconds();
    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{}h ago", seconds / 3600)
    } else if seconds < 86400 * 30 {
        format!("{}d ago", seconds / 86400)
    } else {
        dt.format("%b %d").to_string()
    }
}

/// Truncate a string at the nearest word boundary, up to `max_len` characters.
pub fn truncate_at_word_boundary(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    // Find the last space before max_len
    let truncated = &s[..max_len];
    match truncated.rfind(' ') {
        Some(pos) if pos > max_len / 2 => truncated[..pos].to_string(),
        _ => truncated.to_string(),
    }
}

use crate::session::storage::SessionSearchResult;

/// A filtered session search result with an optional content snippet.
#[derive(Debug, Clone)]
pub struct FilteredSession {
    pub session: crate::session::Session,
    /// If the match came from content (not title/metadata), a snippet around the match.
    pub matched_snippet: Option<String>,
}

/// Extract a snippet centered on the first occurrence of `query` in `content`.
/// Returns `None` if query is not found. Snippet is at most `max_len` chars,
/// with "..." prefix/suffix when truncated.
pub fn extract_snippet(content: &str, query: &str, max_len: usize) -> Option<String> {
    let lower_content = content.to_lowercase();
    let lower_query = query.to_lowercase();
    let pos = lower_content.find(&lower_query)?;

    if content.len() <= max_len {
        return Some(content.to_string());
    }

    // Center the window around the match
    let half = max_len / 2;
    let start = pos.saturating_sub(half);
    let end = (start + max_len).min(content.len());
    let start = if end == content.len() {
        end.saturating_sub(max_len)
    } else {
        start
    };

    // Snap to char boundaries
    let start = content.floor_char_boundary(start);
    let end = content.ceil_char_boundary(end).min(content.len());

    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(&content[start..end]);
    if end < content.len() {
        snippet.push_str("...");
    }
    Some(snippet)
}

/// Filter session search results against a query string.
/// Matches (in priority order): title, ID prefix, provider, model, content.
/// Only content matches get a `matched_snippet`.
pub fn filter_search_results(results: &[SessionSearchResult], query: &str) -> Vec<FilteredSession> {
    if query.is_empty() {
        return results
            .iter()
            .map(|r| FilteredSession {
                session: r.session.clone(),
                matched_snippet: None,
            })
            .collect();
    }

    let needle = query.to_lowercase();
    let mut filtered = Vec::new();

    for r in results {
        let s = &r.session;

        // Title match
        let title_match = s
            .title
            .as_ref()
            .map(|t| t.to_lowercase().contains(&needle))
            .unwrap_or(false);

        // ID prefix match (first 8 chars)
        let id_match = s.id.len() >= 8 && s.id[..8].to_lowercase().contains(&needle);

        // Provider/model match
        let provider_match = s
            .provider
            .as_ref()
            .map(|p| p.to_lowercase().contains(&needle))
            .unwrap_or(false);
        let model_match = s
            .model
            .as_ref()
            .map(|m| m.to_lowercase().contains(&needle))
            .unwrap_or(false);

        let metadata_match = title_match || id_match || provider_match || model_match;

        // Content match
        let content_match = r.content_index.to_lowercase().contains(&needle);

        if metadata_match || content_match {
            let matched_snippet = if !metadata_match && content_match {
                extract_snippet(&r.content_index, query, 60)
            } else {
                None
            };
            filtered.push(FilteredSession {
                session: r.session.clone(),
                matched_snippet,
            });
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(id: &str, title: Option<&str>, turns: u32) -> Session {
        let now = chrono::Utc::now();
        Session {
            id: id.to_string(),
            title: title.map(|t| t.to_string()),
            model: Some("claude-sonnet".to_string()),
            provider: Some("anthropic".to_string()),
            turn_count: turns,
            cwd: Some("/tmp".to_string()),
            created_at: now,
            updated_at: now,
            parent_session_id: None,
            fork_message_count: None,
        }
    }

    #[test]
    fn empty_filter_returns_all() {
        let sessions = vec![
            make_session("aaa-111", Some("First"), 5),
            make_session("bbb-222", Some("Second"), 3),
        ];
        let state = SessionPickerState::new(sessions);
        assert_eq!(state.filtered().len(), 2);
    }

    #[test]
    fn filter_by_title() {
        let sessions = vec![
            make_session("aaa-111", Some("Fix auth bug"), 5),
            make_session("bbb-222", Some("Refactor database"), 3),
        ];
        let mut state = SessionPickerState::new(sessions);
        state.filter = "auth".to_string();
        let filtered = state.filtered();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].title.as_deref(), Some("Fix auth bug"));
    }

    #[test]
    fn filter_by_id_prefix() {
        let sessions = vec![
            make_session("abcd1234-rest", Some("First"), 5),
            make_session("efgh5678-rest", Some("Second"), 3),
        ];
        let mut state = SessionPickerState::new(sessions);
        state.filter = "abcd1234".to_string();
        let filtered = state.filtered();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "abcd1234-rest");
    }

    #[test]
    fn filter_case_insensitive() {
        let sessions = vec![make_session("aaa-111", Some("Fix Auth Bug"), 5)];
        let mut state = SessionPickerState::new(sessions);
        state.filter = "fix auth".to_string();
        assert_eq!(state.filtered().len(), 1);
    }

    #[test]
    fn clamp_selection_after_filter() {
        let sessions = vec![
            make_session("aaa", Some("One"), 1),
            make_session("bbb", Some("Two"), 2),
            make_session("ccc", Some("Three"), 3),
        ];
        let mut state = SessionPickerState::new(sessions);
        state.selected = 2;
        state.filter = "One".to_string();
        state.clamp_selection();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn selected_id_returns_correct_session() {
        let sessions = vec![
            make_session("aaa", Some("One"), 1),
            make_session("bbb", Some("Two"), 2),
        ];
        let mut state = SessionPickerState::new(sessions);
        state.selected = 1;
        assert_eq!(state.selected_id(), Some("bbb".to_string()));
    }

    #[test]
    fn remove_session_updates_list() {
        let sessions = vec![
            make_session("aaa", Some("One"), 1),
            make_session("bbb", Some("Two"), 2),
        ];
        let mut state = SessionPickerState::new(sessions);
        state.selected = 1;
        state.remove_session("bbb");
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.selected, 0);
        assert!(state.confirm_delete.is_none());
    }

    #[test]
    fn format_relative_just_now() {
        let now = chrono::Utc::now();
        assert_eq!(format_relative_time(now), "just now");
    }

    #[test]
    fn format_relative_minutes() {
        let t = chrono::Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(format_relative_time(t), "5m ago");
    }

    #[test]
    fn format_relative_hours() {
        let t = chrono::Utc::now() - chrono::Duration::hours(3);
        assert_eq!(format_relative_time(t), "3h ago");
    }

    #[test]
    fn format_relative_days() {
        let t = chrono::Utc::now() - chrono::Duration::days(7);
        assert_eq!(format_relative_time(t), "7d ago");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_at_word_boundary("hello", 60), "hello");
    }

    #[test]
    fn truncate_at_word_boundary_works() {
        let long = "fix the authentication bug in the login flow handler module";
        let result = truncate_at_word_boundary(long, 40);
        assert!(result.len() <= 40);
        assert!(!result.ends_with(' '));
        // Should break at a word boundary
        assert!(long.starts_with(&result));
    }

    use crate::session::storage::SessionSearchResult;

    fn make_search_result(id: &str, title: Option<&str>, content: &str) -> SessionSearchResult {
        SessionSearchResult {
            session: make_session(id, title, 3),
            content_index: content.to_string(),
        }
    }

    #[test]
    fn extract_snippet_centers_on_match() {
        let content = "The quick brown fox jumps over the lazy dog near the river bank";
        let snippet = super::extract_snippet(content, "lazy", 50);
        assert!(snippet.is_some());
        let snippet = snippet.unwrap();
        assert!(snippet.contains("lazy"));
        assert!(snippet.len() <= 56); // 50 + "..." prefix/suffix
    }

    #[test]
    fn extract_snippet_no_match_returns_none() {
        let content = "The quick brown fox";
        assert!(super::extract_snippet(content, "zebra", 50).is_none());
    }

    #[test]
    fn extract_snippet_short_content_no_ellipsis() {
        let content = "Fix auth bug";
        let snippet = super::extract_snippet(content, "auth", 50);
        assert_eq!(snippet, Some("Fix auth bug".to_string()));
    }

    #[test]
    fn filter_search_results_empty_query_returns_all() {
        let results = vec![
            make_search_result("s1", Some("First"), "hello"),
            make_search_result("s2", Some("Second"), "world"),
        ];
        let filtered = super::filter_search_results(&results, "");
        assert_eq!(filtered.len(), 2);
        assert!(filtered[0].matched_snippet.is_none());
        assert!(filtered[1].matched_snippet.is_none());
    }

    #[test]
    fn filter_search_results_matches_title() {
        let results = vec![
            make_search_result("s1", Some("Fix auth bug"), "some content"),
            make_search_result("s2", Some("Refactor DB"), "other content"),
        ];
        let filtered = super::filter_search_results(&results, "auth");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session.id, "s1");
        // Title match — no snippet needed
        assert!(filtered[0].matched_snippet.is_none());
    }

    #[test]
    fn filter_search_results_matches_content_with_snippet() {
        let results = vec![
            make_search_result(
                "s1",
                Some("Session one"),
                "The JWT token was expiring too early",
            ),
            make_search_result("s2", Some("Session two"), "Refactored the database layer"),
        ];
        let filtered = super::filter_search_results(&results, "JWT");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session.id, "s1");
        assert!(filtered[0].matched_snippet.is_some());
        assert!(
            filtered[0]
                .matched_snippet
                .as_ref()
                .unwrap()
                .contains("JWT")
        );
    }

    #[test]
    fn filter_search_results_matches_provider() {
        let results = vec![make_search_result("s1", Some("Session one"), "content")];
        // The provider is "anthropic" from make_session helper
        let filtered = super::filter_search_results(&results, "anthropic");
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn filter_search_results_title_match_takes_priority_over_content() {
        let results = vec![make_search_result(
            "s1",
            Some("Fix auth bug"),
            "auth token code here",
        )];
        // "auth" matches both title and content — should NOT show snippet (title match is sufficient)
        let filtered = super::filter_search_results(&results, "auth");
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].matched_snippet.is_none());
    }

    #[test]
    fn truncate_no_space_takes_hard_cut() {
        let long = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let result = truncate_at_word_boundary(long, 10);
        assert_eq!(result.len(), 10);
    }
}
