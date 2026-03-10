//! Model context window lookup — static table + runtime cache.
//!
//! Two-tier lookup:
//! 1. Built-in static table with prefix matching (works offline, covers well-known models)
//! 2. Runtime cache populated from provider API responses (covers everything else)
//!
//! Uses prefix matching so dated model IDs (e.g. claude-sonnet-4-20250514)
//! resolve to their family entry.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Built-in context window sizes for well-known models.
/// Format: (model_id_prefix, context_window)
const KNOWN_CONTEXT_WINDOWS: &[(&str, u32)] = &[
    // Anthropic
    ("claude-opus-4", 200_000),
    ("claude-sonnet-4", 200_000),
    ("claude-haiku-4", 200_000),
    ("claude-3-7-sonnet", 200_000),
    ("claude-3-5-sonnet", 200_000),
    ("claude-3-5-haiku", 200_000),
    ("claude-3-opus", 200_000),
    ("claude-3-sonnet", 200_000),
    ("claude-3-haiku", 200_000),
    // OpenAI
    ("gpt-4.1", 1_047_576),
    ("gpt-4o", 128_000),
    ("gpt-4-turbo", 128_000),
    ("o3", 200_000),
    ("o4-mini", 200_000),
    ("o1", 200_000),
    // Google
    ("gemini-2.5-pro", 1_048_576),
    ("gemini-2.5-flash", 1_048_576),
    ("gemini-2.0-flash", 1_048_576),
    ("gemini-1.5-pro", 2_097_152),
    ("gemini-1.5-flash", 1_048_576),
    // DeepSeek
    ("deepseek-chat", 65_536),
    ("deepseek-reasoner", 65_536),
    // Grok
    ("grok-3", 131_072),
    ("grok-2", 131_072),
    // Mistral
    ("mistral-large", 131_072),
    ("mistral-medium", 32_768),
    ("mistral-small", 32_768),
    ("codestral", 32_768),
    ("pixtral-large", 131_072),
    // Meta (via Groq, etc.)
    ("llama-3.3", 131_072),
    ("llama-3.1", 131_072),
    ("llama-3-", 8_192),
];

/// Runtime cache of context windows learned from provider APIs.
fn runtime_cache() -> &'static Mutex<HashMap<String, u32>> {
    static CACHE: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Store a model's context window in the runtime cache.
/// Called when provider APIs return model metadata.
#[allow(dead_code)]
pub fn cache_context_window(model_id: &str, context_window: u32) {
    if let Ok(mut cache) = runtime_cache().lock() {
        cache.insert(model_id.to_string(), context_window);
    }
}

/// Bulk-insert context windows from a list of ModelInfo.
pub fn cache_from_model_list(models: &[(String, Option<u32>)]) {
    if let Ok(mut cache) = runtime_cache().lock() {
        for (id, cw) in models {
            if let Some(cw) = cw {
                cache.insert(id.clone(), *cw);
            }
        }
    }
}

/// Look up a model's context window.
///
/// 1. Static table (prefix match, handles OpenRouter slash IDs)
/// 2. Runtime cache (exact match, populated from provider APIs)
pub fn context_window(model_id: &str) -> Option<u32> {
    // OpenRouter-style IDs: strip the provider prefix for static table lookup
    let effective = model_id.split_once('/').map(|(_, m)| m).unwrap_or(model_id);

    // 1. Static table — prefix match
    if let Some(cw) = KNOWN_CONTEXT_WINDOWS
        .iter()
        .find(|(prefix, _)| effective.starts_with(prefix))
        .map(|(_, cw)| *cw)
    {
        return Some(cw);
    }

    // 2. Runtime cache — exact match on full model ID
    if let Ok(cache) = runtime_cache().lock()
        && let Some(&cw) = cache.get(model_id)
    {
        return Some(cw);
    }

    None
}

/// Return the context window size, defaulting to 200_000 for unknown models.
pub fn context_window_or_default(model_id: &str) -> u32 {
    context_window(model_id).unwrap_or(200_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models() {
        assert_eq!(context_window("claude-sonnet-4-6"), Some(200_000));
        assert_eq!(context_window("claude-sonnet-4-20250514"), Some(200_000));
        assert_eq!(context_window("gpt-4o"), Some(128_000));
        assert_eq!(context_window("gpt-4o-mini"), Some(128_000));
        assert_eq!(context_window("gemini-2.0-flash"), Some(1_048_576));
        assert_eq!(context_window("deepseek-chat"), Some(65_536));
        assert_eq!(context_window("llama-3.3-70b-versatile"), Some(131_072));
    }

    #[test]
    fn openrouter_slash_ids() {
        assert_eq!(context_window("anthropic/claude-sonnet-4-6"), Some(200_000));
        assert_eq!(context_window("openai/gpt-4o"), Some(128_000));
        assert_eq!(context_window("google/gemini-2.0-flash"), Some(1_048_576));
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(context_window("totally-unknown-model"), None);
    }

    #[test]
    fn unknown_model_defaults_to_200k() {
        assert_eq!(context_window_or_default("totally-unknown-model"), 200_000);
    }

    #[test]
    fn runtime_cache_provides_fallback() {
        cache_context_window("arcee-ai/some-model:free", 131_072);
        assert_eq!(context_window("arcee-ai/some-model:free"), Some(131_072));
    }

    #[test]
    fn static_table_wins_over_cache() {
        // Even if cached with a different value, static table takes priority
        cache_context_window("gpt-4o", 999_999);
        assert_eq!(context_window("gpt-4o"), Some(128_000));
    }
}
