#![allow(dead_code)]

//! Local LLM server discovery and probing.

use serde::Deserialize;
use std::time::Duration;

/// Known local LLM server types.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalServerType {
    Ollama,
    LmStudio,
    LlamaCpp,
    Custom,
}

impl LocalServerType {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::LlamaCpp => "llama.cpp",
            Self::Custom => "Custom",
        }
    }

    pub fn default_address(&self) -> &'static str {
        match self {
            Self::Ollama => "http://localhost:11434",
            Self::LmStudio => "http://localhost:1234",
            Self::LlamaCpp => "http://localhost:8080",
            Self::Custom => "",
        }
    }
}

/// A discovered or configured local LLM server.
#[derive(Debug, Clone)]
pub struct LocalServer {
    pub server_type: LocalServerType,
    pub address: String,
    pub available: bool,
    pub models: Vec<String>,
}

/// Probe a server address and return available models.
pub async fn probe_server(address: &str, server_type: &LocalServerType) -> Option<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;

    match server_type {
        LocalServerType::Ollama => probe_ollama(&client, address).await,
        _ => probe_openai_compatible(&client, address).await,
    }
}

/// Probe Ollama's /api/tags endpoint.
async fn probe_ollama(client: &reqwest::Client, address: &str) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct OllamaResponse {
        models: Option<Vec<OllamaModel>>,
    }
    #[derive(Deserialize)]
    struct OllamaModel {
        name: String,
    }

    let url = format!("{}/api/tags", address.trim_end_matches('/'));
    let resp = client.get(&url).send().await.ok()?;
    let data: OllamaResponse = resp.json().await.ok()?;
    Some(
        data.models
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.name)
            .collect(),
    )
}

/// Probe OpenAI-compatible /v1/models endpoint (LM Studio, llama.cpp, Custom).
async fn probe_openai_compatible(client: &reqwest::Client, address: &str) -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct ModelsResponse {
        data: Option<Vec<ModelEntry>>,
    }
    #[derive(Deserialize)]
    struct ModelEntry {
        id: String,
    }

    let url = format!("{}/v1/models", address.trim_end_matches('/'));
    let resp = client.get(&url).send().await.ok()?;
    let data: ModelsResponse = resp.json().await.ok()?;
    Some(
        data.data
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect(),
    )
}

/// Discover all known local servers that are currently running.
/// Probes each default address and returns results.
pub async fn discover_local_servers() -> Vec<LocalServer> {
    let known = vec![
        (LocalServerType::Ollama, "http://localhost:11434"),
        (LocalServerType::LmStudio, "http://localhost:1234"),
        (LocalServerType::LlamaCpp, "http://localhost:8080"),
    ];

    let mut results = Vec::new();
    for (server_type, address) in known {
        let models = probe_server(address, &server_type).await;
        results.push(LocalServer {
            server_type,
            address: address.to_string(),
            available: models.is_some(),
            models: models.unwrap_or_default(),
        });
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_type_display_names() {
        assert_eq!(LocalServerType::Ollama.display_name(), "Ollama");
        assert_eq!(LocalServerType::LmStudio.display_name(), "LM Studio");
        assert_eq!(LocalServerType::LlamaCpp.display_name(), "llama.cpp");
        assert_eq!(LocalServerType::Custom.display_name(), "Custom");
    }

    #[test]
    fn server_type_default_addresses() {
        assert_eq!(
            LocalServerType::Ollama.default_address(),
            "http://localhost:11434"
        );
        assert_eq!(
            LocalServerType::LmStudio.default_address(),
            "http://localhost:1234"
        );
        assert_eq!(
            LocalServerType::LlamaCpp.default_address(),
            "http://localhost:8080"
        );
        assert_eq!(LocalServerType::Custom.default_address(), "");
    }

    #[tokio::test]
    async fn probe_nonexistent_server_returns_none() {
        let result = probe_server("http://localhost:59999", &LocalServerType::Custom).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn discover_returns_three_servers() {
        let servers = discover_local_servers().await;
        assert_eq!(servers.len(), 3);
    }
}
