//! JSON envelope types for the WebSocket protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A message received from a client over WebSocket.
#[derive(Debug, Deserialize)]
pub struct IncomingMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub payload: Option<Value>,
}

/// A message sent to a client over WebSocket.
#[derive(Debug, Serialize)]
pub struct OutgoingMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

impl OutgoingMessage {
    /// Create an event message (core → client push).
    pub fn event(id: &str, event: &str, payload: Value) -> Self {
        Self {
            id: id.to_string(),
            msg_type: "event".to_string(),
            event: Some(event.to_string()),
            command: None,
            payload: Some(payload),
        }
    }

    /// Create an auth message (e.g. pairing responses).
    pub fn auth(id: &str, event: &str, payload: Value) -> Self {
        Self {
            id: id.to_string(),
            msg_type: "auth".to_string(),
            event: Some(event.to_string()),
            command: None,
            payload: Some(payload),
        }
    }

    /// Create an error message.
    pub fn error(id: &str, message: &str) -> Self {
        Self {
            id: id.to_string(),
            msg_type: "error".to_string(),
            event: None,
            command: None,
            payload: Some(serde_json::json!({ "message": message })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_command_message() {
        let raw = r#"{"id":"abc","type":"command","command":"SendMessage","payload":{"text":"hello"}}"#;
        let msg: IncomingMessage = serde_json::from_str(raw).unwrap();
        assert_eq!(msg.id, "abc");
        assert_eq!(msg.msg_type, "command");
        assert_eq!(msg.command.as_deref(), Some("SendMessage"));
        let payload = msg.payload.unwrap();
        assert_eq!(payload["text"], "hello");
    }

    #[test]
    fn deserialize_auth_message() {
        let raw = r#"{"id":"xyz","type":"auth","command":"Pair","payload":{"code":"A3F8K2"}}"#;
        let msg: IncomingMessage = serde_json::from_str(raw).unwrap();
        assert_eq!(msg.id, "xyz");
        assert_eq!(msg.msg_type, "auth");
        assert_eq!(msg.command.as_deref(), Some("Pair"));
        let payload = msg.payload.unwrap();
        assert_eq!(payload["code"], "A3F8K2");
    }

    #[test]
    fn serialize_event_message() {
        let msg = OutgoingMessage::event("msg-1", "TextDelta", json!({"delta": "hello"}));
        let serialized = serde_json::to_string(&msg).unwrap();
        let v: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(v["type"], "event");
        assert_eq!(v["id"], "msg-1");
        assert_eq!(v["event"], "TextDelta");
    }

    #[test]
    fn serialize_error_message() {
        let msg = OutgoingMessage::error("err-1", "something went wrong");
        let serialized = serde_json::to_string(&msg).unwrap();
        let v: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["id"], "err-1");
        assert_eq!(v["payload"]["message"], "something went wrong");
    }
}
