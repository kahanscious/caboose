//! Anthropic Messages API types — request and response serde structs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Request types ──────────────────────────────────────────────

/// Top-level request body for POST /v1/messages.
#[derive(Debug, Serialize)]
pub struct ApiRequest {
    pub model: String,
    pub max_tokens: u32,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Vec<SystemBlock>>,
    pub messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ApiToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingParam>,
}

/// Extended thinking configuration.
#[derive(Debug, Serialize)]
pub struct ThinkingParam {
    #[serde(rename = "type")]
    pub thinking_type: String,
    pub budget_tokens: u32,
}

/// A system-prompt block (always type "text").
#[derive(Debug, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// A conversation message in the request.
#[derive(Debug, Serialize)]
pub struct ApiMessage {
    pub role: String,
    pub content: Vec<ApiContentBlock>,
}

/// A content block within a message.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ApiContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    #[serde(rename = "image")]
    Image { source: ImageSource },
}

/// Image source for base64-encoded image data.
#[derive(Debug, Serialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// A tool definition sent in the request.
#[derive(Debug, Serialize)]
pub struct ApiToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Cache control marker for prompt caching.
#[derive(Debug, Clone, Serialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub control_type: String,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self {
            control_type: "ephemeral".to_string(),
        }
    }
}

// ── Response / SSE event types ─────────────────────────────────

/// Deserialized SSE event data (the `data:` field of each SSE line).
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum SseData {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartPayload },
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        index: u32,
        content_block: ContentBlockInfo,
    },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: u32, delta: DeltaPayload },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDeltaInfo,
        usage: Option<DeltaUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: ErrorInfo },
}

/// Payload within `message_start`.
#[derive(Debug, Deserialize)]
pub struct MessageStartPayload {
    pub usage: Option<StartUsage>,
}

/// Token usage from the `message_start` event.
#[derive(Debug, Deserialize)]
pub struct StartUsage {
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    pub cache_creation_input_tokens: Option<u32>,
    pub cache_read_input_tokens: Option<u32>,
}

/// Info about a content block from `content_block_start`.
#[derive(Debug, Deserialize)]
pub struct ContentBlockInfo {
    #[serde(rename = "type")]
    pub block_type: String,
    pub id: Option<String>,
    pub name: Option<String>,
}

/// Delta payload variants from `content_block_delta`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
#[allow(dead_code)]
pub enum DeltaPayload {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
}

/// Top-level delta from `message_delta`.
#[derive(Debug, Deserialize)]
pub struct MessageDeltaInfo {
    pub stop_reason: Option<String>,
}

/// Usage info from `message_delta`.
#[derive(Debug, Deserialize)]
pub struct DeltaUsage {
    pub output_tokens: u32,
}

/// Error info from `error` events.
#[derive(Debug, Deserialize)]
pub struct ErrorInfo {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}
