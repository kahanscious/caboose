//! Tavily search backend implementation.

use anyhow::{Result, anyhow};
use reqwest::Client;

use crate::tools::search::{SearchBackend, SearchResult};

/// Search backend for the Tavily API.
pub struct TavilyBackend {
    pub base_url: String,
    pub api_key: String,
    pub client: Client,
}

impl TavilyBackend {
    /// Create a new `TavilyBackend`.
    ///
    /// `base_url` defaults to `"https://api.tavily.com"` when `None` is
    /// passed.  Providing an override enables wiremock-based testing.
    pub fn new(api_key: &str, base_url: Option<&str>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url
                .unwrap_or("https://api.tavily.com")
                .trim_end_matches('/')
                .to_string(),
            api_key: api_key.to_string(),
            client,
        }
    }
}

impl SearchBackend for TavilyBackend {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let url = format!("{}/search", self.base_url);

        let body = serde_json::json!({
            "query": query,
            "search_depth": "basic",
            "max_results": max_results,
            "include_answer": true,
        });

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow!("Tavily API error ({status}): {text}"));
        }

        let json: serde_json::Value = response.json().await?;

        let mut results: Vec<SearchResult> = Vec::new();

        // If an AI answer is present, include it as the first result.
        if let Some(answer) = json["answer"].as_str() {
            if !answer.is_empty() {
                results.push(SearchResult {
                    title: "AI Answer".to_string(),
                    url: String::new(),
                    snippet: answer.to_string(),
                });
            }
        }

        if let Some(arr) = json["results"].as_array() {
            for r in arr {
                let title = r["title"].as_str().unwrap_or("Untitled").to_string();
                let url = r["url"].as_str().unwrap_or("").to_string();
                let snippet = r["content"].as_str().unwrap_or("").to_string();
                results.push(SearchResult { title, url, snippet });
            }
        }

        Ok(results)
    }

    fn name(&self) -> &str {
        "tavily"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn tavily_backend_name() {
        let backend = TavilyBackend::new("test-key", None);
        assert_eq!(backend.name(), "tavily");
    }

    #[tokio::test]
    async fn tavily_parses_results() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "answer": "Rust is a systems programming language focused on safety.",
                "results": [
                    {
                        "title": "The Rust Programming Language",
                        "url": "https://www.rust-lang.org",
                        "content": "Rust empowers everyone to build reliable software."
                    }
                ]
            })))
            .mount(&server)
            .await;

        let backend = TavilyBackend::new("test-key", Some(&server.uri()));
        let results = backend.search("rust programming language", 5).await.unwrap();

        // Expect 2 results: AI Answer first, then the one result entry.
        assert_eq!(results.len(), 2);

        let ai = &results[0];
        assert_eq!(ai.title, "AI Answer");
        assert_eq!(
            ai.snippet,
            "Rust is a systems programming language focused on safety."
        );

        let first = &results[1];
        assert_eq!(first.title, "The Rust Programming Language");
        assert_eq!(first.url, "https://www.rust-lang.org");
        assert_eq!(first.snippet, "Rust empowers everyone to build reliable software.");
    }
}
