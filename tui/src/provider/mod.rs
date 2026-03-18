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

/// Thinking/reasoning level for models that support extended thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ThinkingMode {
    /// Thinking disabled — no reasoning tokens requested.
    #[default]
    Off = 0,
    /// Low reasoning effort.
    Low = 1,
    /// Medium reasoning effort (default when toggling on).
    Medium = 2,
    /// High reasoning effort.
    High = 3,
}

impl ThinkingMode {
    /// Toggle between Off and Medium. Non-medium levels toggle to Off.
    pub fn toggle(self) -> Self {
        match self {
            Self::Off => Self::Medium,
            _ => Self::Off,
        }
    }

    pub fn is_on(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Low,
            2 => Self::Medium,
            3 => Self::High,
            _ => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// All variants for use in pickers.
    pub const ALL: &[ThinkingMode] = &[Self::Off, Self::Low, Self::Medium, Self::High];
}

impl std::fmt::Display for ThinkingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod thinking_mode_tests {
    use super::ThinkingMode;

    #[test]
    fn toggle_off_to_medium() {
        assert_eq!(ThinkingMode::Off.toggle(), ThinkingMode::Medium);
    }

    #[test]
    fn toggle_medium_to_off() {
        assert_eq!(ThinkingMode::Medium.toggle(), ThinkingMode::Off);
    }

    #[test]
    fn toggle_low_to_off() {
        assert_eq!(ThinkingMode::Low.toggle(), ThinkingMode::Off);
    }

    #[test]
    fn toggle_high_to_off() {
        assert_eq!(ThinkingMode::High.toggle(), ThinkingMode::Off);
    }

    #[test]
    fn is_on() {
        assert!(!ThinkingMode::Off.is_on());
        assert!(ThinkingMode::Low.is_on());
        assert!(ThinkingMode::Medium.is_on());
        assert!(ThinkingMode::High.is_on());
    }

    #[test]
    fn from_u8_all_variants() {
        assert_eq!(ThinkingMode::from_u8(0), ThinkingMode::Off);
        assert_eq!(ThinkingMode::from_u8(1), ThinkingMode::Low);
        assert_eq!(ThinkingMode::from_u8(2), ThinkingMode::Medium);
        assert_eq!(ThinkingMode::from_u8(3), ThinkingMode::High);
        assert_eq!(ThinkingMode::from_u8(99), ThinkingMode::Off);
    }

    #[test]
    fn label_values() {
        assert_eq!(ThinkingMode::Off.label(), "off");
        assert_eq!(ThinkingMode::Low.label(), "low");
        assert_eq!(ThinkingMode::Medium.label(), "medium");
        assert_eq!(ThinkingMode::High.label(), "high");
    }

    #[test]
    fn display_impl() {
        assert_eq!(format!("{}", ThinkingMode::Medium), "medium");
    }
}

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
    pub supports_thinking: bool,
}

