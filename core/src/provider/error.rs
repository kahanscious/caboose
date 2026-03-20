//! Classified provider errors for retry and display logic.

use std::time::Duration;

/// Broad error category for display-layer rendering.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorCategory {
    Auth,
    Network,
    RateLimit,
    ModelNotFound,
    ServerError,
    Unknown,
}

impl ErrorCategory {
    /// Human-readable label for the error box header.
    pub fn label(&self) -> &'static str {
        match self {
            ErrorCategory::Auth => "Auth Error",
            ErrorCategory::Network => "Network Error",
            ErrorCategory::RateLimit => "Rate Limited",
            ErrorCategory::ModelNotFound => "Model Error",
            ErrorCategory::ServerError => "Server Error",
            ErrorCategory::Unknown => "Error",
        }
    }

    /// Whether this category represents a transient (potentially self-resolving) error.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ErrorCategory::Network | ErrorCategory::RateLimit | ErrorCategory::ServerError
        )
    }
}

/// Classified error from a provider request.
#[derive(Debug)]
pub enum ProviderError {
    /// Rate limited (HTTP 429). May include server-suggested retry delay.
    RateLimit {
        retry_after: Option<Duration>,
        message: String,
    },
    /// Authentication failure (HTTP 401/403).
    Auth {
        #[allow(dead_code)]
        status: u16,
        message: String,
        provider: String,
    },
    /// Network-level error (connection refused, timeout, DNS).
    Network { message: String },
    /// Model not found or invalid (HTTP 404).
    ModelNotFound { model: String, message: String },
    /// Server error (HTTP 500/502/503).
    ServerError { status: u16, message: String },
    /// Unclassified error.
    Unknown(anyhow::Error),
}

#[allow(dead_code)]
impl ProviderError {
    /// Whether this error class is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimit { .. }
                | ProviderError::Network { .. }
                | ProviderError::ServerError { .. }
        )
    }

    /// Maximum retry attempts for this error class.
    pub fn max_retries(&self) -> u32 {
        match self {
            ProviderError::RateLimit { .. } => 8,
            ProviderError::Network { .. } | ProviderError::ServerError { .. } => 3,
            _ => 0,
        }
    }

    /// User-facing error message with actionable hint.
    pub fn user_message(&self) -> String {
        match self {
            ProviderError::RateLimit { message, .. } => {
                format!("Rate limited: {message}")
            }
            ProviderError::Auth {
                provider, message, ..
            } => {
                format!(
                    "Authentication failed for {provider}: {message}. Use /connect {provider} to update your API key."
                )
            }
            ProviderError::Network { message } => {
                format!("Network error: {message}")
            }
            ProviderError::ModelNotFound { model, message } => {
                format!(
                    "Model '{model}' not found: {message}. Use /model to pick an available model."
                )
            }
            ProviderError::ServerError { status, message } => {
                format!("Server error ({status}): {message}. Try again in a few minutes.")
            }
            ProviderError::Unknown(e) => format!("Provider error: {e}"),
        }
    }

    /// Error category for structured display.
    pub fn category(&self) -> ErrorCategory {
        match self {
            ProviderError::Auth { .. } => ErrorCategory::Auth,
            ProviderError::Network { .. } => ErrorCategory::Network,
            ProviderError::RateLimit { .. } => ErrorCategory::RateLimit,
            ProviderError::ModelNotFound { .. } => ErrorCategory::ModelNotFound,
            ProviderError::ServerError { .. } => ErrorCategory::ServerError,
            ProviderError::Unknown(_) => ErrorCategory::Unknown,
        }
    }

    /// Actionable hint for the user, if any.
    pub fn hint(&self) -> Option<String> {
        match self {
            ProviderError::Auth { provider, .. } => {
                Some(format!("Run /connect {provider} to update your API key"))
            }
            ProviderError::Network { .. } => Some("Check your internet connection".to_string()),
            ProviderError::ModelNotFound { .. } => {
                Some("Run /model to pick an available model".to_string())
            }
            ProviderError::ServerError { .. } => {
                Some("The provider may be experiencing issues — try again shortly".to_string())
            }
            ProviderError::RateLimit { .. } => {
                Some("You've hit the provider's rate limit — wait a moment and retry".to_string())
            }
            ProviderError::Unknown(_) => None,
        }
    }

    /// Provider name, if available from the error.
    pub fn provider_name(&self) -> Option<&str> {
        match self {
            ProviderError::Auth { provider, .. } => Some(provider.as_str()),
            _ => None,
        }
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.user_message())
    }
}

