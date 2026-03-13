//! Anthropic provider — direct API streaming with prompt caching.

pub mod cache;
pub mod sse;
pub mod types;

use anyhow::Result;
use futures::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use std::future::Future;
use std::pin::Pin;
use tracing::{debug, warn};

use self::cache::inject_anthropic_cache;
use self::sse::SseAccumulator;
use self::types::*;
use super::{Message, ModelInfo, Provider, StreamEvent, ThinkingMode, ToolDefinition};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const BETA_HEADER: &str = "interleaved-thinking-2025-05-14";
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Anthropic Messages API client with SSE streaming.
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
    thinking_mode: std::sync::atomic::AtomicU8,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::Client::new(),
            thinking_mode: std::sync::atomic::AtomicU8::new(0),
        }
    }

    /// Build the API request body from domain messages and tools.
    fn build_request(&self, messages: &[Message], tools: &[ToolDefinition]) -> ApiRequest {
        let mut system_blocks = Vec::new();
        let mut api_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    // Extract system messages into the system field
                    let text = match &msg.content {
                        val if val.is_string() => val.as_str().unwrap_or("").to_string(),
                        val if val.is_array() => {
                            // Array of content blocks — extract text
                            val.as_array()
                                .unwrap()
                                .iter()
                                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        system_blocks.push(SystemBlock {
                            block_type: "text".to_string(),
                            text,
                            cache_control: None,
                        });
                    }
                }
                role @ ("user" | "assistant") => {
                    let content = self.convert_content(&msg.content);
                    api_messages.push(ApiMessage {
                        role: role.to_string(),
                        content,
                    });
                }
                _ => {
                    warn!(role = msg.role.as_str(), "Unknown message role, skipping");
                }
            }
        }

        let api_tools = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| ApiToolDef {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        input_schema: t.input_schema.clone(),
                        cache_control: None,
                    })
                    .collect(),
            )
        };

        let mode = ThinkingMode::from_u8(
            self.thinking_mode
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let thinking = match mode {
            ThinkingMode::Off => None,
            ThinkingMode::On => Some(ThinkingParam {
                thinking_type: "enabled".to_string(),
                budget_tokens: 10_000,
            }),
        };

        // When thinking is enabled, max_tokens must be larger than budget_tokens
        let max_tokens = match &thinking {
            Some(tp) => tp.budget_tokens + DEFAULT_MAX_TOKENS,
            None => DEFAULT_MAX_TOKENS,
        };

        ApiRequest {
            model: self.model.clone(),
            max_tokens,
            stream: true,
            system: if system_blocks.is_empty() {
                None
            } else {
                Some(system_blocks)
            },
            messages: api_messages,
            tools: api_tools,
            thinking,
        }
    }

    /// Convert a serde_json::Value content field into API content blocks.
    fn convert_content(&self, content: &serde_json::Value) -> Vec<ApiContentBlock> {
        match content {
            // Simple string content
            val if val.is_string() => {
                vec![ApiContentBlock::Text {
                    text: val.as_str().unwrap_or("").to_string(),
                    cache_control: None,
                }]
            }
            // Array of content blocks
            val if val.is_array() => val
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|block| {
                    let block_type = block.get("type").and_then(|t| t.as_str())?;
                    match block_type {
                        "text" => {
                            let text = block.get("text").and_then(|t| t.as_str())?.to_string();
                            Some(ApiContentBlock::Text {
                                text,
                                cache_control: None,
                            })
                        }
                        "tool_use" => {
                            let id = block.get("id").and_then(|v| v.as_str())?.to_string();
                            let name = block.get("name").and_then(|v| v.as_str())?.to_string();
                            let input = block
                                .get("input")
                                .cloned()
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            Some(ApiContentBlock::ToolUse { id, name, input })
                        }
                        "tool_result" => {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())?
                                .to_string();
                            let content = block
                                .get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let is_error = block.get("is_error").and_then(|v| v.as_bool());
                            Some(ApiContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            })
                        }
                        "image" => {
                            let media_type = block
                                .get("media_type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("image/png")
                                .to_string();
                            let data = block
                                .get("data")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            Some(ApiContentBlock::Image {
                                source: ImageSource {
                                    source_type: "base64".to_string(),
                                    media_type,
                                    data,
                                },
                            })
                        }
                        _ => None,
                    }
                })
                .collect(),
            // Fallback
            _ => vec![ApiContentBlock::Text {
                text: content.to_string(),
                cache_control: None,
            }],
        }
    }
}

impl Provider for AnthropicProvider {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn futures::Stream<Item = Result<StreamEvent>> + Send + 'static>> {
        let mut request = self.build_request(messages, tools);
        inject_anthropic_cache(&mut request);

        debug!(
            model = %self.model,
            messages = request.messages.len(),
            tools = request.tools.as_ref().map_or(0, |t| t.len()),
            "Sending Anthropic streaming request"
        );

        let mut req = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json");

        // Add beta header when thinking is enabled
        if request.thinking.is_some() {
            req = req.header("anthropic-beta", BETA_HEADER);
        }

        let req = req.json(&request);

