//! Context window management — LLM-based conversation summarization.

use std::collections::HashSet;

use super::conversation::{Content, ContentBlock, Conversation, Role, truncate_to_boundary};
use crate::provider::{Message as ProviderMessage, StreamEvent};

/// The summarization prompt sent to the provider.
const SUMMARIZATION_INSTRUCTIONS: &str = "\
Below is a mechanically-pruned conversation transcript from a coding session. \
Summarize it into a structured handoff so work can resume without the raw history.

Include specific code snippets, file paths with line numbers, and exact function signatures. \
Prefer concrete detail over general descriptions.

Produce the summary using these sections:

## Intent
What the user wants to accomplish and current objective.

## Technology
Languages, frameworks, libraries, and tools involved.

## Files
Files read or modified with paths and line numbers. Note which were recently active.

## Code
Key function signatures, type definitions, and code snippets that are essential context.

## Errors
Errors encountered, their causes, and resolutions (omit if none).

## Decisions
Technical decisions made and their rationale.

## Progress
What has been completed so far.

## Pending
Work remaining, blocked items, or open questions.

## Next
The immediate next action the agent should take.

Be thorough but concise. Use bullet points. Do not include tool output verbatim.

<transcript>
{transcript}
</transcript>";

/// Token budget for protected recent tool output (chars / 4 ≈ tokens).
const PROTECTED_TOOL_OUTPUT_TOKENS: usize = 40_000;
/// Minimum savings required to trigger pruning (tokens).
const MIN_PRUNING_SAVINGS_TOKENS: usize = 20_000;

/// Pre-compaction pass: prune old tool outputs while protecting recent ones.
///
/// Walks backward through messages, accumulating tool result sizes (chars / 4 as
/// token estimate). The most recent ~40k tokens of tool output are protected.
/// Older tool outputs are replaced with `[tool output pruned — N chars]`.
/// Only applies if total savings >= 20k tokens.
pub fn prune_tool_outputs(conversation: &mut Conversation) -> usize {
    // First pass: walk backward, identify which tool results to protect
    let mut token_budget = PROTECTED_TOOL_OUTPUT_TOKENS;
    let mut protected_indices: HashSet<usize> = HashSet::new();
    let mut total_prunable_chars: usize = 0;

    for (i, msg) in conversation.messages.iter().enumerate().rev() {
        if let Content::Blocks(blocks) = &msg.content {
            for block in blocks {
                if let ContentBlock::ToolResult { content, .. } = block {
                    let tokens = content.len() / 4;
                    if token_budget > 0 {
                        protected_indices.insert(i);
                        token_budget = token_budget.saturating_sub(tokens);
                    } else {
                        total_prunable_chars += content.len();
                    }
                }
            }
        }
    }

    // Only prune if savings are meaningful
    let savings_tokens = total_prunable_chars / 4;
    if savings_tokens < MIN_PRUNING_SAVINGS_TOKENS {
        return 0;
    }

    // Second pass: prune unprotected tool outputs
    let mut changes = 0;
    for (i, msg) in conversation.messages.iter_mut().enumerate() {
        if protected_indices.contains(&i) {
            continue;
        }
        if let Content::Blocks(blocks) = &mut msg.content {
            for block in blocks.iter_mut() {
                if let ContentBlock::ToolResult { content, .. } = block
                    && content.len() > 100
                {
                    // only prune substantial outputs
                    let original_len = content.len();
                    *content = format!("[tool output pruned — {original_len} chars]");
                    changes += 1;
                }
            }
        }
    }

    changes
}

