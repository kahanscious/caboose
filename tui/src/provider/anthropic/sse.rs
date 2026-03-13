//! SSE event parser — converts raw Anthropic SSE events into StreamEvents.

use anyhow::{Context, Result};
use tracing::warn;

use super::types::{DeltaPayload, SseData};
use crate::provider::StreamEvent;

/// Accumulates state across SSE events within a single response.
///
/// Tool call arguments arrive as incremental `partial_json` chunks that must
/// be buffered and parsed as a whole when `content_block_stop` fires.
pub struct SseAccumulator {
    // Token tracking
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,

    // Tool call accumulation
    current_tool_id: Option<String>,
    current_tool_name: Option<String>,
    json_buffer: String,

    // Message metadata
    stop_reason: Option<String>,
}

impl SseAccumulator {
    pub fn new() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            current_tool_id: None,
            current_tool_name: None,
            json_buffer: String::new(),
            stop_reason: None,
        }
    }

    /// Process a single SSE data payload. Returns `Some(StreamEvent)` when
    /// the event produces a domain event, `None` when it's internal bookkeeping.
    pub fn process(&mut self, data: &str) -> Result<Option<StreamEvent>> {
        let sse: SseData = serde_json::from_str(data).context("Failed to parse SSE data")?;

        match sse {
            SseData::MessageStart { message } => {
                if let Some(usage) = message.usage {
                    self.input_tokens = usage.input_tokens;
                    self.output_tokens = usage.output_tokens;
                    self.cache_creation_tokens = usage.cache_creation_input_tokens.unwrap_or(0);
                    self.cache_read_tokens = usage.cache_read_input_tokens.unwrap_or(0);
                }
                Ok(None)
            }

            SseData::ContentBlockStart { content_block, .. } => {
                if content_block.block_type == "tool_use" {
                    self.current_tool_id = content_block.id;
                    self.current_tool_name = content_block.name;
                    self.json_buffer.clear();
                }
                Ok(None)
            }

            SseData::ContentBlockDelta { delta, .. } => match delta {
                DeltaPayload::TextDelta { text } => Ok(Some(StreamEvent::TextDelta(text))),
                DeltaPayload::InputJsonDelta { partial_json } => {
                    self.json_buffer.push_str(&partial_json);
                    Ok(None)
                }
                DeltaPayload::ThinkingDelta { thinking } => {
                    Ok(Some(StreamEvent::ThinkingDelta(thinking)))
                }
                DeltaPayload::SignatureDelta { .. } => Ok(None),
            },

            SseData::ContentBlockStop { .. } => {
                // If we were accumulating a tool call, emit it now
                if let (Some(id), Some(name)) =
                    (self.current_tool_id.take(), self.current_tool_name.take())
                {
                    let arguments = std::mem::take(&mut self.json_buffer);
                    Ok(Some(StreamEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    }))
                } else {
                    Ok(None)
                }
            }

            SseData::MessageDelta { delta, usage } => {
                self.stop_reason = delta.stop_reason;
                if let Some(u) = usage {
                    self.output_tokens = u.output_tokens;
                }
                Ok(None)
            }

            SseData::MessageStop => Ok(Some(StreamEvent::Done {
                input_tokens: Some(self.input_tokens),
                output_tokens: Some(self.output_tokens),
                cache_read_tokens: if self.cache_read_tokens > 0 {
                    Some(self.cache_read_tokens)
                } else {
                    None
                },
                cache_creation_tokens: if self.cache_creation_tokens > 0 {
                    Some(self.cache_creation_tokens)
                } else {
                    None
                },
            })),

            SseData::Ping => Ok(None),

            SseData::Error { error } => {
                warn!(
                    error_type = %error.error_type,
                    message = %error.message,
                    "Anthropic SSE error"
                );
                Ok(Some(StreamEvent::Error(format!(
                    "{}: {}",
                    error.error_type, error.message
                ))))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_start_captures_tokens() {
        let mut acc = SseAccumulator::new();
        let data = r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"output_tokens":1}}}"#;
        let result = acc.process(data).unwrap();
        assert!(result.is_none());
        assert_eq!(acc.input_tokens, 25);
        assert_eq!(acc.output_tokens, 1);
    }

    #[test]
    fn test_message_start_with_cache_tokens() {
        let mut acc = SseAccumulator::new();
        let data = r#"{"type":"message_start","message":{"usage":{"input_tokens":100,"output_tokens":0,"cache_creation_input_tokens":50,"cache_read_input_tokens":30}}}"#;
        let result = acc.process(data).unwrap();
        assert!(result.is_none());
        assert_eq!(acc.cache_creation_tokens, 50);
        assert_eq!(acc.cache_read_tokens, 30);
    }

    #[test]
    fn test_text_delta_yields_stream_event() {
        let mut acc = SseAccumulator::new();
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let result = acc.process(data).unwrap();
        match result {
            Some(StreamEvent::TextDelta(text)) => assert_eq!(text, "Hello"),
            other => panic!("Expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn test_tool_call_accumulation() {
        let mut acc = SseAccumulator::new();

        // content_block_start with tool_use
        let start = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01abc","name":"read_file","input":{}}}"#;
        assert!(acc.process(start).unwrap().is_none());

        // Partial JSON deltas
        let delta1 = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#;
        assert!(acc.process(delta1).unwrap().is_none());

        let delta2 = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#;
        assert!(acc.process(delta2).unwrap().is_none());

        let delta3 = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"src/main.rs\"}"}}"#;
        assert!(acc.process(delta3).unwrap().is_none());

        // content_block_stop emits the tool call
        let stop = r#"{"type":"content_block_stop","index":1}"#;
        let result = acc.process(stop).unwrap();
        match result {
            Some(StreamEvent::ToolCall {
                id,
                name,
                arguments,
            }) => {
                assert_eq!(id, "toolu_01abc");
                assert_eq!(name, "read_file");
                assert_eq!(arguments, r#"{"path": "src/main.rs"}"#);
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn test_text_block_stop_yields_none() {
        let mut acc = SseAccumulator::new();
        // Start a text block (not a tool)
        let start =
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
        assert!(acc.process(start).unwrap().is_none());
        // Stop it — no tool was active, should be None
        let stop = r#"{"type":"content_block_stop","index":0}"#;
        assert!(acc.process(stop).unwrap().is_none());
    }

    #[test]
    fn test_message_delta_and_stop() {
        let mut acc = SseAccumulator::new();
        acc.input_tokens = 100;

        let delta = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":42}}"#;
        assert!(acc.process(delta).unwrap().is_none());
        assert_eq!(acc.output_tokens, 42);

        let stop = r#"{"type":"message_stop"}"#;
        let result = acc.process(stop).unwrap();
        match result {
            Some(StreamEvent::Done {
                input_tokens,
                output_tokens,
                ..
            }) => {
                assert_eq!(input_tokens, Some(100));
                assert_eq!(output_tokens, Some(42));
            }
            other => panic!("Expected Done, got {:?}", other),
        }
    }

    #[test]
    fn test_ping_ignored() {
        let mut acc = SseAccumulator::new();
        let data = r#"{"type":"ping"}"#;
        assert!(acc.process(data).unwrap().is_none());
    }

    #[test]
    fn test_error_event() {
        let mut acc = SseAccumulator::new();
        let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let result = acc.process(data).unwrap();
        match result {
            Some(StreamEvent::Error(msg)) => {
                assert!(msg.contains("overloaded_error"));
                assert!(msg.contains("Overloaded"));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_full_text_stream_sequence() {
        let mut acc = SseAccumulator::new();

        // message_start
        let ms = r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":25,"output_tokens":1}}}"#;
        assert!(acc.process(ms).unwrap().is_none());

        // content_block_start (text)
        let cbs =
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
        assert!(acc.process(cbs).unwrap().is_none());

        // Two text deltas
        let d1 = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        assert!(matches!(
            acc.process(d1).unwrap(),
            Some(StreamEvent::TextDelta(_))
        ));

        let d2 =
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"!"}}"#;
        assert!(matches!(
            acc.process(d2).unwrap(),
            Some(StreamEvent::TextDelta(_))
        ));

        // content_block_stop
        let cbe = r#"{"type":"content_block_stop","index":0}"#;
        assert!(acc.process(cbe).unwrap().is_none());

        // message_delta
        let md = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":15}}"#;
        assert!(acc.process(md).unwrap().is_none());

        // message_stop
        let me = r#"{"type":"message_stop"}"#;
        assert!(matches!(
            acc.process(me).unwrap(),
            Some(StreamEvent::Done { .. })
        ));
    }

    #[test]
    fn test_thinking_delta_yields_stream_event() {
        let mut acc = SseAccumulator::new();
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think about this"}}"#;
        let result = acc.process(data).unwrap();
        match result {
            Some(StreamEvent::ThinkingDelta(text)) => assert_eq!(text, "Let me think about this"),
            other => panic!("Expected ThinkingDelta, got {:?}", other),
        }
    }

    #[test]
    fn malformed_json_data_returns_error() {
        let mut acc = SseAccumulator::new();
        let result = acc.process("not valid json {{{");
        assert!(result.is_err());
    }

    #[test]
    fn empty_text_delta_yields_event() {
        let mut acc = SseAccumulator::new();
        let result = acc.process(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}"#,
        ).unwrap();
        match result {
            Some(StreamEvent::TextDelta(s)) => assert!(s.is_empty()),
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn message_delta_missing_usage_defaults_to_zero() {
        let mut acc = SseAccumulator::new();
        let result = acc
            .process(r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#)
            .unwrap();
        // Should not panic; output_tokens stays at 0
        assert!(result.is_none());
        assert_eq!(acc.output_tokens, 0);
    }
}
