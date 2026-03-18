//! API key management — environment variables and config-based keys.

use serde::{Deserialize, Serialize};

/// API keys for LLM providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeys {
    pub anthropic: Option<String>,
    pub openai: Option<String>,
    pub google: Option<String>,
    pub deepseek: Option<String>,
    pub groq: Option<String>,
    pub openrouter: Option<String>,
    pub mistral: Option<String>,
    pub xai: Option<String>,
    pub together: Option<String>,
    pub fireworks: Option<String>,
    pub cerebras: Option<String>,
    pub sambanova: Option<String>,
    pub perplexity: Option<String>,
    pub cohere: Option<String>,
    pub qwen: Option<String>,
}

impl ApiKeys {
    /// Load keys from environment variables (does not overwrite existing).
    pub fn load_from_env(&mut self) {
        load_env(&mut self.anthropic, "ANTHROPIC_API_KEY");
        load_env(&mut self.openai, "OPENAI_API_KEY");
        load_env(&mut self.google, "GOOGLE_API_KEY");
        load_env(&mut self.google, "GEMINI_API_KEY");
        load_env(&mut self.deepseek, "DEEPSEEK_API_KEY");
        load_env(&mut self.groq, "GROQ_API_KEY");
        load_env(&mut self.openrouter, "OPENROUTER_API_KEY");
        load_env(&mut self.mistral, "MISTRAL_API_KEY");
        load_env(&mut self.xai, "XAI_API_KEY");
        load_env(&mut self.together, "TOGETHER_API_KEY");
        load_env(&mut self.fireworks, "FIREWORKS_API_KEY");
        load_env(&mut self.cerebras, "CEREBRAS_API_KEY");
        load_env(&mut self.sambanova, "SAMBANOVA_API_KEY");
        load_env(&mut self.perplexity, "PERPLEXITY_API_KEY");
        load_env(&mut self.cohere, "COHERE_API_KEY");
        load_env(&mut self.qwen, "DASHSCOPE_API_KEY");
    }

    /// Merge another keys set into this one (other takes precedence).
    pub fn merge(&mut self, other: ApiKeys) {
        merge_opt(&mut self.anthropic, other.anthropic);
        merge_opt(&mut self.openai, other.openai);
        merge_opt(&mut self.google, other.google);
        merge_opt(&mut self.deepseek, other.deepseek);
        merge_opt(&mut self.groq, other.groq);
        merge_opt(&mut self.openrouter, other.openrouter);
        merge_opt(&mut self.mistral, other.mistral);
        merge_opt(&mut self.xai, other.xai);
        merge_opt(&mut self.together, other.together);
        merge_opt(&mut self.fireworks, other.fireworks);
        merge_opt(&mut self.cerebras, other.cerebras);
        merge_opt(&mut self.sambanova, other.sambanova);
        merge_opt(&mut self.perplexity, other.perplexity);
        merge_opt(&mut self.cohere, other.cohere);
        merge_opt(&mut self.qwen, other.qwen);
    }

    /// Set a key for a provider by name.
    pub fn set(&mut self, provider: &str, key: String) {
        match provider {
            "anthropic" => self.anthropic = Some(key),
            "openai" => self.openai = Some(key),
            "google" | "gemini" => self.google = Some(key),
            "deepseek" => self.deepseek = Some(key),
            "groq" => self.groq = Some(key),
            "openrouter" => self.openrouter = Some(key),
            "mistral" => self.mistral = Some(key),
            "xai" => self.xai = Some(key),
            "together" => self.together = Some(key),
            "fireworks" => self.fireworks = Some(key),
            "cerebras" => self.cerebras = Some(key),
            "sambanova" => self.sambanova = Some(key),
            "perplexity" => self.perplexity = Some(key),
            "cohere" => self.cohere = Some(key),
            "qwen" | "dashscope" => self.qwen = Some(key),
            _ => {}
        }
    }

    /// Load keys from an AuthStore (takes precedence, overwrites existing).
    pub fn load_from_auth(&mut self, store: &crate::config::auth::AuthStore) {
        let providers = [
            "anthropic",
            "openai",
            "google",
            "deepseek",
            "groq",
            "openrouter",
            "mistral",
            "xai",
            "together",
            "fireworks",
            "cerebras",
            "sambanova",
            "perplexity",
            "cohere",
            "qwen",
        ];
        for &p in &providers {
            if let Some(key) = store.get(p) {
                self.set(p, key.to_string());
            }
        }
    }

