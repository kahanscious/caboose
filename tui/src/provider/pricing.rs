//! Per-provider, per-model pricing for cost estimation.

use std::collections::HashMap;

/// Pricing for a single model (USD per million tokens).
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_m: f64,
    pub output_per_m: f64,
}

/// Registry of known model pricing. Populated at startup with static
/// entries; OpenRouter entries added dynamically from API response.
#[derive(Debug, Clone)]
pub struct PricingRegistry {
    models: HashMap<String, ModelPricing>,
}

impl PricingRegistry {
    /// Create registry pre-populated with known model pricing.
    pub fn new() -> Self {
        let mut models = HashMap::new();

        // Anthropic
        models.insert(
            "claude-sonnet-4-6".into(),
            ModelPricing {
                input_per_m: 3.0,
                output_per_m: 15.0,
            },
        );
        models.insert(
            "claude-sonnet-4-20250514".into(),
            ModelPricing {
                input_per_m: 3.0,
                output_per_m: 15.0,
            },
        );
        models.insert(
            "claude-haiku-4-5-20251001".into(),
            ModelPricing {
                input_per_m: 0.8,
                output_per_m: 4.0,
            },
        );
        models.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input_per_m: 15.0,
                output_per_m: 75.0,
            },
        );

        // OpenAI
        models.insert(
            "gpt-4o".into(),
            ModelPricing {
                input_per_m: 2.5,
                output_per_m: 10.0,
            },
        );
        models.insert(
            "gpt-4o-mini".into(),
            ModelPricing {
                input_per_m: 0.15,
                output_per_m: 0.6,
            },
        );
        models.insert(
            "o3".into(),
            ModelPricing {
                input_per_m: 10.0,
                output_per_m: 40.0,
            },
        );
        models.insert(
            "o4-mini".into(),
            ModelPricing {
                input_per_m: 1.1,
                output_per_m: 4.4,
            },
        );

        // Gemini
        models.insert(
            "gemini-2.0-flash".into(),
            ModelPricing {
                input_per_m: 0.1,
                output_per_m: 0.4,
            },
        );
        models.insert(
            "gemini-2.5-pro".into(),
            ModelPricing {
                input_per_m: 1.25,
                output_per_m: 10.0,
            },
        );
        models.insert(
            "gemini-2.5-flash".into(),
            ModelPricing {
                input_per_m: 0.15,
                output_per_m: 0.6,
            },
        );

        // DeepSeek
        models.insert(
            "deepseek-chat".into(),
            ModelPricing {
                input_per_m: 0.27,
                output_per_m: 1.1,
            },
        );
        models.insert(
            "deepseek-reasoner".into(),
            ModelPricing {
                input_per_m: 0.55,
                output_per_m: 2.19,
            },
        );

        // Groq
        models.insert(
            "llama-3.3-70b-versatile".into(),
            ModelPricing {
                input_per_m: 0.59,
                output_per_m: 0.79,
            },
        );

        // Mistral
        models.insert(
            "mistral-large-latest".into(),
            ModelPricing {
                input_per_m: 2.0,
                output_per_m: 6.0,
            },
        );

        Self { models }
    }

    /// Insert or overwrite pricing for a model (used by OpenRouter dynamic fetch).
    pub fn insert(&mut self, model_id: String, pricing: ModelPricing) {
        self.models.insert(model_id, pricing);
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

    /// Estimate cost in USD for the given token counts.
    /// Returns `None` if the model has no known pricing.
    pub fn estimate_cost(
        &self,
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) -> Option<f64> {
        let p = self.get(model_id)?;
        let input_cost = (input_tokens as f64) * p.input_per_m / 1_000_000.0;
        let output_cost = (output_tokens as f64) * p.output_per_m / 1_000_000.0;
        Some(input_cost + output_cost)
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
        reg.insert(
            "custom-model".into(),
            ModelPricing {
                input_per_m: 1.0,
                output_per_m: 2.0,
            },
        );
        let p = reg.get("custom-model").expect("should have custom pricing");
        assert!((p.input_per_m - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn prefixed_model_id_falls_back_to_bare() {
        let reg = PricingRegistry::new();
        // OpenRouter-style prefixed ID should resolve to the bare model
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
            ModelPricing {
                input_per_m: 99.0,
                output_per_m: 99.0,
            },
        );
        // Exact match should win over stripped fallback
        let p = reg.get("google/gemini-2.5-pro").unwrap();
        assert!((p.input_per_m - 99.0).abs() < f64::EPSILON);
    }
}
