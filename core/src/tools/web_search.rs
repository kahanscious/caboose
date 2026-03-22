//! Web search tool — search the web via external search providers.

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::config::schema::ServiceConfig;
use crate::tools::ToolResult;
use crate::tools::search::{SearchBackend, format_results};
use crate::tools::search_searxng::SearxngBackend;
use crate::tools::search_tavily::TavilyBackend;

/// Enum dispatch to avoid the `Box<dyn SearchBackend>` object-safety issue
/// (the trait uses RPITIT / `impl Future` in return position).
enum Backend {
    Searxng(SearxngBackend),
    Tavily(TavilyBackend),
}

impl Backend {
    async fn search(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<crate::tools::search::SearchResult>> {
        match self {
            Backend::Searxng(b) => b.search(query, max_results).await,
            Backend::Tavily(b) => b.search(query, max_results).await,
        }
    }
}

/// Build a backend from a `ServiceConfig`.
fn build_backend(config: &ServiceConfig) -> Result<Backend> {
    match config.provider.as_str() {
        "searxng" => {
            let base_url = config.base_url.as_deref().ok_or_else(|| {
                anyhow!("searxng backend requires base_url in [services.web_search]")
            })?;
            let user_agent = config
                .user_agent
                .as_deref()
                .unwrap_or("Caboose/1.0 (SearchService)");
            let backend = SearxngBackend::new(base_url, user_agent)?;
            Ok(Backend::Searxng(backend))
        }
        "tavily" => {
            let key_env = config.api_key_env.as_deref().unwrap_or("TAVILY_API_KEY");
            let api_key = std::env::var(key_env).unwrap_or_default();
            let base_url = config.base_url.as_deref();
            Ok(Backend::Tavily(TavilyBackend::new(&api_key, base_url)))
        }
        other => Err(anyhow!(
            "Unsupported web search provider: '{other}'. Supported: searxng, tavily"
        )),
    }
}

/// Execute a web search.
pub async fn execute(input: &Value, service_config: Option<&ServiceConfig>) -> Result<ToolResult> {
    let query = match input["query"].as_str() {
        Some(q) if !q.trim().is_empty() => q.trim(),
        _ => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: "Missing required parameter: 'query'".to_string(),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

    let config = match service_config {
        Some(c) => c,
        None => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: "Web search is not configured. Add a [services.web_search] section to \
                         your config.toml:\n\n\
                         [services.web_search]\n\
                         provider = \"tavily\"\n\
                         api_key_env = \"TAVILY_API_KEY\"\n\n\
                         Or for a self-hosted SearXNG instance:\n\n\
                         [services.web_search]\n\
                         provider = \"searxng\"\n\
                         base_url = \"https://your-searxng-instance.example.com\""
                    .to_string(),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

    let backend = match build_backend(config) {
        Ok(b) => b,
        Err(e) => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!("Failed to build search backend: {e}"),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

    let max_results = config.max_results.unwrap_or(5);

    match backend.search(query, max_results).await {
        Ok(results) => {
            let output = format_results(&results, query);
            Ok(ToolResult {
                tool_use_id: String::new(),
                output,
                is_error: false,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            })
        }
        Err(e) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Web search failed: {e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_query_returns_error() {
        let result = execute(&serde_json::json!({}), None).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("query"));
    }

    #[tokio::test]
    async fn no_backend_configured_returns_setup_instructions() {
        let result = execute(&serde_json::json!({"query": "rust"}), None)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("[services.web_search]"));
    }

    #[tokio::test]
    async fn unsupported_provider_returns_error() {
        let config = ServiceConfig {
            provider: "unknown_provider".into(),
            api_key_env: None,
            enabled: true,
            base_url: None,
            user_agent: None,
            max_results: None,
        };
        let result = execute(&serde_json::json!({"query": "test"}), Some(&config))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unsupported") || result.output.contains("Failed to build"));
    }
}
