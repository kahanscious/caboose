//! Google Gemini API request/response types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---- Request types ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentRequest {
    pub contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GeminiTool>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeminiContent {
    pub role: String,
    pub parts: Vec<GeminiPart>,
}

/// A part in a Gemini content block.
/// Gemini expects: {"text": "..."} or {"functionCall": {...}} or {"functionResponse": {...}}
/// We use untagged enum with wrapper structs to get the right JSON shape.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GeminiPart {
    Text {
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    FunctionCall {
        function_call: FunctionCallData,
    },
    #[serde(rename_all = "camelCase")]
    FunctionResponse {
        function_response: FunctionResponseData,
    },
    #[serde(rename_all = "camelCase")]
    InlineData {
        inline_data: InlineDataPayload,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionCallData {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionResponseData {
    pub name: String,
    pub response: Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineDataPayload {
    pub mime_type: String,
    pub data: String,
}

impl GeminiPart {
    pub fn text(s: impl Into<String>) -> Self {
        GeminiPart::Text { text: s.into() }
    }

    pub fn function_call(name: impl Into<String>, args: Value) -> Self {
        GeminiPart::FunctionCall {
            function_call: FunctionCallData {
                name: name.into(),
                args,
            },
        }
    }

    pub fn inline_data(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        GeminiPart::InlineData {
            inline_data: InlineDataPayload {
                mime_type: mime_type.into(),
                data: data.into(),
            },
        }
    }

    pub fn function_response(name: impl Into<String>, response: Value) -> Self {
        GeminiPart::FunctionResponse {
            function_response: FunctionResponseData {
                name: name.into(),
                response,
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiTool {
    pub function_declarations: Vec<FunctionDeclaration>,
}

#[derive(Debug, Serialize)]
pub struct FunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

// ---- SSE response types ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChunk {
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub usage_metadata: Option<UsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Option<CandidateContent>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CandidateContent {
    pub parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponsePart {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub function_call: Option<FunctionCallResponse>,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCallResponse {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: u32,
    pub candidates_token_count: Option<u32>,
}

// ---- Models endpoint ----

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub name: String,
    pub display_name: Option<String>,
    pub input_token_limit: Option<u32>,
}
