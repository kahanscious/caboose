//! Per-provider, per-model pricing for cost estimation.

use std::collections::HashMap;

/// Pricing for a single model (USD per million tokens).
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_m: f64,
    pub output_per_m: f64,
    /// Cached input read rate (per M tokens). Defaults to 10% of input rate.
    pub cache_read_per_m: f64,
    /// Cached input creation rate (per M tokens). Defaults to 125% of input rate.
    pub cache_creation_per_m: f64,
}

impl ModelPricing {
    /// Create pricing with default cache rates (10% read, 125% creation).
    pub fn standard(input_per_m: f64, output_per_m: f64) -> Self {
        Self {
            input_per_m,
            output_per_m,
            cache_read_per_m: input_per_m * 0.1,
            cache_creation_per_m: input_per_m * 1.25,
        }
    }

    /// Create pricing with explicit cache rates.
    fn with_cache(
        input_per_m: f64,
        output_per_m: f64,
        cache_read_per_m: f64,
        cache_creation_per_m: f64,
    ) -> Self {
        Self {
            input_per_m,
            output_per_m,
            cache_read_per_m,
            cache_creation_per_m,
        }
    }
}

/// Registry of known model pricing. Populated at startup with static
/// entries; OpenRouter entries added dynamically from API response.
#[derive(Debug, Clone)]
pub struct PricingRegistry {
    models: HashMap<String, ModelPricing>,
}

impl Default for PricingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PricingRegistry {
    /// Create registry pre-populated with known model pricing.
    pub fn new() -> Self {
        let mut models = HashMap::new();

        // Anthropic (explicit cache rates from docs)
        models.insert(
            "claude-sonnet-4-6".into(),
            ModelPricing::with_cache(3.0, 15.0, 0.3, 3.75),
        );
        models.insert(
            "claude-sonnet-4-20250514".into(),
            ModelPricing::with_cache(3.0, 15.0, 0.3, 3.75),
        );
        models.insert(
            "claude-haiku-4-5-20251001".into(),
            ModelPricing::with_cache(0.8, 4.0, 0.08, 1.0),
        );
        models.insert(
            "claude-opus-4-6".into(),
            ModelPricing::with_cache(15.0, 75.0, 1.5, 18.75),
        );

        // OpenAI
        models.insert("gpt-4o".into(), ModelPricing::standard(2.5, 10.0));
        models.insert("gpt-4o-mini".into(), ModelPricing::standard(0.15, 0.6));
        models.insert("o3".into(), ModelPricing::standard(10.0, 40.0));
        models.insert("o4-mini".into(), ModelPricing::standard(1.1, 4.4));

        // Gemini
        models.insert("gemini-2.0-flash".into(), ModelPricing::standard(0.1, 0.4));
        models.insert("gemini-2.5-pro".into(), ModelPricing::standard(1.25, 10.0));
        models.insert("gemini-2.5-flash".into(), ModelPricing::standard(0.15, 0.6));

        // DeepSeek
        models.insert("deepseek-chat".into(), ModelPricing::standard(0.27, 1.1));
        models.insert(
            "deepseek-reasoner".into(),
            ModelPricing::standard(0.55, 2.19),
        );

        // Groq
        models.insert(
            "llama-3.3-70b-versatile".into(),
            ModelPricing::standard(0.59, 0.79),
        );

        // Mistral
        models.insert(
            "mistral-large-latest".into(),
            ModelPricing::standard(2.0, 6.0),
        );

        // xAI
        models.insert("grok-3".into(), ModelPricing::standard(3.0, 15.0));
        models.insert("grok-3-mini".into(), ModelPricing::standard(0.3, 0.5));

        // Fireworks AI
        models.insert(
            "accounts/fireworks/models/llama-v3p3-70b-instruct".into(),
            ModelPricing::standard(0.9, 0.9),
        );

        // SambaNova
        models.insert(
            "Meta-Llama-3.3-70B-Instruct".into(),
            ModelPricing::standard(0.6, 0.6),
        );

        // Together AI
        models.insert(
            "meta-llama/Llama-3.3-70B-Instruct-Turbo".into(),
            ModelPricing::standard(0.88, 0.88),
        );

        // Cerebras
        models.insert("llama-3.3-70b".into(), ModelPricing::standard(0.85, 1.2));

        // Perplexity
        models.insert("sonar-pro".into(), ModelPricing::standard(3.0, 15.0));
        models.insert("sonar".into(), ModelPricing::standard(1.0, 1.0));

        // Cohere
        models.insert("command-r-plus".into(), ModelPricing::standard(2.5, 10.0));
        models.insert("command-r".into(), ModelPricing::standard(0.15, 0.6));

        // Qwen
        models.insert("qwen-plus".into(), ModelPricing::standard(0.8, 2.0));
        models.insert("qwen-max".into(), ModelPricing::standard(2.4, 9.6));

        // AI21 Labs
        models.insert("jamba-1.5-large".into(), ModelPricing::standard(2.0, 8.0));
        models.insert("jamba-1.5-mini".into(), ModelPricing::standard(0.2, 0.4));

        // Moonshot AI (Kimi)
        models.insert("moonshot-v1-128k".into(), ModelPricing::standard(2.0, 5.0));
        models.insert("moonshot-v1-32k".into(), ModelPricing::standard(1.0, 2.5));
        models.insert("moonshot-v1-8k".into(), ModelPricing::standard(0.5, 1.25));

        // 01.AI (Yi)
        models.insert("yi-large".into(), ModelPricing::standard(3.0, 3.0));
        models.insert("yi-medium".into(), ModelPricing::standard(0.5, 0.5));

        // Zhipu AI (GLM)
        models.insert("glm-4-plus".into(), ModelPricing::standard(0.6, 2.2));
        models.insert("glm-4".into(), ModelPricing::standard(0.6, 2.2));

        // Novita AI (reseller pricing)
        models.insert(
            "deepseek/deepseek-v3-0324".into(),
            ModelPricing::standard(0.28, 1.14),
        );

        // Inflection AI
        models.insert("inflection-3-pi".into(), ModelPricing::standard(2.5, 10.0));
        models.insert(
            "inflection-3-productivity".into(),
            ModelPricing::standard(2.5, 10.0),
        );

        // Hugging Face (pass-through pricing varies; using typical rates)
        models.insert(
            "meta-llama/Llama-3.3-70B-Instruct".into(),
            ModelPricing::standard(0.1, 0.4),
        );

        // Reka AI
        models.insert("reka-core".into(), ModelPricing::standard(10.0, 25.0));
        models.insert("reka-flash".into(), ModelPricing::standard(0.8, 2.0));

        Self { models }
    }

