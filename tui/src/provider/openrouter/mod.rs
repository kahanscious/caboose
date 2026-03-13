//! OpenRouter provider — OpenAI-compatible API with multi-model access.

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

const DEFAULT_API_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct OpenRouterProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
    thinking_mode: std::sync::atomic::AtomicU8,
}

impl OpenRouterProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: DEFAULT_API_URL.to_string(),
            client: reqwest::Client::new(),
            thinking_mode: std::sync::atomic::AtomicU8::new(0),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Fetch models with pricing from OpenRouter API.
    pub async fn list_models_with_pricing(
        &self,
    ) -> Result<(
        Vec<ModelInfo>,
        Vec<(String, crate::provider::pricing::ModelPricing)>,
    )> {
        let url = format!("{}/models", self.base_url);
        let resp: OpenRouterModelsResponse = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?
            .json()
            .await?;

        let mut models = Vec::new();
        let mut pricing = Vec::new();

        for m in resp.data {
            models.push(ModelInfo {
                id: m.id.clone(),
                name: m.name.unwrap_or_else(|| m.id.clone()),
                context_window: m.context_length,
                supports_tools: m
                    .supported_parameters
                    .as_ref()
                    .map(|params| params.iter().any(|p| p == "tools"))
                    .unwrap_or(true),
                supports_vision: m
                    .supported_parameters
                    .as_ref()
                    .map(|params| params.iter().any(|p| p == "images"))
                    .unwrap_or(false),
                supports_thinking: m
                    .supported_parameters
                    .as_ref()
                    .map(|params| {
                        params.iter().any(|p| p == "reasoning" || p == "reasoning_content")
                    })
                    .unwrap_or(false),
            });
            if let Some(p) = m.pricing {
                let input_per_token: f64 =
                    p.prompt.as_deref().unwrap_or("0").parse().unwrap_or(0.0);
                let output_per_token: f64 = p
                    .completion
                    .as_deref()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0.0);
                if input_per_token > 0.0 || output_per_token > 0.0 {
                    pricing.push((
                        m.id,
                        crate::provider::pricing::ModelPricing {
                            input_per_m: input_per_token * 1_000_000.0,
                            output_per_m: output_per_token * 1_000_000.0,
                        },
                    ));
                }
            }
        }

        Ok((models, pricing))
    }

    fn build_request(&self, messages: &[Message], tools: &[ToolDefinition]) -> ChatRequest {
        // Reuse OpenAI message format — OpenRouter is API-compatible
        let chat_messages: Vec<ChatMessage> = messages.iter().map(convert_message).collect();

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
}

/// Convert a domain Message to an OpenAI-format ChatMessage.
fn convert_message(msg: &Message) -> ChatMessage {
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
                                    arguments: serde_json::to_string(&input).unwrap_or_default(),
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
        "user" => ChatMessage {
            role: "user".to_string(),
            content: Some(msg.content.clone()),
            tool_calls: None,
            tool_call_id: None,
        },
        _ => ChatMessage {
            role: msg.role.clone(),
            content: Some(msg.content.clone()),
            tool_calls: None,
            tool_call_id: None,
        },
    }
}

/// Expand tool_result blocks in user messages to OpenAI "tool" role messages.
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

impl Provider for OpenRouterProvider {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn futures::Stream<Item = Result<StreamEvent>> + Send + 'static>> {
        let mut request = self.build_request(messages, tools);

        // Expand tool_result blocks
        let expanded = expand_tool_results(request.messages, messages);
        request.messages = expanded;

        let url = format!("{}/chat/completions", self.base_url);
        let api_key = self.api_key.clone();
        let client = self.client.clone();
        let provider_name = "openrouter".to_string();
        let model_name = self.model.clone();

        Box::pin(async_stream::stream! {
            let mut acc = SseAccumulator::new();

            let req = client
                .post(&url)
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .header("HTTP-Referer", "https://trycaboose.dev")
                .header("X-OpenRouter-Title", "Caboose")
                .json(&request);

            debug!("OpenRouter request to {url}");
            let mut es = match req.eventsource() {
                Ok(es) => es,
                Err(e) => {
                    yield Err(anyhow::anyhow!("OpenRouter connection failed: {e}"));
                    return;
                }
            };

            loop {
                match es.next().await {
                    Some(Ok(Event::Open)) => {
                        debug!("OpenRouter SSE connection opened");
                    }
                    Some(Ok(Event::Message(msg))) => {
                        // Check for mid-stream error responses
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&msg.data)
                            && let Some(err) = val.get("error") {
                                let err_msg = err.get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Unknown error");
                                yield Err(anyhow::anyhow!("OpenRouter error: {err_msg}"));
                                es.close();
                                return;
                            }

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
        "openrouter"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn list_models(&self) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>>> + Send + '_>> {
        Box::pin(async {
            let url = format!("{}/models", self.base_url);
            let resp: OpenRouterModelsResponse = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .send()
                .await?
                .json()
                .await?;

            let models: Vec<ModelInfo> = resp
                .data
                .into_iter()
                .map(|m| ModelInfo {
                    name: m.name.unwrap_or_else(|| m.id.clone()),
                    supports_tools: m
                        .supported_parameters
                        .as_ref()
                        .map(|params| params.iter().any(|p| p == "tools"))
                        .unwrap_or(true),
                    supports_vision: m
                        .supported_parameters
                        .as_ref()
                        .map(|params| params.iter().any(|p| p == "images"))
                        .unwrap_or(false),
                    supports_thinking: m
                        .supported_parameters
                        .as_ref()
                        .map(|params| {
                            params.iter().any(|p| p == "reasoning" || p == "reasoning_content")
                        })
                        .unwrap_or(false),
                    id: m.id,
                    context_window: m.context_length,
                })
                .collect();
            Ok(models)
        })
    }

    fn set_thinking_mode(&self, mode: ThinkingMode) {
        self.thinking_mode
            .store(mode as u8, std::sync::atomic::Ordering::Relaxed);
    }
}
