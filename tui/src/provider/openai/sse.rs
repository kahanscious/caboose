//! OpenAI SSE accumulator — parses chat completion stream chunks.

use std::collections::HashMap;
use tracing::warn;

use super::types::{ChatChunk, ChunkToolCall};
use crate::provider::StreamEvent;

/// Buffers in-progress tool calls (OpenAI streams tool args incrementally).
struct ToolBuffer {
    id: String,
    name: String,
    arguments: String,
}

/// Accumulates SSE chunks from the OpenAI streaming API.
pub struct SseAccumulator {
    tool_buffers: HashMap<u32, ToolBuffer>,
    input_tokens: u32,
    output_tokens: u32,
}

impl SseAccumulator {
    pub fn new() -> Self {
        Self {
            tool_buffers: HashMap::new(),
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    /// Process a raw SSE data string. Returns zero or more StreamEvents.
    pub fn process(&mut self, data: &str) -> Vec<StreamEvent> {
        if data == "[DONE]" {
            return vec![StreamEvent::Done {
                input_tokens: Some(self.input_tokens),
                output_tokens: Some(self.output_tokens),
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }];
        }

        let chunk: ChatChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to parse OpenAI SSE chunk: {e}");
                return vec![];
            }
        };

        // Capture usage if present (final chunk)
        if let Some(usage) = &chunk.usage {
            self.input_tokens = usage.prompt_tokens;
            self.output_tokens = usage.completion_tokens;
        }

        let mut events = Vec::new();

        for choice in &chunk.choices {
            // Reasoning delta (OpenAI o1/o3, OpenRouter, DeepSeek)
            let reasoning = choice
                .delta
                .reasoning
                .as_deref()
                .or(choice.delta.reasoning_content.as_deref());
            if let Some(text) = reasoning
                && !text.is_empty()
            {
                events.push(StreamEvent::ThinkingDelta(text.to_string()));
            }

            // Text delta
            if let Some(ref content) = choice.delta.content
                && !content.is_empty()
            {
                events.push(StreamEvent::TextDelta(content.clone()));
            }

            // Tool call deltas
            if let Some(ref tool_calls) = choice.delta.tool_calls {
                for tc in tool_calls {
                    self.accumulate_tool_call(tc);
                }
            }

            // Finish reason — emit buffered tool calls
            if let Some(ref reason) = choice.finish_reason
                && (reason == "tool_calls" || reason == "stop")
            {
                events.extend(self.flush_tool_calls());
            }
        }

        events
    }

    fn accumulate_tool_call(&mut self, tc: &ChunkToolCall) {
        let buf = self
            .tool_buffers
            .entry(tc.index)
            .or_insert_with(|| ToolBuffer {
                id: String::new(),
                name: String::new(),
                arguments: String::new(),
            });

        if let Some(ref id) = tc.id {
            buf.id = id.clone();
        }
        if let Some(ref func) = tc.function {
            if let Some(ref name) = func.name {
                buf.name = name.clone();
            }
            if let Some(ref args) = func.arguments {
                buf.arguments.push_str(args);
            }
        }
    }

    fn flush_tool_calls(&mut self) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        let mut indices: Vec<u32> = self.tool_buffers.keys().copied().collect();
        indices.sort();
        for idx in indices {
            if let Some(buf) = self.tool_buffers.remove(&idx)
                && !buf.name.is_empty()
            {
                events.push(StreamEvent::ToolCall {
                    id: buf.id,
                    name: buf.name,
                    arguments: buf.arguments,
                });
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_delta() {
        let mut acc = SseAccumulator::new();
        let chunk = r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let events = acc.process(chunk);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::TextDelta(t) if t == "Hello"));
    }

