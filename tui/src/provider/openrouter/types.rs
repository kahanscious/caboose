//! OpenRouter-specific types — reuses OpenAI format with richer models response.

// Reuse OpenAI request/response types — format is identical
pub use crate::provider::openai::types::*;

// OpenRouter-specific models response (richer than OpenAI's)
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct OpenRouterModelsResponse {
    pub data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: Option<String>,
    pub context_length: Option<u32>,
    pub pricing: Option<OpenRouterPricing>,
    /// Parameters the model supports (e.g. "tools", "temperature").
    /// Present in OpenRouter's /models response.
    #[serde(default)]
    pub supported_parameters: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct OpenRouterPricing {
    /// Cost per input token as USD string (e.g. "0.000003")
    pub prompt: Option<String>,
    /// Cost per output token as USD string (e.g. "0.000015")
    pub completion: Option<String>,
}
