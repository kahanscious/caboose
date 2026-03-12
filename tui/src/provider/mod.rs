//! Provider layer — LLM provider trait, registry, and implementations.

pub mod anthropic;
pub mod catalog;
pub mod error;
pub mod gemini;
pub mod local;
pub mod models_dev;
pub mod openai;
pub mod openrouter;
pub mod pricing;
pub mod retry;

use anyhow::{Result, bail};
use futures::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::config::Config;

/// Events emitted by provider streams — the universal output format.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum StreamEvent {
    /// Partial text content from the model.
    TextDelta(String),
    /// Partial thinking/reasoning content (Anthropic extended thinking, OpenAI reasoning).
    ThinkingDelta(String),
    /// A complete tool call — arguments have been fully buffered.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Stream complete — includes token usage.
    Done {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        cache_read_tokens: Option<u32>,
        cache_creation_tokens: Option<u32>,
    },
    /// Stream error.
    Error(String),
    /// Structured provider error with classification.
    ProviderError {
        category: crate::provider::error::ErrorCategory,
        provider: String,
        message: String,
        hint: Option<String>,
    },
}

/// Information about an available model from a provider.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_window: Option<u32>,
    pub supports_tools: bool,
    pub supports_vision: bool,
}

/// Trait that all LLM providers must implement.
pub trait Provider: Send + Sync {
    /// Stream a chat completion response.
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send + 'static>>;

    /// Provider display name.
    fn name(&self) -> &str;

    /// Model identifier.
    fn model(&self) -> &str;

    /// Fetch available models from this provider's API.
    fn list_models(&self) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>>> + Send + '_>>;
}

/// A chat message in the conversation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub role: String,
    pub content: serde_json::Value,
}

/// A tool definition sent to the provider.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Registry of available providers — creates providers from config.
pub struct ProviderRegistry {
    config: Config,
}

impl ProviderRegistry {
    pub fn new(config: &Config) -> Self {
        Self {
            config: config.clone(),
        }
    }