impl std::error::Error for ProviderError {}

/// Classify an HTTP status code into a ProviderError.
pub fn classify_status(status: u16, body: &str, provider: &str, model: &str) -> ProviderError {
    match status {
        429 => ProviderError::RateLimit {
            retry_after: None, // caller sets from Retry-After header if available
            message: extract_error_message(body),
        },
        401 | 403 => ProviderError::Auth {
            status,
            message: extract_error_message(body),
            provider: provider.to_string(),
        },
        404 => ProviderError::ModelNotFound {
            model: model.to_string(),
            message: extract_error_message(body),
        },
        500..=599 => ProviderError::ServerError {
            status,
            message: extract_error_message(body),
        },
        _ => ProviderError::Unknown(anyhow::anyhow!(
            "HTTP {status}: {}",
            extract_error_message(body)
        )),
    }
}

/// Try to extract an error message from a JSON response body.
/// Falls back to the raw body if parsing fails.
fn extract_error_message(body: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body) {
        // OpenAI/Anthropic/Mistral style: {"error": {"message": "..."}}
        if let Some(msg) = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
        {
            return msg.to_string();
        }
        // Gemini style: {"error": {"status": "...", "message": "..."}}
        if let Some(msg) = val.get("error").and_then(|e| e.as_str()) {
            return msg.to_string();
        }
    }
    body.to_string()
}

/// Best-effort classification of an error message string back into a category.
/// Used by the retry layer where structured ProviderError has been flattened to anyhow.
pub fn classify_from_string(msg: &str) -> ErrorCategory {
    if msg.contains("Authentication failed") {
        ErrorCategory::Auth
    } else if msg.contains("Rate limited") {
        ErrorCategory::RateLimit
    } else if msg.contains("Network error") {
        ErrorCategory::Network
    } else if msg.contains("not found") {
        ErrorCategory::ModelNotFound
    } else if msg.contains("Server error") {
        ErrorCategory::ServerError
    } else {
        ErrorCategory::Unknown
    }
}

/// Extract a provider name from an error message string, if present.
pub fn provider_from_string(msg: &str) -> Option<String> {
    if let Some(after_for) = msg.strip_prefix("Authentication failed for ")
        && let Some(colon_pos) = after_for.find(':')
    {
        return Some(after_for[..colon_pos].to_string());
    }
    None
}