    /// Insert or overwrite pricing for a model.
    #[allow(dead_code)]
    pub fn insert(&mut self, model_id: String, pricing: ModelPricing) {
        self.models.insert(model_id, pricing);
    }

    /// Load user pricing overrides from config. These take highest priority.
    pub fn load_from_config(
        &mut self,
        overrides: &std::collections::HashMap<String, crate::config::schema::PricingOverride>,
    ) {
        for (model_id, o) in overrides {
            let pricing = ModelPricing {
                input_per_m: o.input_per_m,
                output_per_m: o.output_per_m,
                cache_read_per_m: o.cache_read_per_m.unwrap_or(o.input_per_m * 0.1),
                cache_creation_per_m: o.cache_creation_per_m.unwrap_or(o.input_per_m * 1.25),
            };
            self.models.insert(model_id.clone(), pricing);
        }
    }

    /// Insert pricing with cross-provider mapping: if the model ID contains a
    /// provider prefix (e.g. `anthropic/claude-sonnet-4-6`), also insert the
    /// bare model ID (`claude-sonnet-4-6`) so direct-provider users get pricing.
    /// Does NOT overwrite existing entries for the bare ID (static/user overrides win).
    pub fn insert_with_cross_map(&mut self, model_id: String, pricing: ModelPricing) {
        // Always insert the full ID
        self.models.insert(model_id.clone(), pricing);
        // If prefixed, also insert the bare model ID (if not already present)
        if let Some((_, bare)) = model_id.split_once('/') {
            self.models.entry(bare.to_string()).or_insert(pricing);
        }
    }

