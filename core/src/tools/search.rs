//! Pluggable search backend trait for the web_search tool.

use anyhow::Result;

/// A single search result returned by a backend.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Trait for pluggable search backends.
/// Uses native async fn in traits (edition 2024).
pub trait SearchBackend: Send + Sync {
    fn search(
        &self,
        query: &str,
        max_results: usize,
    ) -> impl std::future::Future<Output = Result<Vec<SearchResult>>> + Send;
    fn name(&self) -> &str;
}

/// Format search results into a string for the LLM.
pub fn format_results(results: &[SearchResult], query: &str) -> String {
    if results.is_empty() {
        return format!("No results found for: {query}");
    }
    let mut output = String::new();
    for (i, r) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. **{}**\n   {}\n   {}\n\n",
            i + 1,
            r.title,
            r.url,
            r.snippet,
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_results_empty() {
        let output = format_results(&[], "rust async");
        assert_eq!(output, "No results found for: rust async");
    }

    #[test]
    fn format_results_numbered() {
        let results = vec![
            SearchResult {
                title: "First Result".to_string(),
                url: "https://example.com/first".to_string(),
                snippet: "A snippet about the first result.".to_string(),
            },
            SearchResult {
                title: "Second Result".to_string(),
                url: "https://example.com/second".to_string(),
                snippet: "A snippet about the second result.".to_string(),
            },
        ];
        let output = format_results(&results, "test query");
        assert!(output.contains("1. **First Result**"));
        assert!(output.contains("https://example.com/first"));
        assert!(output.contains("A snippet about the first result."));
        assert!(output.contains("2. **Second Result**"));
        assert!(output.contains("https://example.com/second"));
        assert!(output.contains("A snippet about the second result."));
    }

    #[test]
    fn format_results_single_entry() {
        let results = vec![SearchResult {
            title: "Only Result".to_string(),
            url: "https://example.com".to_string(),
            snippet: "The only snippet.".to_string(),
        }];
        let output = format_results(&results, "query");
        assert!(output.starts_with("1. **Only Result**"));
        assert!(!output.contains("2."));
    }
}