    #[test]
    fn test_tool_call_accumulation() {
        let mut acc = SseAccumulator::new();

        // First chunk: tool call start
        let events = acc.process(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"read_file","arguments":"{\"pa"}}]},"finish_reason":null}]}"#
        );
        assert!(events.is_empty());

        // Second chunk: more arguments
        let events = acc.process(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"/src\"}"}}]},"finish_reason":null}]}"#
        );
        assert!(events.is_empty());

        // Third chunk: finish
        let events = acc.process(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_abc");
                assert_eq!(name, "read_file");
                assert_eq!(arguments, r#"{"path":"/src"}"#);
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn test_usage_in_final_chunk() {
        let mut acc = SseAccumulator::new();
        acc.process(r#"{"choices":[{"delta":{"content":"Hi"},"finish_reason":null}]}"#);
        acc.process(r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":50}}"#);
        let events = acc.process("[DONE]");
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, Some(100));
                assert_eq!(*output_tokens, Some(50));
            }
            _ => panic!("Expected Done"),
        }
    }

    #[test]
    fn test_done_signal() {
        let mut acc = SseAccumulator::new();
        let events = acc.process("[DONE]");
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::Done { .. }));
    }

    #[test]
    fn test_multiple_concurrent_tool_calls() {
        let mut acc = SseAccumulator::new();

        // Two tool calls streamed simultaneously
        acc.process(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read","arguments":"{}"}}]},"finish_reason":null}]}"#
        );
        acc.process(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_2","function":{"name":"write","arguments":"{}"}}]},"finish_reason":null}]}"#
        );

        let events = acc.process(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#);
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StreamEvent::ToolCall { name, .. } if name == "read"));
        assert!(matches!(&events[1], StreamEvent::ToolCall { name, .. } if name == "write"));
    }

    #[test]
    fn empty_delta_content_yields_nothing() {
        let mut acc = SseAccumulator::new();
        let events = acc.process(r#"{"choices":[{"delta":{"content":""},"index":0}]}"#);
        // Empty content is filtered out
        assert!(events.is_empty());
    }

    #[test]
    fn missing_usage_in_final_chunk_defaults_to_zero() {
        let mut acc = SseAccumulator::new();
        // Finish without any usage chunk
        acc.process(r#"{"choices":[{"delta":{},"finish_reason":"stop","index":0}]}"#);
        let events = acc.process("[DONE]");
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, Some(0));
                assert_eq!(*output_tokens, Some(0));
            }
            _ => panic!("Expected Done"),
        }
    }

    #[test]
    fn done_signal_without_prior_usage() {
        let mut acc = SseAccumulator::new();
        let events = acc.process("[DONE]");
        match &events[0] {
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, Some(0));
                assert_eq!(*output_tokens, Some(0));
            }
            _ => panic!("Expected Done"),
        }
    }

    #[test]
    fn reasoning_field_emits_thinking_delta() {
        let mut acc = SseAccumulator::new();
        let chunk = r#"{"choices":[{"delta":{"reasoning":"Let me think..."},"finish_reason":null}]}"#;
        let events = acc.process(chunk);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::ThinkingDelta(t) if t == "Let me think..."));
    }

    #[test]
    fn reasoning_content_field_emits_thinking_delta() {
        let mut acc = SseAccumulator::new();
        let chunk =
            r#"{"choices":[{"delta":{"reasoning_content":"Step 1..."},"finish_reason":null}]}"#;
        let events = acc.process(chunk);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::ThinkingDelta(t) if t == "Step 1..."));
    }

    #[test]
    fn reasoning_preferred_over_reasoning_content() {
        let mut acc = SseAccumulator::new();
        let chunk = r#"{"choices":[{"delta":{"reasoning":"primary","reasoning_content":"fallback"},"finish_reason":null}]}"#;
        let events = acc.process(chunk);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::ThinkingDelta(t) if t == "primary"));
    }

    #[test]
    fn reasoning_and_content_both_emitted() {
        let mut acc = SseAccumulator::new();
        let chunk = r#"{"choices":[{"delta":{"reasoning":"thinking","content":"hello"},"finish_reason":null}]}"#;
        let events = acc.process(chunk);
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StreamEvent::ThinkingDelta(t) if t == "thinking"));
        assert!(matches!(&events[1], StreamEvent::TextDelta(t) if t == "hello"));
    }

    #[test]
    fn malformed_json_returns_empty() {
        let mut acc = SseAccumulator::new();
        let events = acc.process("not json at all!!!");
        assert!(events.is_empty());
    }
}