/// Mechanical pruning pass — removes noise from the conversation before
/// LLM summarization. Returns the count of messages removed or modified.
///
/// Three rules:
/// 1. Remove cold-stored stubs — ToolResult whose content starts with `[stored:`
///    plus the matching ToolUse message (same id).
/// 2. Remove wasted turns — assistant messages with no tool calls AND text < 50 chars.
/// 3. Truncate long tool outputs — ToolResult content > 200 chars is truncated
///    to 200 chars with `...[truncated]` appended.
pub fn mechanically_prune(conversation: &mut Conversation) -> usize {
    let mut changes = 0usize;

    // --- Pass 0: Replace Image blocks with text placeholders ---
    for msg in &mut conversation.messages {
        if let Content::Blocks(blocks) = &mut msg.content {
            for block in blocks.iter_mut() {
                if let ContentBlock::Image { source_path, .. } = block {
                    let label = source_path.as_deref().unwrap_or("pasted image");
                    *block = ContentBlock::Text {
                        text: format!("[image: {label}]"),
                    };
                    changes += 1;
                }
            }
        }
    }

    // --- Pass 1: Collect IDs of cold-stored ToolResults for removal ---
    let mut cold_stored_ids: HashSet<String> = HashSet::new();
    for msg in &conversation.messages {
        if let Content::Blocks(blocks) = &msg.content {
            for block in blocks {
                if let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                    && content.starts_with("[stored:")
                {
                    cold_stored_ids.insert(tool_use_id.clone());
                }
            }
        }
    }

    // --- Pass 2: Remove cold-stored stub messages and their matching ToolUse messages ---
    if !cold_stored_ids.is_empty() {
        let before = conversation.messages.len();
        conversation.messages.retain(|msg| {
            match &msg.content {
                // Remove ToolResult messages that are cold-stored stubs
                Content::Blocks(blocks) => {
                    // Check if this message is purely a cold-stored ToolResult
                    let is_cold_result = blocks.iter().any(|b| {
                        matches!(b, ContentBlock::ToolResult { tool_use_id, content, .. }
                            if content.starts_with("[stored:") && cold_stored_ids.contains(tool_use_id))
                    });
                    if is_cold_result {
                        return false;
                    }

                    // Check if this is a ToolUse message whose id is in cold_stored_ids
                    let is_matching_tool_use = blocks.iter().all(|b| {
                        matches!(b, ContentBlock::ToolUse { id, .. } if cold_stored_ids.contains(id))
                    }) && blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. }));
                    if is_matching_tool_use {
                        return false;
                    }

                    true
                }
                Content::Text(_) => true,
            }
        });
        let removed = before - conversation.messages.len();
        changes += removed;
    }

    // --- Pass 3: Remove wasted turns ---
    {
        let before = conversation.messages.len();
        conversation.messages.retain(|msg| {
            if msg.role != Role::Assistant {
                return true;
            }
            match &msg.content {
                Content::Text(text) => {
                    // No tool calls in plain text messages; remove if short
                    if text.len() < 50 {
                        return false;
                    }
                    true
                }
                Content::Blocks(blocks) => {
                    let has_tool_call = blocks
                        .iter()
                        .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
                    if has_tool_call {
                        return true;
                    }
                    // No tool calls — check total text length
                    let total_text: usize = blocks
                        .iter()
                        .map(|b| {
                            if let ContentBlock::Text { text } = b {
                                text.len()
                            } else {
                                0
                            }
                        })
                        .sum();
                    if total_text < 50 {
                        return false;
                    }
                    true
                }
            }
        });
        let removed = before - conversation.messages.len();
        changes += removed;
    }

    // --- Pass 4: Truncate long ToolResult content ---
    const TRUNCATE_LIMIT: usize = 200;
    for msg in &mut conversation.messages {
        if let Content::Blocks(blocks) = &mut msg.content {
            for block in blocks.iter_mut() {
                if let ContentBlock::ToolResult { content, .. } = block
                    && content.len() > TRUNCATE_LIMIT
                {
                    let safe_end = truncate_to_boundary(content, TRUNCATE_LIMIT).len();
                    content.truncate(safe_end);
                    content.push_str("...[truncated]");
                    changes += 1;
                }
            }
        }
    }

    changes
}

