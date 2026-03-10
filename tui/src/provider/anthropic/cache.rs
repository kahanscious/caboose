//! Prompt caching — per-provider cache control marker injection.

use super::types::{ApiContentBlock, ApiRequest, CacheControl};

/// Cache strategy for a provider.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum CacheStrategy {
    /// Anthropic-style: cache_control markers on system + tools + context
    Anthropic,
    /// OpenAI-style: promptCacheKey header (not yet implemented)
    OpenAi,
    /// No caching
    None,
}

/// Inject Anthropic prompt caching markers into a request.
///
/// Marks three cache breakpoints (Anthropic caches everything up to and
/// including a `cache_control` marker):
/// 1. Last system block — caches the system prompt
/// 2. Last tool definition — caches the tool schemas
/// 3. Second-to-last user message — caches conversation context
pub fn inject_anthropic_cache(request: &mut ApiRequest) {
    let marker = CacheControl::ephemeral();

    // 1. Mark last system block
    if let Some(system) = &mut request.system
        && let Some(last) = system.last_mut()
    {
        last.cache_control = Some(marker.clone());
    }

    // 2. Mark last tool definition
    if let Some(tools) = &mut request.tools
        && let Some(last) = tools.last_mut()
    {
        last.cache_control = Some(marker.clone());
    }

    // 3. Mark second-to-last user message's last content block
    let user_indices: Vec<usize> = request
        .messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user")
        .map(|(i, _)| i)
        .collect();

    if user_indices.len() >= 2 {
        let idx = user_indices[user_indices.len() - 2];
        if let Some(block) = request.messages[idx].content.last_mut() {
            match block {
                ApiContentBlock::Text { cache_control, .. } => {
                    *cache_control = Some(marker);
                }
                ApiContentBlock::ToolResult { .. } => {
                    // Tool results don't support cache_control — skip
                }
                ApiContentBlock::ToolUse { .. } => {
                    // Tool uses don't support cache_control — skip
                }
                ApiContentBlock::Image { .. } => {
                    // Images don't support cache_control — skip
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::anthropic::types::*;

    fn make_user_msg(text: &str) -> ApiMessage {
        ApiMessage {
            role: "user".to_string(),
            content: vec![ApiContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
        }
    }

    fn make_assistant_msg(text: &str) -> ApiMessage {
        ApiMessage {
            role: "assistant".to_string(),
            content: vec![ApiContentBlock::Text {
                text: text.to_string(),
                cache_control: None,
            }],
        }
    }

    #[test]
    fn test_caches_system_block() {
        let mut request = ApiRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 8192,
            stream: true,
            system: Some(vec![SystemBlock {
                block_type: "text".to_string(),
                text: "You are a helpful assistant.".to_string(),
                cache_control: None,
            }]),
            messages: vec![make_user_msg("Hello")],
            tools: None,
        };
        inject_anthropic_cache(&mut request);
        assert!(request.system.unwrap()[0].cache_control.is_some());
    }

    #[test]
    fn test_caches_last_tool() {
        let mut request = ApiRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 8192,
            stream: true,
            system: None,
            messages: vec![make_user_msg("Hello")],
            tools: Some(vec![
                ApiToolDef {
                    name: "read_file".to_string(),
                    description: "Read a file".to_string(),
                    input_schema: serde_json::json!({}),
                    cache_control: None,
                },
                ApiToolDef {
                    name: "write_file".to_string(),
                    description: "Write a file".to_string(),
                    input_schema: serde_json::json!({}),
                    cache_control: None,
                },
            ]),
        };
        inject_anthropic_cache(&mut request);
        let tools = request.tools.unwrap();
        assert!(tools[0].cache_control.is_none());
        assert!(tools[1].cache_control.is_some());
    }

    #[test]
    fn test_caches_second_to_last_user_message() {
        let mut request = ApiRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 8192,
            stream: true,
            system: None,
            messages: vec![
                make_user_msg("First question"),
                make_assistant_msg("First answer"),
                make_user_msg("Second question"),
                make_assistant_msg("Second answer"),
                make_user_msg("Third question"),
            ],
            tools: None,
        };
        inject_anthropic_cache(&mut request);

        // Second-to-last user message is index 2 ("Second question")
        match &request.messages[2].content[0] {
            ApiContentBlock::Text { cache_control, .. } => {
                assert!(cache_control.is_some());
            }
            _ => panic!("Expected Text block"),
        }
        // Last user message should NOT be cached
        match &request.messages[4].content[0] {
            ApiContentBlock::Text { cache_control, .. } => {
                assert!(cache_control.is_none());
            }
            _ => panic!("Expected Text block"),
        }
    }

    #[test]
    fn test_no_crash_with_single_user_message() {
        let mut request = ApiRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 8192,
            stream: true,
            system: None,
            messages: vec![make_user_msg("Only message")],
            tools: None,
        };
        // Should not panic — just doesn't mark anything
        inject_anthropic_cache(&mut request);
    }
}