/// Extract the actionable hint for a classified string error.
pub fn hint_from_category(category: &ErrorCategory, provider: Option<&str>) -> Option<String> {
    match category {
        ErrorCategory::Auth => {
            let p = provider.unwrap_or("your provider");
            Some(format!("Run /connect {p} to update your API key"))
        }
        ErrorCategory::Network => Some("Check your internet connection".to_string()),
        ErrorCategory::ModelNotFound => Some("Run /model to pick an available model".to_string()),
        ErrorCategory::ServerError => {
            Some("The provider may be experiencing issues — try again shortly".to_string())
        }
        ErrorCategory::RateLimit => {
            Some("You've hit the provider's rate limit — wait a moment and retry".to_string())
        }
        ErrorCategory::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_429_as_rate_limit() {
        let err = classify_status(
            429,
            r#"{"error":{"message":"Rate limit exceeded"}}"#,
            "openai",
            "gpt-4o",
        );
        assert!(matches!(err, ProviderError::RateLimit { .. }));
        assert!(err.is_retryable());
        assert_eq!(err.max_retries(), 8);
    }

    #[test]
    fn classify_401_as_auth() {
        let err = classify_status(
            401,
            r#"{"error":{"message":"Invalid API key"}}"#,
            "anthropic",
            "claude-sonnet-4-6",
        );
        assert!(matches!(err, ProviderError::Auth { status: 401, .. }));
        assert!(!err.is_retryable());
        assert!(err.user_message().contains("/connect anthropic"));
    }

    #[test]
    fn classify_403_as_auth() {
        let err = classify_status(403, "Forbidden", "openai", "gpt-4o");
        assert!(matches!(err, ProviderError::Auth { status: 403, .. }));
    }

    #[test]
    fn classify_404_as_model_not_found() {
        let err = classify_status(
            404,
            r#"{"error":{"message":"Model not found"}}"#,
            "openai",
            "gpt-5-turbo",
        );
        assert!(matches!(err, ProviderError::ModelNotFound { .. }));
        assert!(!err.is_retryable());
        assert!(err.user_message().contains("/model"));
    }

    #[test]
    fn classify_500_as_server_error() {
        let err = classify_status(
            500,
            "Internal Server Error",
            "anthropic",
            "claude-sonnet-4-6",
        );
        assert!(matches!(
            err,
            ProviderError::ServerError { status: 500, .. }
        ));
        assert!(err.is_retryable());
        assert_eq!(err.max_retries(), 3);
    }

    #[test]
    fn classify_502_as_server_error() {
        let err = classify_status(502, "Bad Gateway", "openai", "gpt-4o");
        assert!(matches!(
            err,
            ProviderError::ServerError { status: 502, .. }
        ));
    }

    #[test]
    fn extract_openai_error_message() {
        let body = r#"{"error":{"message":"You exceeded your current quota","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        let err = classify_status(429, body, "openai", "gpt-4o");
        assert!(
            err.user_message()
                .contains("You exceeded your current quota")
        );
    }

    #[test]
    fn extract_plain_text_fallback() {
        let err = classify_status(500, "plain text error", "openai", "gpt-4o");
        assert!(err.user_message().contains("plain text error"));
    }

    #[test]
    fn network_error_is_retryable() {
        let err = ProviderError::Network {
            message: "connection refused".to_string(),
        };
        assert!(err.is_retryable());
        assert_eq!(err.max_retries(), 3);
    }

    #[test]
    fn category_maps_correctly() {
        let auth = classify_status(401, "bad key", "anthropic", "claude");
        assert_eq!(auth.category(), ErrorCategory::Auth);

        let network = ProviderError::Network {
            message: "timeout".into(),
        };
        assert_eq!(network.category(), ErrorCategory::Network);

        let rate = classify_status(429, "slow down", "openai", "gpt-4o");
        assert_eq!(rate.category(), ErrorCategory::RateLimit);

        let not_found = classify_status(404, "no model", "openai", "gpt-5");
        assert_eq!(not_found.category(), ErrorCategory::ModelNotFound);

        let server = classify_status(502, "bad gateway", "anthropic", "claude");
        assert_eq!(server.category(), ErrorCategory::ServerError);
    }

    #[test]
    fn hints_are_actionable() {
        let auth = classify_status(401, "bad key", "anthropic", "claude");
        assert!(auth.hint().unwrap().contains("/connect"));

        let not_found = classify_status(404, "no model", "openai", "gpt-5");
        assert!(not_found.hint().unwrap().contains("/model"));

        let unknown = ProviderError::Unknown(anyhow::anyhow!("wat"));
        assert!(unknown.hint().is_none());
    }

    #[test]
    fn transient_categories() {
        assert!(ErrorCategory::Network.is_transient());
        assert!(ErrorCategory::RateLimit.is_transient());
        assert!(ErrorCategory::ServerError.is_transient());
        assert!(!ErrorCategory::Auth.is_transient());
        assert!(!ErrorCategory::ModelNotFound.is_transient());
        assert!(!ErrorCategory::Unknown.is_transient());
    }

    #[test]
    fn classify_from_string_patterns() {
        assert_eq!(
            classify_from_string("Authentication failed for openai: Invalid key"),
            ErrorCategory::Auth,
        );
        assert_eq!(
            classify_from_string("Rate limited: slow down"),
            ErrorCategory::RateLimit,
        );
        assert_eq!(
            classify_from_string("Network error: connection refused"),
            ErrorCategory::Network,
        );
        assert_eq!(
            classify_from_string("Model 'gpt-5' not found: no such model"),
            ErrorCategory::ModelNotFound,
        );
        assert_eq!(
            classify_from_string("Server error (502): bad gateway"),
            ErrorCategory::ServerError,
        );
        assert_eq!(
            classify_from_string("Something weird happened"),
            ErrorCategory::Unknown,
        );
    }

    #[test]
    fn provider_from_string_extracts_name() {
        assert_eq!(
            provider_from_string("Authentication failed for anthropic: Invalid API key"),
            Some("anthropic".to_string()),
        );
        assert_eq!(provider_from_string("Network error: timeout"), None,);
    }
}
