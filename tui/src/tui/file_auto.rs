//! @file autocomplete — fuzzy file path completion.

/// State for the @file autocomplete dropdown.
#[derive(Debug)]
pub struct FileAutoState {
    pub matches: Vec<String>,
    pub selected: usize,
    #[allow(dead_code)]
    pub prefix: String,
}

impl FileAutoState {
    pub fn new(prefix: String, matches: Vec<String>) -> Self {
        Self {
            matches,
            selected: 0,
            prefix,
        }
    }

    pub fn select_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_down(&mut self) {
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.matches.get(self.selected).map(|s| s.as_str())
    }
}

/// Extract the @-prefixed partial path from input text.
/// Returns the path portion after the last `@`, or None if no `@` present.
pub fn extract_at_prefix(input: &str) -> Option<&str> {
    let at_pos = input.rfind('@')?;
    // Make sure @ is at start of a "word" (preceded by space or is first char)
    if at_pos > 0 && !input.as_bytes()[at_pos - 1].is_ascii_whitespace() {
        return None;
    }
    Some(&input[at_pos + 1..])
}

/// Filter file paths that start with the given prefix.
#[allow(dead_code)]
pub fn filter_file_matches<'a>(prefix: &str, files: &[&'a str]) -> Vec<&'a str> {
    files
        .iter()
        .filter(|f| f.starts_with(prefix))
        .copied()
        .collect()
}

/// Score a directory path against a query string for workspace dir scanning.
/// Returns `Some(score)` if it matches (lower = better), `None` if no match.
pub fn score_path_for_dir(path: &str, query: &str) -> Option<u32> {
    fuzzy_score(query, path)
}

/// Fuzzy match score: checks if all characters of `query` appear in order
/// (case-insensitive) in `candidate`. Returns a score where lower is better,
/// or None if no match. Score prefers:
///   - exact prefix matches
///   - shorter total span of matched characters
///   - matches at path separator boundaries
fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }

    let query_lower: Vec<char> = query.to_lowercase().chars().collect();
    let cand_lower: Vec<char> = candidate.to_lowercase().chars().collect();
    let cand_chars: Vec<char> = candidate.chars().collect();

    // Check if candidate starts with query (prefix match — best score)
    if cand_lower.len() >= query_lower.len() && cand_lower[..query_lower.len()] == query_lower[..] {
        return Some(0);
    }

    // Check if any path component starts with query
    for (i, _) in cand_chars.iter().enumerate() {
        if i == 0 || cand_chars[i - 1] == '/' {
            let remaining = &cand_lower[i..];
            if remaining.len() >= query_lower.len()
                && remaining[..query_lower.len()] == query_lower[..]
            {
                return Some(1);
            }
        }
    }

    // Subsequence match: find all query chars in order
    let mut qi = 0;
    let mut first_match = None;
    let mut last_match = 0;
    let mut boundary_bonus: u32 = 0;

    for (ci, &ch) in cand_lower.iter().enumerate() {
        if qi < query_lower.len() && ch == query_lower[qi] {
            if first_match.is_none() {
                first_match = Some(ci);
            }
            last_match = ci;
            // Bonus for matching at word boundaries (after /, _, -, or uppercase)
            if ci == 0
                || cand_chars[ci - 1] == '/'
                || cand_chars[ci - 1] == '_'
                || cand_chars[ci - 1] == '-'
                || cand_chars[ci - 1] == '.'
            {
                boundary_bonus += 1;
            }
            qi += 1;
        }
    }

    if qi < query_lower.len() {
        return None; // Not all chars matched
    }

    let first = first_match.unwrap_or(0);
    let span = (last_match - first + 1) as u32;
    // Score: span penalized, boundary matches rewarded, position penalized slightly
    let score = 100 + span * 2 + first as u32 - boundary_bonus * 5;
    Some(score)
}

