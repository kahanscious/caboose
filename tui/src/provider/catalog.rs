//! Provider catalog — hardcoded list of known providers for the /connect picker.
//! Functional providers have working backend implementations. Stubs accept keys
//! but show an error when you try to chat.

/// A provider entry in the catalog.
#[allow(dead_code)]
pub struct ProviderEntry {
    /// Internal ID used in config/auth ("anthropic", "openrouter")
    pub id: &'static str,
    /// Display name shown in the picker
    pub display_name: &'static str,
    /// Short description shown next to the name
    pub description: &'static str,
    /// Environment variable name for this provider's key
    pub env_var: &'static str,
    /// Shown in the "Popular" section if true, "Other" if false
    pub popular: bool,
    /// Has a working Provider implementation
    pub functional: bool,
}

impl ProviderEntry {
    pub fn is_local(&self) -> bool {
        matches!(self.id, "ollama" | "lmstudio" | "llamacpp" | "custom")
    }
}

/// Full provider catalog. Popular entries appear first in their section.
pub const CATALOG: &[ProviderEntry] = &[
    // --- Popular ---
    ProviderEntry {
        id: "openrouter",
        display_name: "OpenRouter",
        description: "Multi-model access via single API key",
        env_var: "OPENROUTER_API_KEY",
        popular: true,
        functional: true,
    },
    ProviderEntry {
        id: "anthropic",
        display_name: "Anthropic",
        description: "Claude Max or API key",
        env_var: "ANTHROPIC_API_KEY",
        popular: true,
        functional: true,
    },
    ProviderEntry {
        id: "openai",
        display_name: "OpenAI",
        description: "ChatGPT Plus/Pro or API key",
        env_var: "OPENAI_API_KEY",
        popular: true,
        functional: true,
    },
    ProviderEntry {
        id: "google",
        display_name: "Google",
        description: "Gemini API key",
        env_var: "GEMINI_API_KEY",
        popular: true,
        functional: true,
    },
    // --- Other (alphabetical by display_name) ---
    ProviderEntry {
        id: "cerebras",
        display_name: "Cerebras",
        description: "Ultra-fast inference",
        env_var: "CEREBRAS_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "cohere",
        display_name: "Cohere",
        description: "Enterprise AI models",
        env_var: "COHERE_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "deepseek",
        display_name: "DeepSeek",
        description: "DeepSeek API key",
        env_var: "DEEPSEEK_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "fireworks",
        display_name: "Fireworks AI",
        description: "Fast open-source inference",
        env_var: "FIREWORKS_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "groq",
        display_name: "Groq",
        description: "Groq API key",
        env_var: "GROQ_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "mistral",
        display_name: "Mistral",
        description: "Mistral API key",
        env_var: "MISTRAL_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "perplexity",
        display_name: "Perplexity",
        description: "Search-augmented AI",
        env_var: "PERPLEXITY_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "qwen",
        display_name: "Qwen (DashScope)",
        description: "Alibaba Cloud AI models",
        env_var: "DASHSCOPE_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "sambanova",
        display_name: "SambaNova",
        description: "Fast enterprise inference",
        env_var: "SAMBANOVA_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "together",
        display_name: "Together AI",
        description: "Open-source model hosting",
        env_var: "TOGETHER_API_KEY",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "xai",
        display_name: "xAI",
        description: "Grok models",
        env_var: "XAI_API_KEY",
        popular: false,
        functional: true,
    },
    // --- Local ---
    ProviderEntry {
        id: "ollama",
        display_name: "Ollama",
        description: "Local — localhost:11434",
        env_var: "",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "lmstudio",
        display_name: "LM Studio",
        description: "Local — localhost:1234",
        env_var: "",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "llamacpp",
        display_name: "llama.cpp",
        description: "Local — localhost:8080",
        env_var: "",
        popular: false,
        functional: true,
    },
    ProviderEntry {
        id: "custom",
        display_name: "Custom (OpenAI-compatible)",
        description: "Local — enter your server address",
        env_var: "",
        popular: false,
        functional: true,
    },
];

/// Look up a provider by ID.
#[allow(dead_code)]
pub fn by_id(id: &str) -> Option<&'static ProviderEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_correct() {
        assert!(by_id("ollama").unwrap().is_local());
        assert!(by_id("custom").unwrap().is_local());
        assert!(!by_id("anthropic").unwrap().is_local());
        assert!(!by_id("openai").unwrap().is_local());
    }
}
