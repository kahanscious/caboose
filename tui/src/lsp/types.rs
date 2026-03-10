//! JSON-RPC framing and LSP protocol types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request.
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: i64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: i64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 notification (no id).
#[derive(Debug, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }
}

/// A message read from the LSP server (could be response or notification).
#[derive(Debug, Deserialize)]
pub struct JsonRpcMessage {
    pub id: Option<serde_json::Number>,
    pub method: Option<String>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
    pub params: Option<Value>,
}

impl JsonRpcMessage {
    /// True if this is a response (has id, no method).
    pub fn is_response(&self) -> bool {
        self.id.is_some() && self.method.is_none()
    }

    /// True if this is a notification (has method, no id).
    pub fn is_notification(&self) -> bool {
        self.method.is_some() && self.id.is_none()
    }
}

/// JSON-RPC error object.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

/// Encode a JSON-RPC message with Content-Length header for LSP transport.
pub fn encode_message(body: &[u8]) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let mut msg = header.into_bytes();
    msg.extend_from_slice(body);
    msg
}

/// LSP server lifecycle state.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum ServerState {
    Starting,
    Initializing,
    Ready,
    Error(String),
    ShuttingDown,
    Stopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_message_format() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let encoded = encode_message(body);
        let s = String::from_utf8(encoded).unwrap();
        assert!(s.starts_with("Content-Length: 46\r\n\r\n"));
        assert!(s.ends_with(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#));
    }

    #[test]
    fn request_serialization() {
        let req = JsonRpcRequest::new(
            1,
            "initialize",
            Some(serde_json::json!({"rootUri": "file:///tmp"})),
        );
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""jsonrpc":"2.0""#));
        assert!(json.contains(r#""id":1"#));
        assert!(json.contains(r#""method":"initialize""#));
    }

    #[test]
    fn notification_serialization_no_id() {
        let notif = JsonRpcNotification::new("initialized", None);
        let json = serde_json::to_string(&notif).unwrap();
        assert!(json.contains(r#""method":"initialized""#));
        assert!(!json.contains("id"));
    }

    #[test]
    fn message_is_response() {
        let msg: JsonRpcMessage = serde_json::from_str(r#"{"id":1,"result":{}}"#).unwrap();
        assert!(msg.is_response());
        assert!(!msg.is_notification());
    }

    #[test]
    fn message_is_notification() {
        let msg: JsonRpcMessage =
            serde_json::from_str(r#"{"method":"textDocument/publishDiagnostics","params":{}}"#)
                .unwrap();
        assert!(msg.is_notification());
        assert!(!msg.is_response());
    }
}
