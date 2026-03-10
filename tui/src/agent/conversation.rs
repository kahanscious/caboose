//! Conversation state — message types and history management.

use serde::{Deserialize, Serialize};

use super::cold_storage::{ColdStore, build_stub};

/// Minimum tool output size worth cold-storing (skip tiny results).
const COLD_STORAGE_MIN_SIZE: usize = 500;

/// Truncate a string to at most `max_bytes` bytes, ensuring the cut is on a
/// UTF-8 character boundary. Returns the longest prefix that fits.
pub(crate) fn truncate_to_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message in the conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Content,
    /// For tool results, the tool call ID this responds to.
    pub tool_call_id: Option<String>,
}

/// Message content — can be plain text or structured blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// A content block within a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "image")]
    Image {
        media_type: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_path: Option<String>,
    },
}

/// Conversation history with token tracking.
pub struct Conversation {
    pub messages: Vec<Message>,
    pub system_prompt: String,
    pub estimated_tokens: u32,
}

impl Conversation {
    pub fn new(system_prompt: String) -> Self {
        Self {
            messages: Vec::new(),
            system_prompt,
            estimated_tokens: 0,
        }
    }

    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
        // TODO: update token estimate
    }

    /// Rotate old tool outputs to cold storage, keeping the most recent
    /// `hot_tail_size` tool results inline. Returns count of rotated outputs.
    pub fn rotate_to_cold(
        &mut self,
        store: &ColdStore,
        hot_tail_size: usize,
    ) -> anyhow::Result<usize> {
        // 1. Collect (message_index, block_index) for every ToolResult block.
        let mut tool_result_positions: Vec<(usize, usize)> = Vec::new();
        for (mi, msg) in self.messages.iter().enumerate() {
            if let Content::Blocks(blocks) = &msg.content {
                for (bi, block) in blocks.iter().enumerate() {
                    if matches!(block, ContentBlock::ToolResult { .. }) {
                        tool_result_positions.push((mi, bi));
                    }
                }
            }
        }

        // 2. The most recent `hot_tail_size` stay; older ones are candidates.
        if tool_result_positions.len() <= hot_tail_size {
            return Ok(0);
        }
        let cold_candidates = &tool_result_positions[..tool_result_positions.len() - hot_tail_size];

        // 3. Rotate candidates that qualify.
        let mut rotated = 0usize;
        for &(mi, bi) in cold_candidates {
            let (tool_use_id, content) = {
                let Content::Blocks(blocks) = &self.messages[mi].content else {
                    continue;
                };
                let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = &blocks[bi]
                else {
                    continue;
                };
                (tool_use_id.clone(), content.clone())
            };

            // Skip already-stored results
            if content.starts_with("[stored:") {
                continue;
            }
            // Skip small results
            if content.len() < COLD_STORAGE_MIN_SIZE {
                continue;
            }

            // Find tool name by walking backwards for matching ToolUse
            let (tool_name, tool_args) = self
                .find_tool_info(&tool_use_id)
                .unwrap_or(("unknown", String::new()));

            // Write to cold store
            store.store(&tool_use_id, &content)?;

            // Replace inline content with stub
            let stub = build_stub(&tool_use_id, tool_name, &tool_args, &content);
            if let Content::Blocks(blocks) = &mut self.messages[mi].content
                && let ContentBlock::ToolResult {
                    content: ref mut c, ..
                } = blocks[bi]
            {
                *c = stub;
            }

            rotated += 1;
        }

        Ok(rotated)
    }

    /// Find the tool name and a summary of key arguments for a given tool_use_id.
    /// Returns `(tool_name, arg_summary)` or None if the ToolUse block isn't found.
    pub(crate) fn find_tool_info(&self, tool_use_id: &str) -> Option<(&str, String)> {
        for msg in self.messages.iter().rev() {
            if let Content::Blocks(blocks) = &msg.content {
                for block in blocks {
                    if let ContentBlock::ToolUse { id, name, input } = block
                        && id == tool_use_id
                    {
                        let args = extract_tool_args(name, input);
                        return Some((name.as_str(), args));
                    }
                }
            }
        }
        None
    }

    /// Maximum chars of tool output to include in transcript.
    const TOOL_OUTPUT_TRIM: usize = 200;

    /// Serialize conversation to a human-readable transcript for summarization.
    pub fn serialize_transcript(&self) -> String {
        let mut out = String::new();
        for msg in &self.messages {
            let role_label = match msg.role {
                Role::System => "System",
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool",
            };
            match &msg.content {
                Content::Text(text) => {
                    out.push_str(&format!("{role_label}: {text}\n\n"));
                }
                Content::Blocks(blocks) => {
                    for block in blocks {
                        match block {
                            ContentBlock::Text { text } => {
                                out.push_str(&format!("{role_label}: {text}\n\n"));
                            }
                            ContentBlock::ToolUse { name, input, .. } => {
                                let args = input.to_string();
                                let args_short = if args.len() > Self::TOOL_OUTPUT_TRIM {
                                    format!(
                                        "{}...",
                                        truncate_to_boundary(&args, Self::TOOL_OUTPUT_TRIM)
                                    )
                                } else {
                                    args
                                };
                                out.push_str(&format!("Tool({name}): {args_short}\n\n"));
                            }
                            ContentBlock::ToolResult {
                                content, is_error, ..
                            } => {
                                let status = if *is_error { "error" } else { "ok" };
                                let trimmed = if content.len() > Self::TOOL_OUTPUT_TRIM {
                                    format!(
                                        "{}...",
                                        truncate_to_boundary(content, Self::TOOL_OUTPUT_TRIM)
                                    )
                                } else {
                                    content.clone()
                                };
                                out.push_str(&format!("Tool(result) [{status}]: {trimmed}\n\n"));
                            }
                            ContentBlock::Image { source_path, .. } => {
                                let label = source_path.as_deref().unwrap_or("pasted image");
                                out.push_str(&format!("{role_label}: [image: {label}]\n\n"));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Replace all messages with a single summary message. Resets token estimate.
    pub fn replace_with_summary(&mut self, summary: String) {
        self.messages.clear();
        self.messages.push(Message {
            role: Role::User,
            content: Content::Text(format!(
                "[Conversation summary — resuming from compacted context]\n\n{summary}"
            )),
            tool_call_id: None,
        });
        self.estimated_tokens = 0;
    }
}

/// Extract a human-readable summary of the key arguments for a tool call.
///
/// For known tools, extracts the most relevant field(s).
/// For unknown tools, shows the first string value from the input JSON.
fn extract_tool_args(tool_name: &str, input: &serde_json::Value) -> String {
    let obj = match input.as_object() {
        Some(o) => o,
        None => return String::new(),
    };

    match tool_name {
        // File-path tools: show the path
        "read_file" | "write_file" | "edit_file" | "list_files" => {
            match obj.get("path").and_then(|v| v.as_str()) {
                Some(p) => format!("path: \"{p}\""),
                None => String::new(),
            }
        }
        // Command execution: show the command (truncated)
        "execute_command" => match obj.get("command").and_then(|v| v.as_str()) {
            Some(cmd) => {
                if cmd.len() > 80 {
                    format!("command: \"{}...\"", truncate_to_boundary(cmd, 80))
                } else {
                    format!("command: \"{cmd}\"")
                }
            }
            None => String::new(),
        },
        // Glob: show the pattern
        "glob" => match obj.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => format!("pattern: \"{p}\""),
            None => String::new(),
        },
        // Grep: show pattern + optional path
        "grep" => {
            let pattern = obj.get("pattern").and_then(|v| v.as_str());
            let path = obj.get("path").and_then(|v| v.as_str());
            match (pattern, path) {
                (Some(pat), Some(p)) => format!("pattern: \"{pat}\", path: \"{p}\""),
                (Some(pat), None) => format!("pattern: \"{pat}\""),
                _ => String::new(),
            }
        }
        // Unknown/MCP tools: show first string value
        _ => {
            for (key, value) in obj {
                if let Some(s) = value.as_str() {
                    let display = if s.len() > 80 {
                        format!("{}...", truncate_to_boundary(s, 80))
                    } else {
                        s.to_string()
                    };
                    return format!("{key}: \"{display}\"");
                }
            }
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::cold_storage::ColdStore;

    #[test]
    fn serialize_transcript_text_messages() {
        let mut conv = Conversation::new("You are helpful.".into());
        conv.push(Message {
            role: Role::User,
            content: Content::Text("Hello".into()),
            tool_call_id: None,
        });
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Text("Hi there!".into()),
            tool_call_id: None,
        });
        let transcript = conv.serialize_transcript();
        assert!(transcript.contains("User: Hello"));
        assert!(transcript.contains("Assistant: Hi there!"));
    }

    #[test]
    fn serialize_transcript_tool_results_truncated() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: "x".repeat(500),
                is_error: false,
            }]),
            tool_call_id: Some("t1".into()),
        });
        let transcript = conv.serialize_transcript();
        // Should be truncated — not the full 500 chars
        assert!(transcript.len() < 500);
        assert!(transcript.contains("Tool(result)"));
        assert!(transcript.contains("..."));
    }

    #[test]
    fn serialize_transcript_tool_use_blocks() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![
                ContentBlock::Text {
                    text: "Let me read that.".into(),
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": "/foo.rs"}),
                },
            ]),
            tool_call_id: None,
        });
        let transcript = conv.serialize_transcript();
        assert!(transcript.contains("Assistant: Let me read that."));
        assert!(transcript.contains("Tool(read_file):"));
    }

    #[test]
    fn replace_with_summary_clears_history() {
        let mut conv = Conversation::new("system".into());
        conv.push(Message {
            role: Role::User,
            content: Content::Text("msg1".into()),
            tool_call_id: None,
        });
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Text("msg2".into()),
            tool_call_id: None,
        });
        assert_eq!(conv.messages.len(), 2);

        conv.replace_with_summary("Summary of conversation.".into());
        assert_eq!(conv.messages.len(), 1);
        assert_eq!(conv.messages[0].role, Role::User);
        match &conv.messages[0].content {
            Content::Text(t) => assert!(t.contains("Summary of conversation.")),
            _ => panic!("expected text content"),
        }
        assert_eq!(conv.estimated_tokens, 0);
    }

    /// Helper: build a conversation with N tool use/result pairs.
    /// Each tool result has content of `content_size` chars.
    fn build_conv_with_tool_results(n: usize, content_size: usize) -> Conversation {
        let mut conv = Conversation::new("system".into());
        for i in 0..n {
            let id = format!("tool-{i}");
            // Assistant message with ToolUse
            conv.push(Message {
                role: Role::Assistant,
                content: Content::Blocks(vec![ContentBlock::ToolUse {
                    id: id.clone(),
                    name: "read_file".into(),
                    input: serde_json::json!({"path": format!("/file{i}.rs")}),
                }]),
                tool_call_id: None,
            });
            // Build content of exactly `content_size` bytes using ASCII chars
            let content = "x".repeat(content_size);
            // User message with ToolResult
            conv.push(Message {
                role: Role::User,
                content: Content::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content,
                    is_error: false,
                }]),
                tool_call_id: Some(id),
            });
        }
        conv
    }

    #[test]
    fn rotate_to_cold_rotates_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-rotate");
        store.base_dir = dir.path().join("cold").join("test-rotate");

        let mut conv = build_conv_with_tool_results(15, 600);
        let rotated = conv.rotate_to_cold(&store, 10).unwrap();

        // The 5 oldest should be rotated
        assert_eq!(rotated, 5);

        // Verify rotated messages contain "[stored:" stubs
        for i in 0..5 {
            let msg_idx = i * 2 + 1; // tool result messages at odd indices
            if let Content::Blocks(blocks) = &conv.messages[msg_idx].content {
                if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                    assert!(
                        content.starts_with("[stored:"),
                        "Expected rotated result {i} to start with '[stored:', got: {}",
                        &content[..content.len().min(40)]
                    );
                }
            }
        }

        // Verify the 10 most recent are unchanged (still large, not stubs)
        for i in 5..15 {
            let msg_idx = i * 2 + 1;
            if let Content::Blocks(blocks) = &conv.messages[msg_idx].content {
                if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                    assert!(
                        !content.starts_with("[stored:"),
                        "Hot tail result {i} should NOT be rotated"
                    );
                    assert!(
                        content.len() >= 500,
                        "Hot tail result {i} should keep original content"
                    );
                }
            }
        }
    }

    #[test]
    fn rotate_to_cold_skips_small_results() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-small");
        store.base_dir = dir.path().join("cold").join("test-small");

        // 15 tool results, each under 500 chars
        let mut conv = build_conv_with_tool_results(15, 100);
        let rotated = conv.rotate_to_cold(&store, 10).unwrap();

        // Nothing should be rotated — all too small
        assert_eq!(rotated, 0);
    }

    #[test]
    fn rotate_to_cold_skips_already_stored() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-already");
        store.base_dir = dir.path().join("cold").join("test-already");

        let mut conv = build_conv_with_tool_results(15, 600);

        // First rotation
        let rotated1 = conv.rotate_to_cold(&store, 10).unwrap();
        assert_eq!(rotated1, 5);

        // Second rotation — same conversation, nothing new to rotate
        let rotated2 = conv.rotate_to_cold(&store, 10).unwrap();
        assert_eq!(rotated2, 0);
    }

    #[test]
    fn rotate_to_cold_noop_when_fewer_than_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-few");
        store.base_dir = dir.path().join("cold").join("test-few");

        let mut conv = build_conv_with_tool_results(5, 600);
        let rotated = conv.rotate_to_cold(&store, 10).unwrap();
        assert_eq!(rotated, 0);
    }

    #[test]
    fn rotate_to_cold_finds_tool_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ColdStore::new("test-name");
        store.base_dir = dir.path().join("cold").join("test-name");

        let mut conv = build_conv_with_tool_results(12, 600);
        conv.rotate_to_cold(&store, 10).unwrap();

        // Check that the stub references the tool name and args
        if let Content::Blocks(blocks) = &conv.messages[1].content {
            if let ContentBlock::ToolResult { content, .. } = &blocks[0] {
                assert!(
                    content.contains("read_file"),
                    "Stub should contain tool name 'read_file'"
                );
                assert!(
                    content.contains("/file0.rs"),
                    "Stub should contain the file path arg"
                );
            }
        }
    }

    #[test]
    fn truncate_to_boundary_ascii() {
        let s = "hello world";
        assert_eq!(truncate_to_boundary(s, 5), "hello");
        assert_eq!(truncate_to_boundary(s, 100), s);
        assert_eq!(truncate_to_boundary(s, 0), "");
    }

    #[test]
    fn truncate_to_boundary_multibyte() {
        // Each emoji is 4 bytes in UTF-8
        let s = "\u{1F600}\u{1F601}\u{1F602}"; // 12 bytes total
        assert_eq!(truncate_to_boundary(s, 4), "\u{1F600}");
        assert_eq!(truncate_to_boundary(s, 5), "\u{1F600}"); // can't split next char
        assert_eq!(truncate_to_boundary(s, 8), "\u{1F600}\u{1F601}");
        assert_eq!(truncate_to_boundary(s, 12), s);
    }

    #[test]
    fn truncate_to_boundary_two_byte_chars() {
        // 'e' with acute accent U+00E9 is 2 bytes in UTF-8
        let s = "caf\u{00E9} latte";
        // "caf" = 3 bytes, '\u{00E9}' = 2 bytes (bytes 3..5)
        assert_eq!(truncate_to_boundary(s, 4), "caf"); // byte 4 is mid-char
        assert_eq!(truncate_to_boundary(s, 5), "caf\u{00E9}");
    }

    #[test]
    fn truncate_to_boundary_cjk() {
        // CJK characters are 3 bytes each
        let s = "\u{4F60}\u{597D}\u{4E16}\u{754C}"; // 12 bytes
        assert_eq!(truncate_to_boundary(s, 3), "\u{4F60}");
        assert_eq!(truncate_to_boundary(s, 4), "\u{4F60}"); // mid-char
        assert_eq!(truncate_to_boundary(s, 6), "\u{4F60}\u{597D}");
    }

    #[test]
    fn serialize_transcript_multibyte_tool_output_no_panic() {
        // This would panic before the fix if the trim boundary fell inside a multi-byte char.
        let mut conv = Conversation::new(String::new());
        // Build a string of 4-byte emoji chars that exceeds TOOL_OUTPUT_TRIM (200 bytes)
        let emoji_content: String = std::iter::repeat('\u{1F600}').take(100).collect(); // 400 bytes
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::ToolResult {
                tool_use_id: "t1".into(),
                content: emoji_content,
                is_error: false,
            }]),
            tool_call_id: Some("t1".into()),
        });
        // Should not panic
        let transcript = conv.serialize_transcript();
        assert!(transcript.contains("Tool(result)"));
        assert!(transcript.contains("..."));
    }

    #[test]
    fn serialize_transcript_multibyte_tool_use_args_no_panic() {
        let mut conv = Conversation::new(String::new());
        // Build a JSON input with multi-byte chars exceeding 200 bytes
        let big_value: String = std::iter::repeat('\u{4F60}').take(100).collect(); // 300 bytes
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "write_file".into(),
                input: serde_json::json!({"content": big_value}),
            }]),
            tool_call_id: None,
        });
        // Should not panic
        let transcript = conv.serialize_transcript();
        assert!(transcript.contains("Tool(write_file)"));
    }

    #[test]
    fn find_tool_info_read_file() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "src/main.rs"}),
            }]),
            tool_call_id: None,
        });
        let (name, args) = conv.find_tool_info("t1").unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(args, r#"path: "src/main.rs""#);
    }

    #[test]
    fn find_tool_info_execute_command() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "execute_command".into(),
                input: serde_json::json!({"command": "cargo test --lib"}),
            }]),
            tool_call_id: None,
        });
        let (name, args) = conv.find_tool_info("t1").unwrap();
        assert_eq!(name, "execute_command");
        assert_eq!(args, r#"command: "cargo test --lib""#);
    }

    #[test]
    fn find_tool_info_grep_with_path() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "grep".into(),
                input: serde_json::json!({"pattern": "TODO", "path": "src/"}),
            }]),
            tool_call_id: None,
        });
        let (name, args) = conv.find_tool_info("t1").unwrap();
        assert_eq!(name, "grep");
        assert_eq!(args, r#"pattern: "TODO", path: "src/""#);
    }

    #[test]
    fn find_tool_info_unknown_tool_generic_fallback() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "mcp_search".into(),
                input: serde_json::json!({"query": "find all users", "limit": 10}),
            }]),
            tool_call_id: None,
        });
        let (name, args) = conv.find_tool_info("t1").unwrap();
        assert_eq!(name, "mcp_search");
        assert_eq!(args, r#"query: "find all users""#);
    }

    #[test]
    fn find_tool_info_empty_input() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "some_tool".into(),
                input: serde_json::json!({}),
            }]),
            tool_call_id: None,
        });
        let (name, args) = conv.find_tool_info("t1").unwrap();
        assert_eq!(name, "some_tool");
        assert_eq!(args, "");
    }

    #[test]
    fn find_tool_info_not_found() {
        let conv = Conversation::new(String::new());
        assert!(conv.find_tool_info("nonexistent").is_none());
    }

    #[test]
    fn find_tool_info_long_command_truncated() {
        let mut conv = Conversation::new(String::new());
        let long_cmd = "x".repeat(120);
        conv.push(Message {
            role: Role::Assistant,
            content: Content::Blocks(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "execute_command".into(),
                input: serde_json::json!({"command": long_cmd}),
            }]),
            tool_call_id: None,
        });
        let (_name, args) = conv.find_tool_info("t1").unwrap();
        // Should be truncated to 80 chars + "..."
        assert!(args.len() < 120);
        assert!(args.contains("..."));
    }

    #[test]
    fn serialize_transcript_image_blocks() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::Image {
                media_type: "image/png".into(),
                data: "base64data".into(),
                source_path: Some("screenshot.png".into()),
            }]),
            tool_call_id: None,
        });
        let transcript = conv.serialize_transcript();
        assert!(transcript.contains("User: [image: screenshot.png]"));
        // Should NOT contain the base64 data
        assert!(!transcript.contains("base64data"));
    }

    #[test]
    fn serialize_transcript_image_no_path() {
        let mut conv = Conversation::new(String::new());
        conv.push(Message {
            role: Role::User,
            content: Content::Blocks(vec![ContentBlock::Image {
                media_type: "image/png".into(),
                data: "base64data".into(),
                source_path: None,
            }]),
            tool_call_id: None,
        });
        let transcript = conv.serialize_transcript();
        assert!(transcript.contains("User: [image: pasted image]"));
    }

    #[test]
    fn image_content_blocks_persist_in_session() {
        let blocks = vec![
            ContentBlock::Text {
                text: "Here's a screenshot".into(),
            },
            ContentBlock::Image {
                media_type: "image/png".into(),
                data: "iVBORw0KGgo=".into(),
                source_path: Some("screenshot.png".into()),
            },
        ];
        let content = Content::Blocks(blocks);
        let json = serde_json::to_string(&content).unwrap();
        let deserialized: Content = serde_json::from_str(&json).unwrap();

        if let Content::Blocks(blocks) = deserialized {
            assert_eq!(blocks.len(), 2);
            match &blocks[1] {
                ContentBlock::Image {
                    media_type,
                    data,
                    source_path,
                } => {
                    assert_eq!(media_type, "image/png");
                    assert_eq!(data, "iVBORw0KGgo=");
                    assert_eq!(source_path.as_deref(), Some("screenshot.png"));
                }
                _ => panic!("expected Image block"),
            }
        } else {
            panic!("expected Blocks content");
        }
    }

    #[test]
    fn image_content_block_serde_roundtrip() {
        let block = ContentBlock::Image {
            media_type: "image/jpeg".into(),
            data: "abc123".into(),
            source_path: Some("photo.jpg".into()),
        };
        let json = serde_json::to_string(&block).unwrap();
        let deserialized: ContentBlock = serde_json::from_str(&json).unwrap();
        match deserialized {
            ContentBlock::Image {
                media_type,
                data,
                source_path,
            } => {
                assert_eq!(media_type, "image/jpeg");
                assert_eq!(data, "abc123");
                assert_eq!(source_path, Some("photo.jpg".into()));
            }
            _ => panic!("expected Image variant"),
        }

        // Also test without source_path
        let block_no_path = ContentBlock::Image {
            media_type: "image/png".into(),
            data: "xyz".into(),
            source_path: None,
        };
        let json2 = serde_json::to_string(&block_no_path).unwrap();
        assert!(!json2.contains("source_path"));
        let deserialized2: ContentBlock = serde_json::from_str(&json2).unwrap();
        match deserialized2 {
            ContentBlock::Image { source_path, .. } => {
                assert_eq!(source_path, None);
            }
            _ => panic!("expected Image variant"),
        }
    }
}