    /// Look up pricing for a model. Returns `None` for unknown models.
    /// Falls back to stripping provider prefix (e.g. `google/gemini-2.5-pro` → `gemini-2.5-pro`)
    /// for OpenRouter-style model IDs.
    pub fn get(&self, model_id: &str) -> Option<ModelPricing> {
        self.models
            .get(model_id)
            .or_else(|| {
                model_id
                    .split_once('/')
                    .and_then(|(_, bare)| self.models.get(bare))
            })
            .copied()
    }

    /// Estimate cost in USD for the given token counts (without cache breakdown).
    /// Returns `None` if the model has no known pricing.
    pub fn estimate_cost(
        &self,
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Option<f64> {
        self.estimate_cost_with_cache(model_id, input_tokens, output_tokens, 0, 0)
    }

    /// Estimate cost in USD with cache token breakdown.
    /// `cache_read_tokens` and `cache_creation_tokens` are subtracted from `input_tokens`
    /// and charged at their respective rates.
    pub fn estimate_cost_with_cache(
        &self,
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_creation_tokens: u32,
    ) -> Option<f64> {
        let p = self.get(model_id)?;
        // Cache tokens are a subset of input tokens charged at different rates.
        // Regular input = total input - cache_read - cache_creation
        let regular_input = input_tokens.saturating_sub(cache_read_tokens + cache_creation_tokens);
        let input_cost = (regular_input as f64) * p.input_per_m / 1_000_000.0;
        let cache_read_cost = (cache_read_tokens as f64) * p.cache_read_per_m / 1_000_000.0;
        let cache_create_cost =
            (cache_creation_tokens as f64) * p.cache_creation_per_m / 1_000_000.0;
        let output_cost = (output_tokens as f64) * p.output_per_m / 1_000_000.0;
        Some(input_cost + cache_read_cost + cache_create_cost + output_cost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_uses_specific_pricing() {
        let reg = PricingRegistry::new();
        let p = reg
            .get("claude-opus-4-6")
            .expect("should have opus pricing");
        assert!((p.input_per_m - 15.0).abs() < f64::EPSILON);
        assert!((p.output_per_m - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unknown_model_returns_none() {
        let reg = PricingRegistry::new();
        assert!(reg.get("some-unknown-model").is_none());
    }

    #[test]
    fn estimate_cost_calculation() {
        let reg = PricingRegistry::new();
        // 1M input tokens at $3/M = $3.00, 1M output at $15/M = $15.00
        let cost = reg
            .estimate_cost("claude-sonnet-4-6", 1_000_000, 1_000_000)
            .unwrap();
        assert!((cost - 18.0).abs() < 0.001);
    }

    #[test]
    fn estimate_cost_unknown_model_returns_none() {
        let reg = PricingRegistry::new();
        assert!(reg.estimate_cost("unknown-model", 1000, 1000).is_none());
    }

    #[test]
    fn insert_overrides_pricing() {
        let mut reg = PricingRegistry::new();
        reg.insert("custom-model".into(), ModelPricing::standard(1.0, 2.0));
        let p = reg.get("custom-model").expect("should have custom pricing");
        assert!((p.input_per_m - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn prefixed_model_id_falls_back_to_bare() {
        let reg = PricingRegistry::new();
        let p = reg
            .get("google/gemini-2.5-pro")
            .expect("should fall back to gemini-2.5-pro");
        assert!((p.input_per_m - 1.25).abs() < f64::EPSILON);
    }

    #[test]
    fn prefixed_model_prefers_exact_match() {
        let mut reg = PricingRegistry::new();
        reg.insert(
            "google/gemini-2.5-pro".into(),
            ModelPricing::standard(99.0, 99.0),
        );
        let p = reg.get("google/gemini-2.5-pro").unwrap();
        assert!((p.input_per_m - 99.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cache_cost_reduces_regular_input() {
        let reg = PricingRegistry::new();
        // Claude Sonnet: $3/M input, $0.30/M cache read, $3.75/M cache creation, $15/M output
        // 100k input, 50k cache read, 10k cache creation, 20k output
        // Regular input = 100k - 50k - 10k = 40k
        let cost = reg
            .estimate_cost_with_cache("claude-sonnet-4-6", 100_000, 20_000, 50_000, 10_000)
            .unwrap();
        let expected = (40_000.0 * 3.0 / 1_000_000.0)   // regular input
            + (50_000.0 * 0.3 / 1_000_000.0)             // cache read
            + (10_000.0 * 3.75 / 1_000_000.0)            // cache creation
            + (20_000.0 * 15.0 / 1_000_000.0); // output
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn standard_creates_default_cache_rates() {
        let p = ModelPricing::standard(10.0, 20.0);
        assert!((p.cache_read_per_m - 1.0).abs() < f64::EPSILON); // 10% of input
        assert!((p.cache_creation_per_m - 12.5).abs() < f64::EPSILON); // 125% of input
    }

    #[test]
    fn insert_with_cross_map_creates_bare_id() {
        let mut reg = PricingRegistry::new();
        reg.insert_with_cross_map(
            "anthropic/claude-test-model".into(),
            ModelPricing::standard(5.0, 25.0),
        );
        // Full ID works
        let p = reg.get("anthropic/claude-test-model").unwrap();
        assert!((p.input_per_m - 5.0).abs() < f64::EPSILON);
        // Bare ID also works
        let p2 = reg.get("claude-test-model").unwrap();
        assert!((p2.input_per_m - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn insert_with_cross_map_does_not_overwrite_existing_bare() {
        let mut reg = PricingRegistry::new();
        // Static entry for claude-sonnet-4-6 already exists at $3/$15
        assert!(reg.get("claude-sonnet-4-6").is_some());
        // Insert OpenRouter version — should NOT overwrite the bare entry
        reg.insert_with_cross_map(
            "anthropic/claude-sonnet-4-6".into(),
            ModelPricing::standard(99.0, 99.0),
        );
        // Full prefixed ID gets the new price
        let p = reg.get("anthropic/claude-sonnet-4-6").unwrap();
        assert!((p.input_per_m - 99.0).abs() < f64::EPSILON);
        // Bare ID keeps the original static price
        let p2 = reg.get("claude-sonnet-4-6").unwrap();
        assert!((p2.input_per_m - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn load_from_config_overrides_static() {
        let mut reg = PricingRegistry::new();
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "claude-sonnet-4-6".to_string(),
            crate::config::schema::PricingOverride {
                input_per_m: 1.0,
                output_per_m: 2.0,
                cache_read_per_m: None,
                cache_creation_per_m: None,
            },
        );
        reg.load_from_config(&overrides);
        let p = reg.get("claude-sonnet-4-6").unwrap();
        assert!((p.input_per_m - 1.0).abs() < f64::EPSILON);
        assert!((p.output_per_m - 2.0).abs() < f64::EPSILON);
        // Cache rates default from input
        assert!((p.cache_read_per_m - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn load_from_config_with_explicit_cache_rates() {
        let mut reg = PricingRegistry::new();
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "custom-model".to_string(),
            crate::config::schema::PricingOverride {
                input_per_m: 10.0,
                output_per_m: 20.0,
                cache_read_per_m: Some(0.5),
                cache_creation_per_m: Some(15.0),
            },
        );
        reg.load_from_config(&overrides);
        let p = reg.get("custom-model").unwrap();
        assert!((p.cache_read_per_m - 0.5).abs() < f64::EPSILON);
        assert!((p.cache_creation_per_m - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn new_providers_have_pricing() {
        let reg = PricingRegistry::new();
        assert!(reg.get("jamba-1.5-large").is_some());
        assert!(reg.get("moonshot-v1-128k").is_some());
        assert!(reg.get("yi-large").is_some());
        assert!(reg.get("glm-4-plus").is_some());
        assert!(reg.get("deepseek/deepseek-v3-0324").is_some());
        assert!(reg.get("inflection-3-pi").is_some());
        assert!(reg.get("meta-llama/Llama-3.3-70B-Instruct").is_some());
        assert!(reg.get("reka-core").is_some());
    }
}