        let provider_name = "anthropic".to_string();
        let model_name = self.model.clone();

        Box::pin(async_stream::stream! {
            let mut es = EventSource::new(req)
                .expect("Failed to create EventSource");
            let mut acc = SseAccumulator::new();

            while let Some(event) = es.next().await {
                match event {
                    Ok(Event::Open) => {
                        debug!("SSE connection opened");
                    }
                    Ok(Event::Message(msg)) => {
                        match acc.process(&msg.data) {
                            Ok(Some(stream_event)) => {
                                let is_done = matches!(stream_event, StreamEvent::Done { .. });
                                yield Ok(stream_event);
                                if is_done {
                                    es.close();
                                    break;
                                }
                            }
                            Ok(None) => {
                                // Internal bookkeeping, continue
                            }
                            Err(e) => {
                                warn!(error = %e, data = %msg.data, "Failed to parse SSE event");
                                // Don't kill the stream for parse errors
                            }
                        }
                    }
                    Err(reqwest_eventsource::Error::StreamEnded) => {
                        break;
                    }
                    Err(e) => {
                        let provider_err = match &e {
                            reqwest_eventsource::Error::InvalidStatusCode(status, response) => {
                                let status_code = status.as_u16();
                                let retry_after_str = response
                                    .headers()
                                    .get("retry-after")
                                    .and_then(|v| v.to_str().ok())
                                    .map(String::from);
                                let mut err = crate::provider::error::classify_status(
                                    status_code,
                                    &e.to_string(),
                                    &provider_name,
                                    &model_name,
                                );
                                if let (
                                    Some(retry_str),
                                    crate::provider::error::ProviderError::RateLimit { retry_after, .. },
                                ) = (&retry_after_str, &mut err)
                                {
                                    *retry_after = crate::provider::retry::parse_retry_after(retry_str);
                                }
                                err
                            }
                            _ => crate::provider::error::ProviderError::Network {
                                message: e.to_string(),
                            },
                        };
                        yield Err(anyhow::anyhow!("{}", provider_err.user_message()));
                        break;
                    }
                }
            }
        })
    }

    fn name(&self) -> &str {
        "anthropic"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn list_models(&self) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>>> + Send + '_>> {
        Box::pin(async {
            // Anthropic has no public /models endpoint — return hardcoded list
            Ok(vec![
                ModelInfo {
                    id: "claude-sonnet-4-6".into(),
                    name: "Claude Sonnet 4.6".into(),
                    context_window: Some(200_000),
                    supports_tools: true,
                    supports_vision: true,
                    supports_thinking: true,
                },
                ModelInfo {
                    id: "claude-haiku-4-5-20251001".into(),
                    name: "Claude Haiku 4.5".into(),
                    context_window: Some(200_000),
                    supports_tools: true,
                    supports_vision: true,
                    supports_thinking: true,
                },
                ModelInfo {
                    id: "claude-opus-4-6".into(),
                    name: "Claude Opus 4.6".into(),
                    context_window: Some(200_000),
                    supports_tools: true,
                    supports_vision: true,
                    supports_thinking: true,
                },
            ])
        })
    }

    fn set_thinking_mode(&self, mode: ThinkingMode) {
        self.thinking_mode
            .store(mode as u8, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn convert_content_handles_image_block() {
        let provider = AnthropicProvider::new("test-key".into(), "test-model".into());

        let content = json!([
            {
                "type": "image",
                "media_type": "image/jpeg",
                "data": "abc123base64data"
            }
        ]);

        let blocks = provider.convert_content(&content);
        assert_eq!(blocks.len(), 1);

        let serialized = serde_json::to_value(&blocks[0]).unwrap();
        assert_eq!(serialized["type"], "image");
        assert_eq!(serialized["source"]["type"], "base64");
        assert_eq!(serialized["source"]["media_type"], "image/jpeg");
        assert_eq!(serialized["source"]["data"], "abc123base64data");
    }

    #[test]
    fn convert_content_image_defaults_to_png() {
        let provider = AnthropicProvider::new("test-key".into(), "test-model".into());

        let content = json!([
            {
                "type": "image",
                "data": "somedata"
            }
        ]);

        let blocks = provider.convert_content(&content);
        assert_eq!(blocks.len(), 1);

        let serialized = serde_json::to_value(&blocks[0]).unwrap();
        assert_eq!(serialized["source"]["media_type"], "image/png");
    }

    #[test]
    fn convert_content_mixed_text_and_image() {
        let provider = AnthropicProvider::new("test-key".into(), "test-model".into());

        let content = json!([
            { "type": "text", "text": "Look at this image:" },
            { "type": "image", "media_type": "image/png", "data": "iVBOR..." }
        ]);

        let blocks = provider.convert_content(&content);
        assert_eq!(blocks.len(), 2);

        let s0 = serde_json::to_value(&blocks[0]).unwrap();
        assert_eq!(s0["type"], "text");
        assert_eq!(s0["text"], "Look at this image:");

        let s1 = serde_json::to_value(&blocks[1]).unwrap();
        assert_eq!(s1["type"], "image");
        assert_eq!(s1["source"]["type"], "base64");
    }
}
