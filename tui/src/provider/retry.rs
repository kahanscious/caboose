//! Retry logic with exponential backoff and jitter.

use crate::provider::{Message, ModelInfo, Provider, StreamEvent, ToolDefinition};
use futures::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// Retry configuration.
pub struct RetryPolicy {
    pub base_delay: Duration,
    pub max_retries: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_secs(2),
            max_retries: 8,
        }
    }
}

/// Calculate the delay for a given retry attempt using exponential backoff with fixed padding.
///
/// Formula: base_delay * 2^attempt + 20% padding
///
/// `attempt` is 0-indexed (first retry = attempt 0).
/// Note: the 20% padding is deterministic, not random jitter. For a single-user TUI
/// this is fine; true jitter would require a random source.
pub fn backoff_delay(base: Duration, attempt: u32) -> Duration {
    let multiplier = 2u64.saturating_pow(attempt);
    let base_ms = base.as_millis() as u64;
    let delay_ms = base_ms.saturating_mul(multiplier);
    let padding_ms = delay_ms / 5; // 20% padding
    Duration::from_millis(delay_ms + padding_ms)
}

/// Calculate backoff delay, respecting server-provided Retry-After if available.
///
/// Note: currently unused by RetryProvider because Retry-After duration is lost
/// when provider errors are flattened to anyhow strings. Kept for future use when
/// structured error propagation is added.
#[allow(dead_code)]
pub fn effective_delay(base: Duration, attempt: u32, retry_after: Option<Duration>) -> Duration {
    match retry_after {
        Some(server_delay) => server_delay.max(backoff_delay(base, attempt)),
        None => backoff_delay(base, attempt),
    }
}

/// Parse the Retry-After header value (seconds) into a Duration.
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Middleware provider that wraps an inner provider with retry logic.
///
/// Retries on transient errors (rate limits, server errors, network errors)
/// with exponential backoff. Non-retryable errors (auth, not-found) are
/// passed through immediately.
pub struct RetryProvider {
    inner: Arc<dyn Provider>,
    policy: RetryPolicy,
}

impl RetryProvider {
    pub fn new(inner: Arc<dyn Provider>) -> Self {
        Self {
            inner,
            policy: RetryPolicy::default(),
        }
    }

    #[allow(dead_code)]
    pub fn with_policy(mut self, policy: RetryPolicy) -> Self {
        self.policy = policy;
        self
    }
}

impl Provider for RetryProvider {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamEvent>> + Send + 'static>> {
        let inner = self.inner.clone();
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        let max_retries = self.policy.max_retries;
        let base_delay = self.policy.base_delay;

        Box::pin(async_stream::stream! {
            let mut attempt = 0u32;

            loop {
                let mut stream = inner.stream(&messages, &tools);
                let mut first_item = true;
                let mut should_retry = false;

                while let Some(result) = futures::StreamExt::next(&mut stream).await {
                    match result {
                        Ok(event) => {
                            first_item = false;
                            yield Ok(event);
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            let is_retryable = msg.contains("Rate limited")
                                || msg.contains("Server error")
                                || msg.contains("Network error");

                            if is_retryable && attempt < max_retries && first_item {
                                attempt += 1;
                                let delay = backoff_delay(base_delay, attempt - 1);
                                warn!(
                                    attempt = attempt,
                                    max = max_retries,
                                    delay_ms = delay.as_millis() as u64,
                                    error = %msg,
                                    "Retrying provider request"
                                );
                                let delay_display = if delay.as_millis() < 1000 {
                                    format!("{}ms", delay.as_millis())
                                } else {
                                    format!("{:.1}s", delay.as_secs_f64())
                                };
                                yield Ok(StreamEvent::Error(format!(
                                    "{}. Retrying in {}... (attempt {}/{})",
                                    msg,
                                    delay_display,
                                    attempt,
                                    max_retries
                                )));
                                tokio::time::sleep(delay).await;
                                should_retry = true;
                                break;
                            } else {
                                let category = crate::provider::error::classify_from_string(&msg);
                                let provider = crate::provider::error::provider_from_string(&msg);
                                let hint = crate::provider::error::hint_from_category(
                                    &category,
                                    provider.as_deref(),
                                );
                                yield Ok(StreamEvent::ProviderError {
                                    category,
                                    provider: provider.unwrap_or_else(|| inner.name().to_string()),
                                    message: msg,
                                    hint,
                                });
                                return;
                            }
                        }
                    }
                }

                if !should_retry {
                    return;
                }
            }
        })
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model(&self) -> &str {
        self.inner.model()
    }

    fn list_models(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ModelInfo>>> + Send + '_>> {
        self.inner.list_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_each_attempt() {
        let base = Duration::from_secs(2);
        let d0 = backoff_delay(base, 0);
        let d1 = backoff_delay(base, 1);
        let d2 = backoff_delay(base, 2);

        assert!(d1 > d0);
        assert!(d2 > d1);
        // Check approximate values (with 20% jitter)
        assert!(d0.as_millis() >= 2000 && d0.as_millis() <= 2500);
        assert!(d1.as_millis() >= 4000 && d1.as_millis() <= 5000);
        assert!(d2.as_millis() >= 8000 && d2.as_millis() <= 10000);
    }

    #[test]
    fn effective_delay_respects_retry_after() {
        let base = Duration::from_secs(2);
        let server = Duration::from_secs(30);
        let delay = effective_delay(base, 0, Some(server));
        assert!(delay >= Duration::from_secs(30));
    }

    #[test]
    fn effective_delay_uses_backoff_when_no_retry_after() {
        let base = Duration::from_secs(2);
        let delay = effective_delay(base, 0, None);
        assert!(delay.as_millis() >= 2000 && delay.as_millis() <= 2500);
    }

    #[test]
    fn parse_retry_after_valid() {
        assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after(" 5 "), Some(Duration::from_secs(5)));
    }