/// Build provider messages for a compaction summarization request.
pub fn build_compaction_messages(system_prompt: &str, transcript: &str) -> Vec<ProviderMessage> {
    let user_content = SUMMARIZATION_INSTRUCTIONS.replace("{transcript}", transcript);
    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(ProviderMessage {
            role: "system".to_string(),
            content: serde_json::json!(system_prompt),
        });
    }
    messages.push(ProviderMessage {
        role: "user".to_string(),
        content: serde_json::json!(user_content),
    });
    messages
}

/// Check if auto-compaction should trigger.
/// `threshold` is the fraction of context window at which to compact (default 1.0).
pub fn needs_compaction(input_tokens: u32, context_window: u32, threshold: f64) -> bool {
    context_window > 0 && (input_tokens as f64 / context_window as f64) >= threshold
}

/// Collect a full text response from a stream of StreamEvents.
/// Returns the concatenated text deltas.
pub async fn collect_stream_text(
    mut stream: std::pin::Pin<Box<dyn futures::Stream<Item = anyhow::Result<StreamEvent>> + Send>>,
) -> anyhow::Result<String> {
    use futures::StreamExt;
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta(delta) => text.push_str(&delta),
            StreamEvent::Done { .. } => break,
            StreamEvent::Error(e) => return Err(anyhow::anyhow!("Compaction stream error: {e}")),
            _ => {} // ignore tool calls during compaction
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::super::conversation::Message;
    use super::*;

    #[test]
    fn needs_compaction_over_context_window() {
        assert!(needs_compaction(210_000, 200_000, 1.0));
    }

    #[test]
    fn needs_compaction_under_context_window() {
        assert!(!needs_compaction(180_000, 200_000, 1.0));
    }

    #[test]
    fn needs_compaction_at_context_window() {
        // At exactly 100%, auto-compact triggers
        assert!(needs_compaction(200_000, 200_000, 1.0));
    }

    #[test]
    fn needs_compaction_custom_threshold() {
        // 75% threshold: 160k/200k = 80% → should trigger
        assert!(needs_compaction(160_000, 200_000, 0.75));
        // 75% threshold: 140k/200k = 70% → should not trigger
        assert!(!needs_compaction(140_000, 200_000, 0.75));
        // Exactly at threshold
        assert!(needs_compaction(150_000, 200_000, 0.75));
    }

    #[test]
    fn needs_compaction_zero_context_window() {
        assert!(!needs_compaction(100, 0, 1.0));
    }

    #[test]
    fn build_messages_includes_system_and_transcript() {
        let msgs =
            build_compaction_messages("You are helpful.", "User: Hi\n\nAssistant: Hello\n\n");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
        let user_text = msgs[1].content.as_str().unwrap();
        assert!(user_text.contains("User: Hi"));
        assert!(user_text.contains("## Intent"));
    }

    #[test]
    fn build_messages_no_system_prompt() {
        let msgs = build_compaction_messages("", "transcript");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[tokio::test]
    async fn collect_stream_text_concatenates_deltas() {
        let stream = futures::stream::iter(vec![
            Ok(StreamEvent::TextDelta("Hello ".into())),
            Ok(StreamEvent::TextDelta("world".into())),
            Ok(StreamEvent::Done {
                input_tokens: Some(10),
                output_tokens: Some(5),
                cache_read_tokens: None,
                cache_creation_tokens: None,
            }),
        ]);
        let result = collect_stream_text(Box::pin(stream)).await.unwrap();
        assert_eq!(result, "Hello world");
    }

    #[tokio::test]
    async fn collect_stream_text_handles_error() {
        let stream = futures::stream::iter(vec![
            Ok(StreamEvent::TextDelta("partial".into())),
            Ok(StreamEvent::Error("boom".into())),
        ]);
        let result = collect_stream_text(Box::pin(stream)).await;
        assert!(result.is_err());
    }

    // --- mechanically_prune tests ---

    #[test]
    fn prune_removes_cold_stored_stubs_and_matching_tool_use() {
        let mut conv = Conversation::new("system".into());

        // Assistant message with a ToolUse whose result is cold-stored
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "/foo.rs"}),
            }]),
            tool_call_id: None,
        });
        // Cold-stored ToolResult
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "[stored: t1 | read_file | 1200 bytes]".into(),
                is_error: false,
            }]),
            tool_call_id: Some("t1".into()),
        });
        // A normal user message that should survive
        conv.push(Message {
            role: Role::User,
            content: Content::Text("Please continue working on the feature.".into()),
            tool_call_id: None,
        });

        assert_eq!(conv.messages.len(), 3);
        let pruned = mechanically_prune(&mut conv);

        // Both the ToolUse and ToolResult should be removed
        assert_eq!(conv.messages.len(), 1);
        assert!(pruned >= 2);
        // The surviving message should be the normal user message
        match &conv.messages[0].content {
            Content::Text(t) => assert!(t.contains("continue working")),
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn prune_removes_wasted_assistant_turns() {
        let mut conv = Conversation::new("system".into());

        // User message
        conv.push(Message {
            role: Role::User,
            content: Content::Text("Do something.".into()),
            tool_call_id: None,
        });
        // Short assistant reply with no tool calls (wasted turn)
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Text("OK".into()),
            tool_call_id: None,
        });
        // Another short assistant with blocks but no tool calls
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::Text {
                text: "Sure".into(),
            }]),
            tool_call_id: None,
        });

        assert_eq!(conv.messages.len(), 3);
        let pruned = mechanically_prune(&mut conv);

        // Both short assistant messages should be removed
        assert_eq!(conv.messages.len(), 1);
        assert!(pruned >= 2);
        assert_eq!(conv.messages[0].role, Role::User);
    }

    #[test]
    fn prune_truncates_long_tool_outputs() {
        let mut conv = Conversation::new("system".into());

        let long_output = "x".repeat(500);
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: long_output,
                is_error: false,
            }]),
            tool_call_id: Some("t1".into()),
        });

        let pruned = mechanically_prune(&mut conv);

        assert!(pruned >= 1);
        assert_eq!(conv.messages.len(), 1);
        if let Content::Blocks(blocks) = &conv.messages[0].content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(content.ends_with("...[truncated]"));
                // 200 chars + "...[truncated]" (14 chars) = 214
                assert_eq!(content.len(), 214);
            } else {
                panic!("expected ToolResult block");
            }
        } else {
            panic!("expected blocks content");
        }
    }

    #[test]
    fn prune_preserves_normal_messages() {
        let mut conv = Conversation::new("system".into());

        // Normal user message
        conv.push(Message {
            role: Role::User,
            content: Content::Text("Please implement the feature described in the spec.".into()),
            tool_call_id: None,
        });
        // Substantial assistant message (>= 50 chars)
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Text(
                "I'll implement that feature now. Let me start by reading the relevant files."
                    .into(),
            ),
            tool_call_id: None,
        });
        // Assistant with tool calls (should never be removed regardless of text length)
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![
                ContentBlock::Text {
                    text: "Reading.".into(),
                },
                ContentBlock::ToolUse {
                    id: "t2".into(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": "/bar.rs"}),
                },
            ]),
            tool_call_id: None,
        });
        // Short tool result (under 200, no truncation needed)
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t2".into(),
                content: "fn main() {}".into(),
                is_error: false,
            }]),
            tool_call_id: Some("t2".into()),
        });

        assert_eq!(conv.messages.len(), 4);
        let pruned = mechanically_prune(&mut conv);

        // Nothing should be removed or modified
        assert_eq!(pruned, 0);
        assert_eq!(conv.messages.len(), 4);
    }

    #[test]
    fn prune_strips_images_to_text_placeholders() {
        let mut conv = Conversation::new("system".into());

        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![
                ContentBlock::Text {
                    text: "Check this screenshot".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "base64data".into(),
                    source_path: Some("screenshot.png".into()),
                },
            ]),
            tool_call_id: None,
        });

        let pruned = mechanically_prune(&mut conv);
        assert!(pruned >= 1);
        assert_eq!(conv.messages.len(), 1);

        if let Content::Blocks(blocks) = &conv.messages[0].content {
            assert_eq!(blocks.len(), 2);
            match &blocks[1] {
                ContentBlock::Text { text } => {
                    assert_eq!(text, "[image: screenshot.png]");
                }
                _ => panic!("expected Text block after image stripping"),
            }
        } else {
            panic!("expected blocks content");
        }
    }

    #[test]
    fn prune_strips_images_without_source_path() {
        let mut conv = Conversation::new("system".into());

        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::Image {
                media_type: "image/png".into(),
                data: "base64data".into(),
                source_path: None,
            }]),
            tool_call_id: None,
        });

        let pruned = mechanically_prune(&mut conv);
        assert!(pruned >= 1);

        if let Content::Blocks(blocks) = &conv.messages[0].content {
            match &blocks[0] {
                ContentBlock::Text { text } => {
                    assert_eq!(text, "[image: pasted image]");
                }
                _ => panic!("expected Text block after image stripping"),
            }
        } else {
            panic!("expected blocks content");
        }
    }

    #[test]
    fn prune_tool_outputs_protects_recent_and_trims_old() {
        let mut conv = Conversation::new("system".into());

        // Add 80 tool results with 4000 chars each (= ~1000 tokens each, ~80k tokens total)
        // Budget protects 40k tokens (~40 results), leaving 40 results prunable (~40k tokens > 20k min)
        for i in 0..80 {
            let id = format!("t{i}");
            conv.push(Message {
                role: Role::Assistant,
                content: Content::Blocks(vec![ContentBlock::ToolUse {
                    id: id.clone(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": format!("/file{i}.rs")}),
                }]),
                tool_call_id: None,
            });
            conv.push(Message {
                role: Role::User,
                content: Content::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: "x".repeat(4000),
                    is_error: false,
                }]),
                tool_call_id: Some(id),
            });
        }

        let changes = prune_tool_outputs(&mut conv);

        // Recent ~40k tokens of output should be protected (last ~10 results at 1000 tokens each)
        // Older results should be pruned
        assert!(changes > 0);

        // Last few results should still have original content
        let last_result = &conv.messages[conv.messages.len() - 1];
        if let Content::Blocks(blocks) = &last_result.content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert_eq!(
                    content.len(),
                    4000,
                    "most recent result should be preserved"
                );
            }
        }

        // Earlier results should be pruned
        let early_result = &conv.messages[1]; // first tool result
        if let Content::Blocks(blocks) = &early_result.content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(
                    content.contains("[tool output pruned"),
                    "early result should be pruned"
                );
            }
        }
    }

    #[test]
    fn prune_tool_outputs_noop_when_under_threshold() {
        let mut conv = Conversation::new("system".into());

        // Add 5 small tool results (well under 40k + 20k threshold)
        for i in 0..5 {
            let id = format!("t{i}");
            conv.push(Message {
                role: Role::Assistant,
                content: Content::Blocks(vec![ContentBlock::ToolUse {
                    id: id.clone(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": format!("/file{i}.rs")}),
                }]),
                tool_call_id: None,
            });
            conv.push(Message {
                role: Role::User,
                content: Content::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content: "short output".into(),
                    is_error: false,
                }]),
                tool_call_id: Some(id),
            });
        }

        let changes = prune_tool_outputs(&mut conv);
        assert_eq!(changes, 0);
    }

    #[test]
    fn summarization_prompt_has_nine_sections() {
        let msgs = build_compaction_messages("sys", "transcript");
        let user_text = msgs.last().unwrap().content.as_str().unwrap();
        let sections = [
            "## Intent",
            "## Technology",
            "## Files",
            "## Code",
            "## Errors",
            "## Decisions",
            "## Progress",
            "## Pending",
            "## Next",
        ];
        for section in &sections {
            assert!(user_text.contains(section), "missing section: {section}");
        }
    }
}