/// Scan the working directory for files matching a partial path.
/// Uses case-insensitive fuzzy matching. Respects `.gitignore` when present.
/// Returns relative paths sorted by match quality, capped at `max_results`.
pub fn scan_files(cwd: &std::path::Path, partial: &str, max_results: usize) -> Vec<String> {
    let scan_limit = 2000;
    let mut candidates = Vec::new();

    // WalkBuilder respects .gitignore automatically (falls back to showing
    // everything if no .gitignore exists). Hidden files are skipped by default.
    let walker = ignore::WalkBuilder::new(cwd)
        .max_depth(Some(10))
        .require_git(false)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(cwd) {
            candidates.push(rel.to_string_lossy().to_string());
            if candidates.len() >= scan_limit {
                break;
            }
        }
    }

    // Score and sort candidates by fuzzy match quality (case-insensitive)
    let mut scored: Vec<(u32, String)> = candidates
        .into_iter()
        .filter_map(|path| fuzzy_score(partial, &path).map(|score| (score, path)))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(max_results);
    scored.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_at_prefix_at_end() {
        assert_eq!(extract_at_prefix("hello @src/ma"), Some("src/ma"));
    }

    #[test]
    fn extract_at_prefix_empty() {
        assert_eq!(extract_at_prefix("hello world"), None);
    }

    #[test]
    fn extract_at_prefix_bare_at() {
        assert_eq!(extract_at_prefix("@"), Some(""));
    }

    #[test]
    fn extract_at_prefix_no_space_before_at() {
        assert_eq!(extract_at_prefix("email@domain"), None);
    }

    #[test]
    fn filter_matches_basic() {
        let files = vec!["src/main.rs", "src/app.rs", "src/lib.rs", "README.md"];
        let matches = filter_file_matches("src/m", &files);
        assert_eq!(matches, vec!["src/main.rs"]);
    }

    #[test]
    fn filter_matches_empty_prefix() {
        let files = vec!["a.rs", "b.rs"];
        let matches = filter_file_matches("", &files);
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn state_select_up_down() {
        let mut state = FileAutoState::new(
            "src/".to_string(),
            vec![
                "src/main.rs".to_string(),
                "src/app.rs".to_string(),
                "src/lib.rs".to_string(),
            ],
        );
        assert_eq!(state.selected, 0);
        state.select_down();
        assert_eq!(state.selected, 1);
        state.select_down();
        assert_eq!(state.selected, 2);
        state.select_down(); // stays at 2 (max)
        assert_eq!(state.selected, 2);
        state.select_up();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn selected_path_returns_correct() {
        let state = FileAutoState::new(
            "".to_string(),
            vec!["foo.rs".to_string(), "bar.rs".to_string()],
        );
        assert_eq!(state.selected_path(), Some("foo.rs"));
    }

    #[test]
    fn fuzzy_score_exact_prefix_best() {
        let score = fuzzy_score("src", "src/main.rs").unwrap();
        assert_eq!(score, 0);
    }

    #[test]
    fn fuzzy_score_path_component_prefix() {
        let score = fuzzy_score("main", "src/main.rs").unwrap();
        assert_eq!(score, 1);
    }

    #[test]
    fn fuzzy_score_subsequence_matches() {
        let score = fuzzy_score("road", "TUI_LAUNCH_ROADMAP.md");
        assert!(score.is_some());
    }

    #[test]
    fn fuzzy_score_no_match_returns_none() {
        assert!(fuzzy_score("xyz", "abc.rs").is_none());
    }

    #[test]
    fn fuzzy_score_empty_query_matches_all() {
        assert_eq!(fuzzy_score("", "anything.rs"), Some(0));
    }

    #[test]
    fn fuzzy_score_case_insensitive() {
        let score = fuzzy_score("readme", "README.md");
        assert!(score.is_some());
        assert_eq!(score.unwrap(), 0); // prefix match
    }

    #[test]
    fn fuzzy_score_prefers_prefix_over_subsequence() {
        let prefix_score = fuzzy_score("app", "app.rs").unwrap();
        let subseq_score = fuzzy_score("app", "src/mapper.rs").unwrap();
        assert!(prefix_score < subseq_score);
    }
}
