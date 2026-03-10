//! Grep tool — regex search across files.

use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use crate::agent::tools::ToolResult;

/// Search file contents with a regex pattern.
pub async fn execute(input: &Value) -> Result<ToolResult> {
    let pattern = input["pattern"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'pattern'"))?;
    let path = input["path"].as_str().unwrap_or(".");
    let include = input["include"].as_str();

    let re = match regex::Regex::new(pattern) {
        Ok(re) => re,
        Err(e) => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!("Invalid regex: {e}"),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

    let mut results = Vec::new();
    search_dir(Path::new(path), &re, include, &mut results).await;

    Ok(ToolResult {
        tool_use_id: String::new(),
        output: if results.is_empty() {
            "No matches found".to_string()
        } else {
            results.join("\n")
        },
        is_error: false,
        tool_name: None,
        file_path: None,
        files_modified: vec![],
        lines_added: 0,
        lines_removed: 0,
    })
}

async fn search_dir(
    dir: &Path,
    re: &regex::Regex,
    include: Option<&str>,
    results: &mut Vec<String>,
) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        // Skip hidden directories
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'))
        {
            continue;
        }

        if path.is_dir() {
            Box::pin(search_dir(&path, re, include, results)).await;
        } else if path.is_file() {
            // Apply include filter
            if let Some(glob_pattern) = include {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !glob::Pattern::new(glob_pattern)
                    .map(|p| p.matches(name))
                    .unwrap_or(false)
                {
                    continue;
                }
            }

            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        results.push(format!("{}:{}: {}", path.display(), i + 1, line));
                        if results.len() >= 500 {
                            return;
                        }
                    }
                }
            }
        }
    }
}
