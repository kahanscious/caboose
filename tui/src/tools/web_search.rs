//! Web search tool — search the web via external search providers.

use anyhow::Result;
use serde_json::Value;

use crate::agent::tools::ToolResult;

/// Execute a web search.
///
/// `provider` — which search backend to use (e.g. "tavily").
/// `api_key` — the API key for the provider.
pub async fn execute(input: &Value, provider: &str, api_key: &str) -> Result<ToolResult> {
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

    match provider {
        "tavily" => execute_tavily(query, api_key).await,
        _ => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!(
                "Unsupported web search provider: '{provider}'. Currently supported: tavily"
            ),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}

async fn execute_tavily(query: &str, api_key: &str) -> Result<ToolResult> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let body = serde_json::json!({
        "query": query,
        "search_depth": "basic",
        "max_results": 5,
        "include_answer": true,
    });

    let response = match client
        .post("https://api.tavily.com/search")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!("Web search request failed: {e}"),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Tavily API error ({status}): {text}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        });
    }

    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let mut output = String::new();

    // Include the AI-generated answer if present
    if let Some(answer) = json["answer"].as_str() {
        output.push_str(&format!("**Answer:** {answer}\n\n---\n\n"));
    }

    // Format individual results
    if let Some(results) = json["results"].as_array() {
        if results.is_empty() {
            output.push_str(&format!("No results found for: {query}"));
        } else {
            for (i, r) in results.iter().enumerate() {
                let title = r["title"].as_str().unwrap_or("Untitled");
                let url = r["url"].as_str().unwrap_or("");
                let content = r["content"].as_str().unwrap_or("");
                output.push_str(&format!(
                    "{}. **{}**\n   {}\n   {}\n\n",
                    i + 1,
                    title,
                    url,
                    content
                ));
            }
        }
    } else {
        output.push_str(&format!("No results found for: {query}"));
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_query_returns_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(execute(&serde_json::json!({}), "tavily", "fake-key"))
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("query"));
    }

    #[test]
    fn unsupported_provider_returns_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(execute(
                &serde_json::json!({"query": "test"}),
                "unknown_provider",
                "fake-key",
            ))
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unsupported"));
    }
}