    /// Remove a key for a provider by name.
    pub fn clear(&mut self, provider: &str) {
        match provider {
            "anthropic" => self.anthropic = None,
            "openai" => self.openai = None,
            "google" | "gemini" => self.google = None,
            "deepseek" => self.deepseek = None,
            "groq" => self.groq = None,
            "openrouter" => self.openrouter = None,
            "mistral" => self.mistral = None,
            "xai" => self.xai = None,
            "together" => self.together = None,
            "fireworks" => self.fireworks = None,
            "cerebras" => self.cerebras = None,
            "sambanova" => self.sambanova = None,
            "perplexity" => self.perplexity = None,
            "cohere" => self.cohere = None,
            "qwen" | "dashscope" => self.qwen = None,
            _ => {}
        }
    }

    /// Get key for a provider by name.
    pub fn get(&self, provider: &str) -> Option<&str> {
        match provider {
            "anthropic" => self.anthropic.as_deref(),
            "openai" => self.openai.as_deref(),
            "google" | "gemini" => self.google.as_deref(),
            "deepseek" => self.deepseek.as_deref(),
            "groq" => self.groq.as_deref(),
            "openrouter" => self.openrouter.as_deref(),
            "mistral" => self.mistral.as_deref(),
            "xai" => self.xai.as_deref(),
            "together" => self.together.as_deref(),
            "fireworks" => self.fireworks.as_deref(),
            "cerebras" => self.cerebras.as_deref(),
            "sambanova" => self.sambanova.as_deref(),
            "perplexity" => self.perplexity.as_deref(),
            "cohere" => self.cohere.as_deref(),
            "qwen" | "dashscope" => self.qwen.as_deref(),
            _ => None,
        }
    }
}

fn load_env(target: &mut Option<String>, var: &str) {
    if target.is_none()
        && let Ok(val) = std::env::var(var)
        && !val.is_empty()
    {
        *target = Some(val);
    }
}

fn merge_opt(target: &mut Option<String>, source: Option<String>) {
    if source.is_some() {
        *target = source;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xai_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("xai").is_none());
        keys.set("xai", "xai-test".to_string());
        assert_eq!(keys.get("xai"), Some("xai-test"));
        keys.clear("xai");
        assert!(keys.get("xai").is_none());
    }

    #[test]
    fn together_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("together").is_none());
        keys.set("together", "together-test".to_string());
        assert_eq!(keys.get("together"), Some("together-test"));
        keys.clear("together");
        assert!(keys.get("together").is_none());
    }

    #[test]
    fn fireworks_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("fireworks").is_none());
        keys.set("fireworks", "fw-test".to_string());
        assert_eq!(keys.get("fireworks"), Some("fw-test"));
        keys.clear("fireworks");
        assert!(keys.get("fireworks").is_none());
    }

    #[test]
    fn cerebras_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("cerebras").is_none());
        keys.set("cerebras", "csk-test".to_string());
        assert_eq!(keys.get("cerebras"), Some("csk-test"));
        keys.clear("cerebras");
        assert!(keys.get("cerebras").is_none());
    }

    #[test]
    fn sambanova_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("sambanova").is_none());
        keys.set("sambanova", "snova-test".to_string());
        assert_eq!(keys.get("sambanova"), Some("snova-test"));
        keys.clear("sambanova");
        assert!(keys.get("sambanova").is_none());
    }

    #[test]
    fn perplexity_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("perplexity").is_none());
        keys.set("perplexity", "pplx-test".to_string());
        assert_eq!(keys.get("perplexity"), Some("pplx-test"));
        keys.clear("perplexity");
        assert!(keys.get("perplexity").is_none());
    }

    #[test]
    fn cohere_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("cohere").is_none());
        keys.set("cohere", "cohere-test".to_string());
        assert_eq!(keys.get("cohere"), Some("cohere-test"));
        keys.clear("cohere");
        assert!(keys.get("cohere").is_none());
    }

    #[test]
    fn qwen_key_roundtrip() {
        let mut keys = ApiKeys::default();
        assert!(keys.get("qwen").is_none());
        keys.set("qwen", "sk-qwen-test".to_string());
        assert_eq!(keys.get("qwen"), Some("sk-qwen-test"));
        keys.clear("qwen");
        assert!(keys.get("qwen").is_none());
    }
}