    /// Get a provider by name, or the default provider.
    pub fn get_provider(
        &self,
        name: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<Box<dyn Provider>> {
        let provider_name = name
            .or(self.config.default_provider.as_deref())
            .unwrap_or("anthropic");

        // Model resolution: CLI override > per-provider config > global default
        let provider_cfg = self.config.providers.get(provider_name);
        let model = model_override
            .or(provider_cfg.and_then(|c| c.model.as_deref()))
            .or(self.config.default_model.as_deref());

        match provider_name {
            "anthropic" => {
                let api_key = self
                    .config
                    .keys
                    .get("anthropic")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for anthropic. Set ANTHROPIC_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("claude-sonnet-4-6").to_string();
                let provider = anthropic::AnthropicProvider::new(api_key, model);
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "openai" => {
                let api_key = self
                    .config
                    .keys
                    .get("openai")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for openai. Set OPENAI_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("gpt-4o").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "gemini" | "google" => {
                let api_key = self
                    .config
                    .keys
                    .get("gemini")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for gemini. Set GEMINI_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("gemini-2.0-flash").to_string();
                let mut provider = gemini::GeminiProvider::new(api_key, model);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "openrouter" => {
                let api_key = self
                    .config
                    .keys
                    .get("openrouter")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for openrouter. Set OPENROUTER_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("anthropic/claude-sonnet-4.6").to_string();
                let mut provider = openrouter::OpenRouterProvider::new(api_key, model);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "deepseek" => {
                let api_key = self
                    .config
                    .keys
                    .get("deepseek")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for deepseek. Set DEEPSEEK_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("deepseek-chat").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.deepseek.com".to_string())
                    .with_provider_name("deepseek".to_string())
                    .with_model_filter(Some(|id: &str| id.starts_with("deepseek-")));
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "groq" => {
                let api_key = self
                    .config
                    .keys
                    .get("groq")
                    .ok_or_else(|| {
                        anyhow::anyhow!("No API key for groq. Set GROQ_API_KEY or add to config.")
                    })?
                    .to_string();
                let model = model.unwrap_or("llama-3.3-70b-versatile").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.groq.com/openai/v1".to_string())
                    .with_provider_name("groq".to_string())
                    .with_model_filter(None);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "mistral" => {
                let api_key = self
                    .config
                    .keys
                    .get("mistral")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for mistral. Set MISTRAL_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("mistral-large-latest").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.mistral.ai/v1".to_string())
                    .with_provider_name("mistral".to_string())
                    .with_model_filter(Some(|id: &str| {
                        id.starts_with("mistral-")
                            || id.starts_with("codestral-")
                            || id.starts_with("pixtral-")
                    }));
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "ollama" | "lmstudio" | "llamacpp" | "custom" => {
                let local_cfg = self.config.local_providers.get(provider_name);

                let address =
                    local_cfg
                        .map(|c| c.address.clone())
                        .unwrap_or_else(|| match provider_name {
                            "ollama" => "http://localhost:11434".to_string(),
                            "lmstudio" => "http://localhost:1234".to_string(),
                            "llamacpp" => "http://localhost:8080".to_string(),
                            _ => String::new(),
                        });

                if address.is_empty() {
                    bail!(
                        "Custom provider requires an address. Configure it in settings or use /connect."
                    );
                }

                // Ollama needs /v1 suffix for OpenAI compatibility mode
                let base_url = if provider_name == "ollama" {
                    format!("{}/v1", address.trim_end_matches('/'))
                } else {
                    address
                };

                // Model resolution: CLI override > local config > provider config > "default"
                let model_name = model
                    .or(local_cfg.and_then(|c| c.model.as_deref()))
                    .unwrap_or("default")
                    .to_string();

                let provider = openai::OpenAiProvider::new("local".to_string(), model_name)
                    .with_base_url(base_url)
                    .with_provider_name(provider_name.to_string());

                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            other => bail!(
                "Unknown provider: '{other}'. Supported: anthropic, openai, gemini, openrouter, deepseek, groq, mistral, ollama, lmstudio, llamacpp, custom"
            ),
        }
    }

    /// Like `get_provider` but returns an `Arc<dyn Provider + Send + Sync>`
    /// suitable for sharing across async tasks.
    pub fn get_provider_arc(
        &self,
        name: Option<&str>,
        model_override: Option<&str>,
    ) -> Result<Arc<dyn Provider + Send + Sync>> {
        let boxed = self.get_provider(name, model_override)?;
        Ok(Arc::from(BoxedProvider(boxed)))
    }
}

/// Thin wrapper that lets a `Box<dyn Provider>` be placed behind an `Arc`.
struct BoxedProvider(Box<dyn Provider>);

impl Provider for BoxedProvider {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send + 'static>> {
        self.0.stream(messages, tools)
    }

    fn name(&self) -> &str {
        self.0.name()
    }

    fn model(&self) -> &str {
        self.0.model()
    }

    fn list_models(&self) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>>> + Send + '_>> {
        self.0.list_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a Config with a single key set for testing.
    fn config_with_key(provider: &str, key: &str) -> Config {
        let mut config = Config::default();
        config.keys.set(provider, key.to_string());
        config
    }

    #[test]
    fn deepseek_provider_has_correct_name_and_model() {
        let config = config_with_key("deepseek", "sk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("deepseek"), None).unwrap();
        assert_eq!(provider.name(), "deepseek");
        assert_eq!(provider.model(), "deepseek-chat");
    }

    #[test]
    fn deepseek_provider_respects_model_override() {
        let config = config_with_key("deepseek", "sk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("deepseek"), Some("deepseek-reasoner"))
            .unwrap();
        assert_eq!(provider.model(), "deepseek-reasoner");
    }

    #[test]
    fn groq_provider_has_correct_name_and_model() {
        let config = config_with_key("groq", "gsk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("groq"), None).unwrap();
        assert_eq!(provider.name(), "groq");
        assert_eq!(provider.model(), "llama-3.3-70b-versatile");
    }

    #[test]
    fn groq_provider_respects_model_override() {
        let config = config_with_key("groq", "gsk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("groq"), Some("llama-3.1-8b-instant"))
            .unwrap();
        assert_eq!(provider.model(), "llama-3.1-8b-instant");
    }

    #[test]
    fn unknown_provider_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("not-a-provider"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("deepseek"));
        assert!(msg.contains("groq"));
        assert!(msg.contains("mistral"));
    }

    #[test]
    fn missing_deepseek_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("deepseek"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("DEEPSEEK_API_KEY"));
    }

    #[test]
    fn missing_groq_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("groq"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("GROQ_API_KEY"));
    }

    #[test]
    fn mistral_provider_has_correct_name_and_model() {
        let config = config_with_key("mistral", "sk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("mistral"), None).unwrap();
        assert_eq!(provider.name(), "mistral");
        assert_eq!(provider.model(), "mistral-large-latest");
    }

    #[test]
    fn mistral_provider_respects_model_override() {
        let config = config_with_key("mistral", "sk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("mistral"), Some("codestral-latest"))
            .unwrap();
        assert_eq!(provider.model(), "codestral-latest");
    }

    #[test]
    fn missing_mistral_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("mistral"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("MISTRAL_API_KEY"));
    }

    #[test]
    fn context_window_defaults_for_unknown_model() {
        // Unknown model should fall back to 200_000
        assert_eq!(
            models_dev::context_window_or_default("unknown-model"),
            200_000
        );
    }

    #[test]
    fn model_info_supports_vision_field() {
        let vision_model = ModelInfo {
            id: "claude-sonnet-4-6".into(),
            name: "Claude Sonnet 4.6".into(),
            context_window: Some(200_000),
            supports_tools: true,
            supports_vision: true,
        };
        assert!(vision_model.supports_vision);

        let text_only_model = ModelInfo {
            id: "o1-preview".into(),
            name: "o1-preview".into(),
            context_window: Some(128_000),
            supports_tools: false,
            supports_vision: false,
        };
        assert!(!text_only_model.supports_vision);
    }
}
