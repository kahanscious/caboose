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
    // --- Other (alphabetical) ---
    ProviderEntry {
        id: "deepseek",
        display_name: "DeepSeek",
        description: "DeepSeek API key",
        env_var: "DEEPSEEK_API_KEY",
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
];

/// Get popular providers (for the top section of the picker).
#[allow(dead_code)]
pub fn popular() -> impl Iterator<Item = &'static ProviderEntry> {
    CATALOG.iter().filter(|e| e.popular)
}

/// Get non-popular providers (for the "Other" section), alphabetical by display_name.
#[allow(dead_code)]
pub fn other() -> impl Iterator<Item = &'static ProviderEntry> {
    CATALOG.iter().filter(|e| !e.popular)
}

/// Look up a provider by ID.
#[allow(dead_code)]
pub fn by_id(id: &str) -> Option<&'static ProviderEntry> {
    CATALOG.iter().find(|e| e.id == id)
}