    #[test]
    fn parse_retry_after_invalid() {
        assert_eq!(parse_retry_after("not-a-number"), None);
        assert_eq!(parse_retry_after(""), None);
    }

    #[test]
    fn backoff_does_not_overflow() {
        let base = Duration::from_secs(2);
        let delay = backoff_delay(base, 30);
        assert!(delay.as_secs() > 0);
    }
}

#[cfg(test)]
mod retry_provider_tests {
    use super::*;
    use crate::provider::{Message, ModelInfo, Provider, StreamEvent, ToolDefinition};
    use futures::StreamExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A mock provider that fails N times then succeeds.
    struct FailingProvider {
        failures_remaining: Arc<AtomicU32>,
    }

    impl FailingProvider {
        fn new(fail_count: u32) -> Self {
            Self {
                failures_remaining: Arc::new(AtomicU32::new(fail_count)),
            }
        }
    }

    impl Provider for FailingProvider {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamEvent>> + Send + 'static>>
        {
            let remaining = self.failures_remaining.clone();
            Box::pin(async_stream::stream! {
                let r = remaining.fetch_sub(1, Ordering::SeqCst);
                if r > 0 {
                    yield Err(anyhow::anyhow!(
                        "Server error (502): Bad Gateway. Try again in a few minutes."
                    ));
                } else {
                    yield Ok(StreamEvent::TextDelta("hello".to_string()));
                    yield Ok(StreamEvent::Done {
                        input_tokens: Some(10),
                        output_tokens: Some(5),
                        cache_read_tokens: None,
                        cache_creation_tokens: None,
                    });
                }
            })
        }

        fn name(&self) -> &str {
            "test"
        }
        fn model(&self) -> &str {
            "test-model"
        }
        fn list_models(
            &self,
        ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<ModelInfo>>> + Send + '_>>
        {
            Box::pin(async { Ok(vec![]) })
        }
    }

    #[tokio::test]
    async fn retry_provider_retries_on_server_error() {
        let inner = FailingProvider::new(2);
        let provider = RetryProvider::new(Arc::new(inner)).with_policy(RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_retries: 5,
        });
        let mut stream = provider.stream(&[], &[]);
        let mut got_text = false;
        while let Some(result) = stream.next().await {
            if let Ok(StreamEvent::TextDelta(text)) = result {
                assert_eq!(text, "hello");
                got_text = true;
            }
        }
        assert!(got_text, "should have received text after retries");
    }

    #[tokio::test]
    async fn retry_provider_gives_up_after_max_retries() {
        let inner = FailingProvider::new(100);
        let provider = RetryProvider::new(Arc::new(inner)).with_policy(RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_retries: 3,
        });
        let mut stream = provider.stream(&[], &[]);
        let mut got_provider_error = false;
        while let Some(result) = stream.next().await {
            if let Ok(StreamEvent::ProviderError { category, .. }) = result {
                got_provider_error = true;
                assert_eq!(category, crate::provider::error::ErrorCategory::ServerError);
            }
        }
        assert!(
            got_provider_error,
            "should have received structured provider error after exhausting retries"
        );
    }

    #[tokio::test]
    async fn retry_provider_does_not_retry_auth_errors() {
        struct AuthFailProvider;
        impl Provider for AuthFailProvider {
            fn stream(
                &self,
                _messages: &[Message],
                _tools: &[ToolDefinition],
            ) -> Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamEvent>> + Send + 'static>>
            {
                Box::pin(async_stream::stream! {
                    yield Err(anyhow::anyhow!(
                        "Authentication failed for test: Invalid API key. Use /connect test to update your API key."
                    ));
                })
            }
            fn name(&self) -> &str {
                "test"
            }
            fn model(&self) -> &str {
                "test-model"
            }
            fn list_models(
                &self,
            ) -> Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<Vec<ModelInfo>>> + Send + '_>,
            > {
                Box::pin(async { Ok(vec![]) })
            }
        }

        let provider = RetryProvider::new(Arc::new(AuthFailProvider)).with_policy(RetryPolicy {
            base_delay: Duration::from_millis(1),
            max_retries: 5,
        });
        let mut stream = provider.stream(&[], &[]);
        let mut got_provider_error = false;
        while let Some(result) = stream.next().await {
            if let Ok(StreamEvent::ProviderError { category, .. }) = result {
                got_provider_error = true;
                assert_eq!(category, crate::provider::error::ErrorCategory::Auth);
            }
        }
        assert!(
            got_provider_error,
            "should have received auth provider error without retrying"
        );
    }

    #[test]
    fn retry_provider_delegates_name_and_model() {
        let inner = FailingProvider::new(0);
        let provider = RetryProvider::new(Arc::new(inner));
        assert_eq!(provider.name(), "test");
        assert_eq!(provider.model(), "test-model");
    }
}
