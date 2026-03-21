//! SearXNG search backend implementation.

use anyhow::{Result, anyhow};
use reqwest::Client;

use crate::tools::search::{SearchBackend, SearchResult};

/// Search backend for a self-hosted SearXNG instance.
pub struct SearxngBackend {
    pub base_url: String,
    pub user_agent: String,
    pub client: Client,
}

impl SearxngBackend {
    /// Create a new `SearxngBackend`.
    ///
    /// Builds a reqwest client with a 5-second timeout and the specified
    /// user-agent header.
    pub fn new(base_url: &str, user_agent: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .user_agent(user_agent)
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            user_agent: user_agent.to_string(),
            client,
        })
    }
}

impl SearchBackend for SearxngBackend {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let url = format!("{}/search", self.base_url);

        let response = self
            .client
            .get(&url)
            .query(&[("q", query), ("format", "json"), ("language", "en")])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "SearXNG returned HTTP {}",
                response.status()
            ));
        }

        let json: serde_json::Value = response.json().await?;

        let results = json["results"]
            .as_array()
            .ok_or_else(|| anyhow!("SearXNG response missing 'results' array"))?;

        let mapped: Vec<SearchResult> = results
            .iter()
            .filter_map(|r| {
                let title = r["title"].as_str()?.to_string();
                let url = r["url"].as_str()?.to_string();
                let snippet = r["content"].as_str().unwrap_or("").to_string();
                Some(SearchResult { title, url, snippet })
            })
            .take(max_results)
            .collect();

        Ok(mapped)
    }

    fn name(&self) -> &str {
        "searxng"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn searxng_parses_results() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    {
                        "title": "Rust Programming Language",
                        "url": "https://www.rust-lang.org",
                        "content": "A language empowering everyone."
                    },
                    {
                        "title": "Rust by Example",
                        "url": "https://doc.rust-lang.org/rust-by-example/",
                        "content": "Learn Rust with examples."
                    }
                ]
            })))
            .mount(&server)
            .await;

        let backend = SearxngBackend::new(&server.uri(), "test-agent").unwrap();
        let results = backend.search("rust", 10).await.unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[0].url, "https://www.rust-lang.org");
        assert_eq!(results[0].snippet, "A language empowering everyone.");
        assert_eq!(results[1].title, "Rust by Example");
        assert_eq!(results[1].url, "https://doc.rust-lang.org/rust-by-example/");
        assert_eq!(results[1].snippet, "Learn Rust with examples.");
    }

    #[tokio::test]
    async fn searxng_respects_max_results() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    { "title": "Result 1", "url": "https://a.com", "content": "A" },
                    { "title": "Result 2", "url": "https://b.com", "content": "B" },
                    { "title": "Result 3", "url": "https://c.com", "content": "C" }
                ]
            })))
            .mount(&server)
            .await;

        let backend = SearxngBackend::new(&server.uri(), "test-agent").unwrap();
        let results = backend.search("test", 2).await.unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Result 1");
        assert_eq!(results[1].title, "Result 2");
    }

    #[tokio::test]
    async fn searxng_handles_empty_results() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": []
            })))
            .mount(&server)
            .await;

        let backend = SearxngBackend::new(&server.uri(), "test-agent").unwrap();
        let results = backend.search("nothing", 10).await.unwrap();

        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn searxng_handles_server_error() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/search"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let backend = SearxngBackend::new(&server.uri(), "test-agent").unwrap();
        let result = backend.search("test", 10).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"), "expected 500 in error message, got: {err}");
    }

    #[tokio::test]
    async fn searxng_e2e_with_web_search_tool() {
        use crate::config::schema::ServiceConfig;

        let mock_server = MockServer::start().await;
        let body = serde_json::json!({
            "query": "rust memory safety",
            "number_of_results": 2,
            "results": [
                {
                    "title": "Memory Safety - The Rust Programming Language",
                    "url": "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
                    "content": "Rust's central feature is ownership.",
                    "engine": "google",
                    "score": 5.0
                },
                {
                    "title": "Rust (programming language) - Wikipedia",
                    "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                    "content": "Rust emphasizes performance, type safety, and concurrency.",
                    "engine": "bing",
                    "score": 3.2
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path_regex("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        let config = ServiceConfig {
            provider: "searxng".into(),
            api_key_env: None,
            enabled: true,
            base_url: Some(mock_server.uri()),
            user_agent: Some("Caboose/1.0 (SearchService)".into()),
            max_results: Some(10),
        };

        let result = crate::tools::web_search::execute(
            &serde_json::json!({"query": "rust memory safety"}),
            Some(&config),
        )
        .await
        .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("Memory Safety"));
        assert!(result.output.contains("rust-lang.org"));
        assert!(result.output.contains("Wikipedia"));
    }

    #[tokio::test]
    async fn searxng_handles_partial_results() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [
                    { "title": "Valid Result", "url": "https://valid.com", "content": "Valid" },
                    { "url": "https://notitle.com", "content": "No title here" },
                    { "title": "No URL result", "content": "No URL here" },
                    { "title": null, "url": null, "content": "Both null" }
                ]
            })))
            .mount(&server)
            .await;

        let backend = SearxngBackend::new(&server.uri(), "test-agent").unwrap();
        let results = backend.search("test", 10).await.unwrap();

        // Only the entry with both title and url should be included
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Valid Result");
        assert_eq!(results[0].url, "https://valid.com");
    }
}
