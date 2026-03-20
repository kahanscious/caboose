//! Gemini SSE accumulator — simpler than OpenAI/Anthropic because
//! Gemini sends complete function calls (no incremental buffering).

use tracing::warn;
use uuid::Uuid;

use super::types::StreamChunk;
use crate::provider::StreamEvent;

pub struct SseAccumulator {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl SseAccumulator {
    pub fn new() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    pub fn process(&mut self, data: &str) -> Vec<StreamEvent> {
        let chunk: StreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to parse Gemini SSE chunk: {e}");
                return vec![];
            }
        };

        if let Some(usage) = &chunk.usage_metadata {
            self.input_tokens = usage.prompt_token_count;
            self.output_tokens = usage.candidates_token_count.unwrap_or(0);
        }

        let mut events = Vec::new();

        for candidate in &chunk.candidates {
            if let Some(content) = &candidate.content {
                for part in &content.parts {
                    if let Some(text) = &part.text
                        && !text.is_empty()
                    {
                        events.push(StreamEvent::TextDelta(text.clone()));
                    }
                    if let Some(fc) = &part.function_call {
                        events.push(StreamEvent::ToolCall {
                            id: Uuid::new_v4().to_string(),
                            name: fc.name.clone(),
                            arguments: serde_json::to_string(&fc.args).unwrap_or_default(),
                        });
                    }
                }
            }

            if let Some(reason) = &candidate.finish_reason
                && (reason == "STOP" || reason == "MAX_TOKENS")
            {
                events.push(StreamEvent::Done {
                    input_tokens: Some(self.input_tokens),
                    output_tokens: Some(self.output_tokens),
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
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
        let events = acc.process(r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}"#);
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], StreamEvent::TextDelta(t) if t == "Hello"));
    }

    #[test]
    fn test_function_call() {
        let mut acc = SseAccumulator::new();
        let events = acc.process(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_file","args":{"path":"/src"}}}]}}]}"#
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolCall {
                name, arguments, ..
            } => {
                assert_eq!(name, "read_file");
                assert!(arguments.contains("/src"));
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn test_usage_metadata() {
        let mut acc = SseAccumulator::new();
        acc.process(r#"{"candidates":[{"content":{"parts":[{"text":"Hi"}]}}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":50}}"#);
        assert_eq!(acc.input_tokens, 100);
        assert_eq!(acc.output_tokens, 50);
    }

    #[test]
    fn test_finish_reason_stop() {
        let mut acc = SseAccumulator::new();
        let events = acc.process(
            r#"{"candidates":[{"content":{"parts":[{"text":"done"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5}}"#
        );
        // Should get text delta + done
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta(_)))
        );
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
    }

    #[test]
    fn missing_usage_metadata_defaults_to_zero() {
        let mut acc = SseAccumulator::new();
        let events = acc.process(r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]}}]}"#);
        assert!(matches!(&events[0], StreamEvent::TextDelta(_)));
        assert_eq!(acc.input_tokens, 0);
        assert_eq!(acc.output_tokens, 0);
    }

    #[test]
    fn empty_parts_array_yields_nothing() {
        let mut acc = SseAccumulator::new();
        let events = acc.process(r#"{"candidates":[{"content":{"parts":[]}}]}"#);
        assert!(events.is_empty());
    }

    #[test]
    fn finish_reason_without_content_yields_done() {
        let mut acc = SseAccumulator::new();
        let events = acc.process(
            r#"{"candidates":[{"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":50}}"#,
        );
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
            other => panic!("Expected Done, got {:?}", other),
        }
    }
}
