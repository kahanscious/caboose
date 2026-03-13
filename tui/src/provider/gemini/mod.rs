//! Gemini provider — Google Generative AI streaming API.

pub mod sse;
pub mod types;

use anyhow::Result;
use futures::StreamExt;
use reqwest_eventsource::{Event, RequestBuilderExt};
use std::future::Future;
use std::pin::Pin;
use tracing::{debug, warn};

use self::sse::SseAccumulator;
use self::types::*;
use super::{Message, ModelInfo, Provider, StreamEvent, ThinkingMode, ToolDefinition};

const DEFAULT_API_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct GeminiProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
    thinking_mode: std::sync::atomic::AtomicU8,
}

impl GeminiProvider {
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

    fn build_request(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> GenerateContentRequest {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    let text = if let Some(s) = msg.content.as_str() {
                        s.to_string()
                    } else {
                        msg.content.to_string()
                    };
                    system_instruction = Some(GeminiContent {
                        role: "user".to_string(), // Gemini system_instruction uses "user" role
                        parts: vec![GeminiPart::text(text)],
                    });
                }
                "user" => {
                    let parts = self.convert_user_parts(&msg.content);
                    contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts,
                    });
                }
                "assistant" => {
                    let parts = self.convert_assistant_parts(&msg.content);
                    contents.push(GeminiContent {
                        role: "model".to_string(),
                        parts,
                    });
                }
                _ => {
                    warn!(
                        role = msg.role.as_str(),
                        "Unknown message role for Gemini, skipping"
                    );
                }
            }
        }

        let gemini_tools = if tools.is_empty() {
            None
        } else {
            Some(vec![GeminiTool {
                function_declarations: tools
                    .iter()
                    .map(|t| FunctionDeclaration {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.input_schema.clone(),
                    })
                    .collect(),
            }])
        };

        let mode = ThinkingMode::from_u8(
            self.thinking_mode
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        let generation_config = match mode {
            ThinkingMode::Off => None,
            ThinkingMode::On => Some(GenerationConfig {
                thinking_config: Some(ThinkingConfig {
                    thinking_budget: 10_000,
                }),
            }),
        };

        GenerateContentRequest {
            contents,
            system_instruction,
            tools: gemini_tools,
            generation_config,
        }
    }

    fn convert_user_parts(&self, content: &serde_json::Value) -> Vec<GeminiPart> {
        if let Some(text) = content.as_str() {
            return vec![GeminiPart::text(text)];
        }

        if let Some(arr) = content.as_array() {
            let mut parts = Vec::new();
            for block in arr {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            parts.push(GeminiPart::text(text));
                        }
                    }
                    Some("image") => {
                        let mime_type = block
                            .get("media_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("image/png")
                            .to_string();
                        let data = block
                            .get("data")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        parts.push(GeminiPart::inline_data(mime_type, data));
                    }
                    Some("tool_result") => {
                        // Convert to Gemini functionResponse
                        let name = block
                            .get("tool_use_id") // We need the tool name, but we only have the ID
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let content_val = block
                            .get("content")
                            .cloned()
                            .unwrap_or(serde_json::Value::String(String::new()));
                        let response = serde_json::json!({ "result": content_val });
                        parts.push(GeminiPart::function_response(name, response));
                    }
                    _ => {}
                }
            }
            if !parts.is_empty() {
                return parts;
            }
        }

        vec![GeminiPart::text(content.to_string())]
    }

    fn convert_assistant_parts(&self, content: &serde_json::Value) -> Vec<GeminiPart> {
        if let Some(text) = content.as_str() {
            return vec![GeminiPart::text(text)];
        }

        if let Some(arr) = content.as_array() {
            let mut parts = Vec::new();
            for block in arr {
                match block.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            parts.push(GeminiPart::text(text));
                        }
                    }
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let input = block.get("input").cloned().unwrap_or_default();
                        parts.push(GeminiPart::function_call(name, input));
                    }
                    _ => {}
                }
            }
            if !parts.is_empty() {
                return parts;
            }
        }

        vec![GeminiPart::text(content.to_string())]
    }
}

impl Provider for GeminiProvider {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Pin<Box<dyn futures::Stream<Item = Result<StreamEvent>> + Send + 'static>> {
        let request = self.build_request(messages, tools);

        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url, self.model, self.api_key
        );
        let client = self.client.clone();
        let provider_name = "gemini".to_string();
        let model_name = self.model.clone();

        Box::pin(async_stream::stream! {
            let mut acc = SseAccumulator::new();

            let req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&request);

            debug!("Gemini request to {}", url.split('?').next().unwrap_or(&url));
            let mut es = req.eventsource().unwrap();

            loop {
                match es.next().await {
                    Some(Ok(Event::Open)) => {
                        debug!("Gemini SSE connection opened");
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
        "gemini"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn set_thinking_mode(&self, mode: ThinkingMode) {
        self.thinking_mode
            .store(mode as u8, std::sync::atomic::Ordering::Relaxed);
    }

    fn list_models(&self) -> Pin<Box<dyn Future<Output = Result<Vec<ModelInfo>>> + Send + '_>> {
        Box::pin(async {
            let url = format!("{}/models?key={}", self.base_url, self.api_key);
            let resp: ModelsResponse = self.client.get(&url).send().await?.json().await?;

            let models: Vec<ModelInfo> = resp
                .models
                .into_iter()
                .filter(|m| m.name.contains("gemini"))
                .map(|m| {
                    let id = m
                        .name
                        .strip_prefix("models/")
                        .unwrap_or(&m.name)
                        .to_string();
                    let thinking = id.starts_with("gemini-2.5");
                    ModelInfo {
                        name: m.display_name.unwrap_or_else(|| id.clone()),
                        id,
                        context_window: m.input_token_limit,
                        supports_tools: true,
                        supports_vision: true,
                        supports_thinking: thinking,
                    }
                })
                .collect();
            Ok(models)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_inline_data_serialization() {
        let part = GeminiPart::inline_data("image/webp", "AAAA");
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(
            json,
            json!({"inlineData": {"mimeType": "image/webp", "data": "AAAA"}})
        );
    }

    #[test]
    fn test_convert_user_parts_image_block() {
        let provider = GeminiProvider::new("test-key".into(), "gemini-pro".into());
        let content = json!([
            {"type": "text", "text": "What is this?"},
            {"type": "image", "media_type": "image/webp", "data": "AAAA"}
        ]);
        let parts = provider.convert_user_parts(&content);
        assert_eq!(parts.len(), 2);

        let json_parts: Vec<serde_json::Value> = parts
            .iter()
            .map(|p| serde_json::to_value(p).unwrap())
            .collect();
        assert_eq!(json_parts[0], json!({"text": "What is this?"}));
        assert_eq!(
            json_parts[1],
            json!({"inlineData": {"mimeType": "image/webp", "data": "AAAA"}})
        );
    }

    #[test]
    fn test_convert_user_parts_image_default_mime() {
        let provider = GeminiProvider::new("test-key".into(), "gemini-pro".into());
        let content = json!([
            {"type": "image", "data": "BBBB"}
        ]);
        let parts = provider.convert_user_parts(&content);
        assert_eq!(parts.len(), 1);

        let json_val = serde_json::to_value(&parts[0]).unwrap();
        assert_eq!(
            json_val,
            json!({"inlineData": {"mimeType": "image/png", "data": "BBBB"}})
        );
    }
}
