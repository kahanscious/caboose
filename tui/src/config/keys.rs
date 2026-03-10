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