/// Trait that all LLM providers must implement to provide a unified interface for streaming chat completions.
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

    /// Set the thinking/reasoning mode for subsequent stream calls.
    /// Default implementation is a no-op for providers that don't support thinking.
    fn set_thinking_mode(&self, _mode: ThinkingMode) {}
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
            "xai" => {
                let api_key = self
                    .config
                    .keys
                    .get("xai")
                    .ok_or_else(|| {
                        anyhow::anyhow!("No API key for xai. Set XAI_API_KEY or add to config.")
                    })?
                    .to_string();
                let model = model.unwrap_or("grok-3").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.x.ai/v1".to_string())
                    .with_provider_name("xai".to_string())
                    .with_model_filter(Some(|id: &str| id.starts_with("grok-")));
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "together" => {
                let api_key = self
                    .config
                    .keys
                    .get("together")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for together. Set TOGETHER_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model
                    .unwrap_or("meta-llama/Llama-3.3-70B-Instruct-Turbo")
                    .to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.together.xyz/v1".to_string())
                    .with_provider_name("together".to_string())
                    .with_model_filter(None);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "fireworks" => {
                let api_key = self
                    .config
                    .keys
                    .get("fireworks")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for fireworks. Set FIREWORKS_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model
                    .unwrap_or("accounts/fireworks/models/llama-v3p3-70b-instruct")
                    .to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.fireworks.ai/inference/v1".to_string())
                    .with_provider_name("fireworks".to_string())
                    .with_model_filter(None);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "cerebras" => {
                let api_key = self
                    .config
                    .keys
                    .get("cerebras")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for cerebras. Set CEREBRAS_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("llama-3.3-70b").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.cerebras.ai/v1".to_string())
                    .with_provider_name("cerebras".to_string())
                    .with_model_filter(None);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "sambanova" => {
                let api_key = self
                    .config
                    .keys
                    .get("sambanova")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for sambanova. Set SAMBANOVA_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("Meta-Llama-3.3-70B-Instruct").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.sambanova.ai/v1".to_string())
                    .with_provider_name("sambanova".to_string())
                    .with_model_filter(None);
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "perplexity" => {
                let api_key = self
                    .config
                    .keys
                    .get("perplexity")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for perplexity. Set PERPLEXITY_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("sonar-pro").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.perplexity.ai".to_string())
                    .with_provider_name("perplexity".to_string())
                    .with_model_filter(Some(|id: &str| {
                        id.starts_with("sonar") || id.starts_with("llama-")
                    }));
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "cohere" => {
                let api_key = self
                    .config
                    .keys
                    .get("cohere")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for cohere. Set COHERE_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("command-r-plus").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://api.cohere.com/v2".to_string())
                    .with_provider_name("cohere".to_string())
                    .with_model_filter(Some(|id: &str| id.starts_with("command-")));
                if let Some(base_url) = provider_cfg.and_then(|c| c.base_url.as_deref()) {
                    provider = provider.with_base_url(base_url.to_string());
                }
                Ok(Box::new(retry::RetryProvider::new(Arc::new(provider))))
            }
            "qwen" | "dashscope" => {
                let api_key = self
                    .config
                    .keys
                    .get("qwen")
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No API key for qwen. Set DASHSCOPE_API_KEY or add to config."
                        )
                    })?
                    .to_string();
                let model = model.unwrap_or("qwen-plus").to_string();
                let mut provider = openai::OpenAiProvider::new(api_key, model)
                    .with_base_url("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string())
                    .with_provider_name("qwen".to_string())
                    .with_model_filter(Some(|id: &str| id.starts_with("qwen-")));
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
                "Unknown provider: '{other}'. Supported: anthropic, openai, gemini, openrouter, deepseek, groq, mistral, xai, together, fireworks, cerebras, sambanova, perplexity, cohere, qwen, ollama, lmstudio, llamacpp, custom"
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

    fn set_thinking_mode(&self, mode: ThinkingMode) {
        self.0.set_thinking_mode(mode);
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
        assert!(msg.contains("xai"));
        assert!(msg.contains("together"));
        assert!(msg.contains("fireworks"));
        assert!(msg.contains("cerebras"));
        assert!(msg.contains("sambanova"));
        assert!(msg.contains("perplexity"));
        assert!(msg.contains("cohere"));
        assert!(msg.contains("qwen"));
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
    fn xai_provider_has_correct_name_and_model() {
        let config = config_with_key("xai", "xai-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("xai"), None).unwrap();
        assert_eq!(provider.name(), "xai");
        assert_eq!(provider.model(), "grok-3");
    }

    #[test]
    fn xai_provider_respects_model_override() {
        let config = config_with_key("xai", "xai-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("xai"), Some("grok-3-mini"))
            .unwrap();
        assert_eq!(provider.model(), "grok-3-mini");
    }

    #[test]
    fn missing_xai_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("xai"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("XAI_API_KEY"));
    }

    #[test]
    fn together_provider_has_correct_name_and_model() {
        let config = config_with_key("together", "together-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("together"), None).unwrap();
        assert_eq!(provider.name(), "together");
        assert_eq!(provider.model(), "meta-llama/Llama-3.3-70B-Instruct-Turbo");
    }

    #[test]
    fn together_provider_respects_model_override() {
        let config = config_with_key("together", "together-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(
                Some("together"),
                Some("meta-llama/Llama-3.1-8B-Instruct-Turbo"),
            )
            .unwrap();
        assert_eq!(provider.model(), "meta-llama/Llama-3.1-8B-Instruct-Turbo");
    }

    #[test]
    fn missing_together_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("together"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("TOGETHER_API_KEY"));
    }

    #[test]
    fn fireworks_provider_has_correct_name_and_model() {
        let config = config_with_key("fireworks", "fw-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("fireworks"), None).unwrap();
        assert_eq!(provider.name(), "fireworks");
        assert_eq!(
            provider.model(),
            "accounts/fireworks/models/llama-v3p3-70b-instruct"
        );
    }

    #[test]
    fn fireworks_provider_respects_model_override() {
        let config = config_with_key("fireworks", "fw-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(
                Some("fireworks"),
                Some("accounts/fireworks/models/mixtral-8x7b-instruct"),
            )
            .unwrap();
        assert_eq!(
            provider.model(),
            "accounts/fireworks/models/mixtral-8x7b-instruct"
        );
    }

    #[test]
    fn missing_fireworks_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("fireworks"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("FIREWORKS_API_KEY"));
    }

    #[test]
    fn cerebras_provider_has_correct_name_and_model() {
        let config = config_with_key("cerebras", "csk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("cerebras"), None).unwrap();
        assert_eq!(provider.name(), "cerebras");
        assert_eq!(provider.model(), "llama-3.3-70b");
    }

    #[test]
    fn cerebras_provider_respects_model_override() {
        let config = config_with_key("cerebras", "csk-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("cerebras"), Some("llama-3.1-8b"))
            .unwrap();
        assert_eq!(provider.model(), "llama-3.1-8b");
    }

    #[test]
    fn missing_cerebras_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("cerebras"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("CEREBRAS_API_KEY"));
    }

    #[test]
    fn sambanova_provider_has_correct_name_and_model() {
        let config = config_with_key("sambanova", "snova-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("sambanova"), None).unwrap();
        assert_eq!(provider.name(), "sambanova");
        assert_eq!(provider.model(), "Meta-Llama-3.3-70B-Instruct");
    }

    #[test]
    fn sambanova_provider_respects_model_override() {
        let config = config_with_key("sambanova", "snova-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("sambanova"), Some("Meta-Llama-3.1-8B-Instruct"))
            .unwrap();
        assert_eq!(provider.model(), "Meta-Llama-3.1-8B-Instruct");
    }

    #[test]
    fn missing_sambanova_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("sambanova"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("SAMBANOVA_API_KEY"));
    }

    #[test]
    fn perplexity_provider_has_correct_name_and_model() {
        let config = config_with_key("perplexity", "pplx-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("perplexity"), None).unwrap();
        assert_eq!(provider.name(), "perplexity");
        assert_eq!(provider.model(), "sonar-pro");
    }

    #[test]
    fn perplexity_provider_respects_model_override() {
        let config = config_with_key("perplexity", "pplx-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("perplexity"), Some("sonar"))
            .unwrap();
        assert_eq!(provider.model(), "sonar");
    }

    #[test]
    fn missing_perplexity_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("perplexity"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("PERPLEXITY_API_KEY"));
    }

    #[test]
    fn cohere_provider_has_correct_name_and_model() {
        let config = config_with_key("cohere", "cohere-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("cohere"), None).unwrap();
        assert_eq!(provider.name(), "cohere");
        assert_eq!(provider.model(), "command-r-plus");
    }

    #[test]
    fn cohere_provider_respects_model_override() {
        let config = config_with_key("cohere", "cohere-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("cohere"), Some("command-r"))
            .unwrap();
        assert_eq!(provider.model(), "command-r");
    }

    #[test]
    fn missing_cohere_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("cohere"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("COHERE_API_KEY"));
    }

    #[test]
    fn qwen_provider_has_correct_name_and_model() {
        let config = config_with_key("qwen", "sk-qwen-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("qwen"), None).unwrap();
        assert_eq!(provider.name(), "qwen");
        assert_eq!(provider.model(), "qwen-plus");
    }

    #[test]
    fn qwen_provider_respects_model_override() {
        let config = config_with_key("qwen", "sk-qwen-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry
            .get_provider(Some("qwen"), Some("qwen-max"))
            .unwrap();
        assert_eq!(provider.model(), "qwen-max");
    }

    #[test]
    fn missing_qwen_key_returns_error() {
        let config = Config::default();
        let registry = ProviderRegistry::new(&config);
        let result = registry.get_provider(Some("qwen"), None);
        let msg = result.err().expect("expected error").to_string();
        assert!(msg.contains("DASHSCOPE_API_KEY"));
    }

    #[test]
    fn dashscope_alias_resolves_to_qwen() {
        let config = config_with_key("qwen", "sk-qwen-test");
        let registry = ProviderRegistry::new(&config);
        let provider = registry.get_provider(Some("dashscope"), None).unwrap();
        assert_eq!(provider.name(), "qwen");
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
            supports_thinking: true,
        };
        assert!(vision_model.supports_vision);

        let text_only_model = ModelInfo {
            id: "o1-preview".into(),
            name: "o1-preview".into(),
            context_window: Some(128_000),
            supports_tools: false,
            supports_vision: false,
            supports_thinking: false,
        };
        assert!(!text_only_model.supports_vision);
    }
}
