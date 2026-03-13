//! OpenAI provider — Chat Completions API with SSE streaming.

pub mod sse;
pub mod types;

use anyhow::Result;
use futures::StreamExt;
use reqwest_eventsource::{Event, RequestBuilderExt};
use std::future::Future;
use std::pin::Pin;
use tracing::debug;

use self::sse::SseAccumulator;
use self::types::*;
use super::{Message, ModelInfo, Provider, StreamEvent, ThinkingMode, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.openai.com/v1";
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct OpenAiProvider {
    api_key: String,
    model: String,
    base_url: String,
    provider_name: String,
    model_filter: Option<fn(&str) -> bool>,
    client: reqwest::Client,
    thinking_mode: std::sync::atomic::AtomicU8,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: DEFAULT_API_URL.to_string(),
            provider_name: "openai".to_string(),
            model_filter: Some(|id: &str| id.starts_with("gpt-") || id.starts_with("o")),
            client: reqwest::Client::new(),
            thinking_mode: std::sync::atomic::AtomicU8::new(0),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    pub fn with_provider_name(mut self, name: String) -> Self {
        self.provider_name = name;
        self
    }

    pub fn with_model_filter(mut self, filter: Option<fn(&str) -> bool>) -> Self {
        self.model_filter = filter;
        self
    }

    fn build_request(&self, messages: &[Message], tools: &[ToolDefinition]) -> ChatRequest {
        let chat_messages: Vec<ChatMessage> =
            messages.iter().map(|m| self.convert_message(m)).collect();

        let chat_tools = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| ChatTool {
                        tool_type: "function".to_string(),
                        function: ChatFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.input_schema.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let mode = ThinkingMode::from_u8(
            self.thinking_mode
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let reasoning_effort = match mode {
            ThinkingMode::Off => None,
            ThinkingMode::On => Some("high".to_string()),
        };

        ChatRequest {
            model: self.model.clone(),
            messages: chat_messages,
            stream: true,
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
            tools: chat_tools,
            max_tokens: Some(DEFAULT_MAX_TOKENS),
            reasoning_effort,
        }
    }

    fn convert_message(&self, msg: &Message) -> ChatMessage {
        match msg.role.as_str() {
            "system" => ChatMessage {
                role: "system".to_string(),
                content: Some(msg.content.clone()),
                tool_calls: None,
                tool_call_id: None,
            },
            "assistant" => {
                if let Some(arr) = msg.content.as_array() {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for block in arr {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                    text_parts.push(t.to_string());
                                }
                            }
                            Some("tool_use") => {
                                let id = block
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let name = block
                                    .get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let input = block.get("input").cloned().unwrap_or_default();
                                tool_calls.push(ChatToolCall {
                                    id,
                                    call_type: "function".to_string(),
                                    function: ChatFunctionCall {
                                        name,
                                        arguments: serde_json::to_string(&input)
                                            .unwrap_or_default(),
                                    },
                                });
                            }
                            _ => {}
                        }
                    }

                    ChatMessage {
                        role: "assistant".to_string(),
                        content: if text_parts.is_empty() {
                            None
                        } else {
                            Some(serde_json::Value::String(text_parts.join("")))
                        },
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                    }
                } else {
                    ChatMessage {
                        role: "assistant".to_string(),
                        content: Some(msg.content.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    }
                }
            }
            "user" => {
                // User messages may contain tool_result blocks — handled by expand_tool_results.
                // Image blocks need conversion to OpenAI's image_url format.
                if let Some(arr) = msg.content.as_array() {
                    let has_image = arr
                        .iter()
                        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("image"));
                    if has_image {
                        let mut parts = Vec::new();
                        for block in arr {
                            match block.get("type").and_then(|t| t.as_str()) {
                                Some("text") => {
                                    let text = block
                                        .get("text")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default();
                                    parts.push(serde_json::json!({
                                        "type": "text",
                                        "text": text
                                    }));
                                }
                                Some("image") => {
                                    let media_type = block
                                        .get("media_type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("image/png");
                                    let data = block
                                        .get("data")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or_default();
                                    parts.push(serde_json::json!({
                                        "type": "image_url",
                                        "image_url": {
                                            "url": format!("data:{media_type};base64,{data}")
                                        }
                                    }));
                                }
                                _ => {
                                    // Pass through other block types (e.g. tool_result)
                                    parts.push(block.clone());
                                }
                            }
                        }
                        ChatMessage {
                            role: "user".to_string(),
                            content: Some(serde_json::Value::Array(parts)),
                            tool_calls: None,
                            tool_call_id: None,
                        }
                    } else {
                        ChatMessage {
                            role: "user".to_string(),
                            content: Some(msg.content.clone()),
                            tool_calls: None,
                            tool_call_id: None,
                        }
                    }
                } else {
                    ChatMessage {
                        role: "user".to_string(),
                        content: Some(msg.content.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    }
                }
            }
            _ => ChatMessage {
                role: msg.role.clone(),
                content: Some(msg.content.clone()),
                tool_calls: None,
                tool_call_id: None,
            },
        }
    }

    /// Convert provider::Messages that contain tool_result blocks into the
    /// OpenAI format (separate "tool" role messages).
    fn expand_tool_results(messages: Vec<ChatMessage>, source: &[Message]) -> Vec<ChatMessage> {
        let mut result = Vec::new();

        for (i, msg) in messages.into_iter().enumerate() {
            if msg.role == "user" {
                if let Some(src) = source.get(i)
                    && let Some(arr) = src.content.as_array()
                {
                    let mut has_tool_results = false;
                    for block in arr {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            has_tool_results = true;
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let content = block
                                .get("content")
                                .cloned()
                                .unwrap_or(serde_json::Value::String(String::new()));
                            result.push(ChatMessage {
                                role: "tool".to_string(),
                                content: Some(content),
                                tool_calls: None,
                                tool_call_id: Some(tool_use_id),
                            });
                        }
                    }
                    if !has_tool_results {
                        result.push(msg);
                    }
                    continue;
                }
                result.push(msg);
            } else {
                result.push(msg);
            }
        }

        result
    }
}

impl Provider for OpenAiProvider {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn futures::Stream<Item = Result<StreamEvent>> + Send + 'static>> {
        let mut request = self.build_request(messages, tools);

        // Expand tool_result blocks into separate "tool" role messages
        let expanded = Self::expand_tool_results(request.messages, messages);
        request.messages = expanded;

        let url = format!("{}/chat/completions", self.base_url);
        let api_key = self.api_key.clone();
        let client = self.client.clone();
        let provider_name = self.provider_name.clone();
        let model_name = self.model.clone();

        Box::pin(async_stream::stream! {
            let mut acc = SseAccumulator::new();

            let req = client
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .json(&request);

            debug!("OpenAI request to {url}");
            let mut es = req.eventsource().unwrap();

            loop {
                match es.next().await {
                    Some(Ok(Event::Open)) => {
                        debug!("OpenAI SSE connection opened");
                    }
                    Some(Ok(Event::Message(msg))) => {
                        let events = acc.process(&msg.data);
                        for event in events {
                            let is_done = matches!(event, StreamEvent::Done { .. });
                            yield Ok(event);
                            if is_done {
                                es.close();
                                return;
                            }
                        }
                    }
                    Some(Err(reqwest_eventsource::Error::StreamEnded)) => {
                        break;
                    }
                    Some(Err(e)) => {
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
                    None => break,
                }
            }
        })
    }

    fn name(&self) -> &str {
        &self.provider_name
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn list_models(&self) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>>> + Send + '_>> {
        Box::pin(async {
            let url = format!("{}/models", self.base_url);
            let resp: ModelsResponse = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .send()
                .await?
                .json()
                .await?;

            let iter = resp.data.into_iter();
            let filtered: Box<dyn Iterator<Item = _>> = if let Some(filter) = self.model_filter {
                Box::new(iter.filter(move |m| filter(&m.id)))
            } else {
                Box::new(iter)
            };

            let mut models: Vec<ModelInfo> = filtered
                .map(|m| {
                    let vision = m.id.contains("gpt-4o")
                        || m.id.contains("gpt-4-turbo")
                        || m.id.contains("gpt-4-vision");
                    let thinking = m.id.starts_with("o1")
                        || m.id.starts_with("o3")
                        || m.id.starts_with("o4");
                    ModelInfo {
                        name: m.id.clone(),
                        id: m.id,
                        context_window: None,
                        supports_tools: true,
                        supports_vision: vision,
                        supports_thinking: thinking,
                    }
                })
                .collect();
            models.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(models)
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

    fn test_provider() -> OpenAiProvider {
        OpenAiProvider::new("test-key".to_string(), "gpt-4o".to_string())
    }

    #[test]
    fn convert_message_handles_image_block() {
        let provider = test_provider();
        let msg = Message {
            role: "user".to_string(),
            content: json!([
                {"type": "text", "text": "What is in this image?"},
                {"type": "image", "media_type": "image/jpeg", "data": "abc123base64data"}
            ]),
        };

        let result = provider.convert_message(&msg);
        assert_eq!(result.role, "user");

        let content = result.content.expect("content should be present");
        let parts = content.as_array().expect("content should be an array");
        assert_eq!(parts.len(), 2);

        // First part: text
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "What is in this image?");

        // Second part: image_url with data URL
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(
            parts[1]["image_url"]["url"],
            "data:image/jpeg;base64,abc123base64data"
        );
    }

    #[test]
    fn convert_message_image_defaults_to_png() {
        let provider = test_provider();
        let msg = Message {
            role: "user".to_string(),
            content: json!([
                {"type": "image", "data": "pngdata"}
            ]),
        };

        let result = provider.convert_message(&msg);
        let content = result.content.unwrap();
        let parts = content.as_array().unwrap();
        assert_eq!(
            parts[0]["image_url"]["url"],
            "data:image/png;base64,pngdata"
        );
    }

    #[test]
    fn convert_message_user_without_images_passes_through() {
        let provider = test_provider();
        let msg = Message {
            role: "user".to_string(),
            content: json!([
                {"type": "text", "text": "Hello"},
                {"type": "tool_result", "tool_use_id": "123", "content": "ok"}
            ]),
        };

        let result = provider.convert_message(&msg);
        // Without image blocks, content passes through as-is
        let content = result.content.unwrap();
        let arr = content.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[1]["type"], "tool_result");
    }
}
