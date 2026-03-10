//! Agent layer — multi-turn conversation loop with tool execution.

pub mod cold_storage;
pub mod compaction;
pub mod conversation;
pub mod permission;
pub mod tools;

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::provider::{self, Provider, ToolDefinition};
use cold_storage::ColdStore;
use conversation::{Content, ContentBlock, Conversation, Message, Role};
use permission::{PermissionMode, ToolDecision, check_permission};

/// Events flowing from the stream task to the app event loop.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Partial text from the model.
    TextDelta(String),
    /// Model requested a tool call.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Stream finished — model's turn is done.
    TurnComplete {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// Stream error.
    Error(String),
    /// Structured provider error with classification.
    ProviderError {
        category: crate::provider::error::ErrorCategory,
        provider: String,
        message: String,
        hint: Option<String>,
    },
    /// Compaction finished — conversation has been summarized.
    CompactionComplete,
}

/// A tool call waiting for permission check or execution.
#[derive(Debug, Clone)]
pub struct PendingToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Agent loop state machine.
#[derive(Debug)]
pub enum AgentState {
    /// Waiting for user input.
    Idle,
    /// Provider stream is active — text/tool events arriving.
    Streaming,
    /// Tool calls collected, waiting for user approval.
    PendingApproval {
        tool_calls: Vec<PendingToolCall>,
        current_index: usize,
    },
    /// Tools are executing.
    ExecutingTools,
    /// Agent is running a compaction summarization.
    Compacting,
}

/// The agent loop manages multi-turn conversations with tool execution.
pub struct AgentLoop {
    pub conversation: Conversation,
    pub state: AgentState,
    pub turn_count: u32,
    pub permission_mode: PermissionMode,
    pub streaming_text: String,
    pub pending_tool_calls: Vec<PendingToolCall>,
    pub session_allows: HashSet<String>,
    pub allow_list: Vec<String>,
    pub deny_list: Vec<String>,
    /// Additional env var names to strip when executing shell commands.
    pub additional_secrets: Vec<String>,
    pub last_input_tokens: u32,
    pub last_output_tokens: u32,
    /// Tokens-per-second for the last completed stream.
    pub last_tokens_per_sec: Option<f64>,
    /// When the first TextDelta arrived (for tok/s calculation, excludes TTFT).
    first_token_at: Option<Instant>,
    /// Full context window for the active model. Used for sidebar display
    /// and compaction triggers.
    pub context_window: u32,
    /// Fraction of context window at which auto-compaction triggers (default 1.0).
    pub compaction_threshold: f64,
    /// Whether the auto-handoff prompt has been shown this session.
    pub handoff_prompted: bool,
    /// Cold storage for large tool outputs.
    pub(crate) cold_store: Option<ColdStore>,
    /// Hot tail size for cold storage rotation (default: 10).
    pub hot_tail_size: usize,
    /// When true, compaction was triggered mid-flow and should resume streaming after.
    resume_after_compaction: bool,
    /// Stashed tool defs for resuming after compaction.
    pub(crate) stashed_tool_defs: Vec<ToolDefinition>,
    /// Recently read file paths (most recent first, max 10).
    pub recent_files: VecDeque<PathBuf>,
    event_rx: Option<mpsc::UnboundedReceiver<AgentEvent>>,
}

const MAX_TURNS: u32 = 1000;

impl AgentLoop {
    pub fn new(system_prompt: String, permission_mode: PermissionMode) -> Self {
        Self {
            conversation: Conversation::new(system_prompt),
            state: AgentState::Idle,
            turn_count: 0,
            permission_mode,
            streaming_text: String::new(),
            pending_tool_calls: Vec::new(),
            session_allows: HashSet::new(),
            allow_list: Vec::new(),
            deny_list: Vec::new(),
            additional_secrets: Vec::new(),
            last_input_tokens: 0,
            last_output_tokens: 0,
            last_tokens_per_sec: None,
            first_token_at: None,
            context_window: 200_000,
            compaction_threshold: 0.85,
            handoff_prompted: false,
            cold_store: None,
            hot_tail_size: 10,
            resume_after_compaction: false,
            stashed_tool_defs: Vec::new(),
            recent_files: VecDeque::new(),
            event_rx: None,
        }
    }

    /// Initialize cold storage for this session.
    pub fn init_cold_store(&mut self, session_id: &str) {
        self.cold_store = Some(ColdStore::new(session_id));
    }

    /// Send a user message and begin an agent turn.
    pub fn send_message(
        &mut self,
        content: String,
        provider: &dyn Provider,
        tool_defs: &[ToolDefinition],
    ) {
        // Add user message to conversation
        self.conversation.push(Message {
            role: Role::User,
            content: Content::Text(content),
            tool_call_id: None,
        });

        // Check if compaction is needed before streaming
        // NOTE: compaction_model override deferred to post-launch.
        // Currently always uses the active provider for compaction.
        if compaction::needs_compaction(
            self.last_input_tokens,
            self.context_window,
            self.compaction_threshold,
        ) {
            self.resume_after_compaction = true;
            self.stashed_tool_defs = tool_defs.to_vec();
            self.compact(provider, None);
        } else {
            self.start_stream(provider, tool_defs);
        }
    }

    /// Send a user message with mixed content (text + images).
    pub fn send_message_with_blocks(
        &mut self,
        blocks: Vec<ContentBlock>,
        provider: &dyn Provider,
        tool_defs: &[ToolDefinition],
    ) {
        self.conversation.push(Message {
            role: Role::User,
            content: Content::Blocks(blocks),
            tool_call_id: None,
        });

        if compaction::needs_compaction(
            self.last_input_tokens,
            self.context_window,
            self.compaction_threshold,
        ) {
            self.resume_after_compaction = true;
            self.stashed_tool_defs = tool_defs.to_vec();
            self.compact(provider, None);
        } else {
            self.start_stream(provider, tool_defs);
        }
    }

    /// Start a new provider stream task.
    pub(crate) fn start_stream(&mut self, provider: &dyn Provider, tool_defs: &[ToolDefinition]) {
        self.streaming_text.clear();
        self.pending_tool_calls.clear();
        self.state = AgentState::Streaming;
        self.first_token_at = None;

        let messages = self.to_provider_messages();
        let tools = tool_defs.to_vec();

        let (tx, rx) = mpsc::unbounded_channel();
        self.event_rx = Some(rx);

        // Build the stream from provider — the stream is 'static so we can
        // move it into the spawned task.
        use futures::StreamExt;
        let stream = provider.stream(&messages, &tools);

        tokio::spawn(async move {
            let mut stream = stream;
            while let Some(event_result) = stream.next().await {
                let agent_event = match event_result {
                    Ok(provider::StreamEvent::TextDelta(text)) => AgentEvent::TextDelta(text),
                    Ok(provider::StreamEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    }) => AgentEvent::ToolCall {
                        id,
                        name,
                        arguments,
                    },
                    Ok(provider::StreamEvent::ThinkingDelta(_)) => {
                        // Extended thinking — ignored for now
                        continue;
                    }
                    Ok(provider::StreamEvent::Done {
                        input_tokens,
                        output_tokens,
                        ..
                    }) => AgentEvent::TurnComplete {
                        input_tokens: input_tokens.unwrap_or(0),
                        output_tokens: output_tokens.unwrap_or(0),
                    },
                    Ok(provider::StreamEvent::Error(e)) => AgentEvent::Error(e),
                    Ok(provider::StreamEvent::ProviderError {
                        category,
                        provider,
                        message,
                        hint,
                    }) => AgentEvent::ProviderError {
                        category,
                        provider,
                        message,
                        hint,
                    },
                    Err(e) => AgentEvent::Error(e.to_string()),
                };
                if tx.send(agent_event).is_err() {
                    break; // receiver dropped, agent cancelled
                }
            }
        });
    }

    /// Poll for events from the stream task. Called every tick from the app loop.
    /// Returns any events that the UI should react to.
    pub fn poll(&mut self) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // Take the receiver out to avoid borrow conflicts when processing events
        let Some(mut rx) = self.event_rx.take() else {
            return events;
        };

        // Drain all available events without blocking
        let mut got_terminal_event = false;
        while let Ok(event) = rx.try_recv() {
            match &event {
                AgentEvent::TextDelta(text) => {
                    if self.first_token_at.is_none() {
                        self.first_token_at = Some(Instant::now());
                    }
                    self.streaming_text.push_str(text);
                }
                AgentEvent::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    let args: serde_json::Value = match serde_json::from_str(arguments) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!(
                                tool = %name,
                                error = %e,
                                raw_args = %arguments,
                                "Failed to parse tool call arguments as JSON"
                            );
                            serde_json::Value::Null
                        }
                    };
                    self.pending_tool_calls.push(PendingToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: args,
                    });
                }
                AgentEvent::TurnComplete {
                    input_tokens,
                    output_tokens,
                } => {
                    self.last_input_tokens = *input_tokens;
                    self.last_output_tokens = *output_tokens;
                    // Calculate tokens-per-second from first token to completion (excludes TTFT)
                    if let Some(first) = self.first_token_at.take() {
                        let elapsed = first.elapsed().as_secs_f64();
                        if elapsed > 0.0 && *output_tokens > 0 {
                            self.last_tokens_per_sec = Some(*output_tokens as f64 / elapsed);
                        }
                    }
                    got_terminal_event = true;
                }
                AgentEvent::Error(_) | AgentEvent::ProviderError { .. } => {
                    self.state = AgentState::Idle;
                    got_terminal_event = true;
                }
                AgentEvent::CompactionComplete => {
                    self.finalize_compaction();
                    self.state = AgentState::Idle;
                    // resume_after_compaction is checked by App
                    got_terminal_event = true;
                }
            }
            events.push(event);
        }

        if got_terminal_event {
            // finalize_turn handles TurnComplete state transitions
            if events
                .iter()
                .any(|e| matches!(e, AgentEvent::TurnComplete { .. }))
            {
                self.finalize_turn();
            }
            // Don't put the receiver back — stream is done
        } else {
            // Stream still active, put the receiver back
            self.event_rx = Some(rx);
        }

        events
    }

    /// Called when TurnComplete arrives. Commits the assistant message and
    /// decides what to do with pending tool calls.
    fn finalize_turn(&mut self) {
        self.event_rx = None;
        self.turn_count += 1;

        // Build assistant message content blocks
        let mut blocks = Vec::new();
        if !self.streaming_text.is_empty() {
            blocks.push(ContentBlock::Text {
                text: std::mem::take(&mut self.streaming_text),
            });
        }
        for tc in &self.pending_tool_calls {
            blocks.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.arguments.clone(),
            });
        }

        if !blocks.is_empty() {
            self.conversation.push(Message {
                role: Role::Assistant,
                content: Content::Blocks(blocks),
                tool_call_id: None,
            });
        }

        if self.pending_tool_calls.is_empty() || self.turn_count >= MAX_TURNS {
            // No tool calls or turn limit reached — done
            self.state = AgentState::Idle;
        } else {
            // Check permissions for each tool call
            self.check_tool_permissions();
        }
    }

    /// Check permissions for pending tool calls, transitioning to
    /// PendingApproval or ExecutingTools as appropriate.
    fn check_tool_permissions(&mut self) {
        let mut needs_approval = false;

        for tc in &self.pending_tool_calls {
            let decision = check_permission(
                &self.permission_mode,
                &tc.name,
                &tc.arguments,
                &self.allow_list,
                &self.deny_list,
                &self.session_allows,
                None, // CLI tool overrides resolved later in execute_pending_tools
            );
            if matches!(decision, ToolDecision::RequireApproval) {
                needs_approval = true;
                break;
            }
        }

        if needs_approval {
            self.state = AgentState::PendingApproval {
                tool_calls: self.pending_tool_calls.clone(),
                current_index: 0,
            };
        } else {
            self.state = AgentState::ExecutingTools;
        }
    }

    /// Execute all pending tool calls (after permission granted).
    /// Returns tool results to add to the conversation.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_pending_tools(
        &mut self,
        mcp_manager: &mut crate::mcp::McpManager,
        services: Option<&crate::config::schema::ServicesConfig>,
        cli_tools: Option<&std::collections::HashMap<String, crate::config::schema::CliToolConfig>>,
        deny_commands: &[String],
        hooks: Option<&crate::config::schema::HooksConfig>,
        exec_tools: Option<
            &std::collections::HashMap<String, crate::config::schema::ExecutableToolConfig>,
        >,
    ) -> Vec<tools::ToolResult> {
        let mut results = Vec::new();

        let tool_calls = std::mem::take(&mut self.pending_tool_calls);
        for tc in &tool_calls {
            // Fire PreToolUse hooks
            if let Some(hooks_config) = hooks {
                let context = serde_json::json!({
                    "event": "PreToolUse",
                    "tool_name": tc.name,
                    "tool_input": tc.arguments,
                });
                let hook_results = crate::hooks::fire_hooks_for_tool(
                    &hooks_config.pre_tool_use,
                    context,
                    &tc.name,
                )
                .await;

                // Check for deny action from hooks
                let mut denied = false;
                for r in &hook_results {
                    if let Some(crate::hooks::HookAction::Deny(reason)) = &r.action {
                        results.push(tools::ToolResult {
                            tool_use_id: tc.id.clone(),
                            output: format!("Blocked by hook: {reason}"),
                            is_error: true,
                            tool_name: Some(tc.name.clone()),
                            file_path: None,
                            files_modified: vec![],
                            lines_added: 0,
                            lines_removed: 0,
                        });
                        denied = true;
                        break;
                    }
                }
                if denied {
                    continue;
                }
            }

            // Look up per-tool permission override for CLI tools
            let tool_permission = if tc.name.starts_with("cli_") {
                cli_tools
                    .and_then(|r| r.get(&tc.name[4..]))
                    .and_then(|c| c.permission.as_deref())
            } else {
                None
            };

            let mut decision = check_permission(
                &self.permission_mode,
                &tc.name,
                &tc.arguments,
                &self.allow_list,
                &self.deny_list,
                &self.session_allows,
                tool_permission,
            );

            // Fire PermissionRequest hooks (only if tool would normally need approval)
            if matches!(decision, ToolDecision::RequireApproval)
                && let Some(hooks_config) = hooks
                && !hooks_config.permission_request.is_empty()
            {
                let context = serde_json::json!({
                    "event": "PermissionRequest",
                    "tool_name": tc.name,
                    "tool_input": tc.arguments,
                    "permission_mode": format!("{:?}", self.permission_mode),
                });
                let hook_results = crate::hooks::fire_hooks_for_tool(
                    &hooks_config.permission_request,
                    context,
                    &tc.name,
                )
                .await;
                for r in &hook_results {
                    if let Some(crate::hooks::HookAction::Allow) = &r.action {
                        decision = ToolDecision::AutoExecute;
                        break;
                    }
                    if let Some(crate::hooks::HookAction::Deny(reason)) = &r.action {
                        decision = ToolDecision::Blocked(format!(
                            "Blocked by PermissionRequest hook: {reason}"
                        ));
                        break;
                    }
                }
            }

            let result = match decision {
                ToolDecision::Blocked(reason) => tools::ToolResult {
                    tool_use_id: tc.id.clone(),
                    output: format!("Tool blocked: {reason}"),
                    is_error: true,
                    tool_name: Some(tc.name.clone()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                },
                _ => {
                    match tools::execute_tool(
                        &tc.name,
                        &tc.arguments,
                        &self.additional_secrets,
                        Some(mcp_manager),
                        None,
                        services,
                        cli_tools,
                        deny_commands,
                        exec_tools,
                    )
                    .await
                    {
                        Ok(mut result) => {
                            result.tool_use_id = tc.id.clone();
                            result
                        }
                        Err(e) => tools::ToolResult {
                            tool_use_id: tc.id.clone(),
                            output: format!("Tool error: {e}"),
                            is_error: true,
                            tool_name: Some(tc.name.clone()),
                            file_path: None,
                            files_modified: vec![],
                            lines_added: 0,
                            lines_removed: 0,
                        },
                    }
                }
            };

            // Fire PostToolUse or PostToolUseFailure hooks
            if let Some(hooks_config) = hooks {
                if result.is_error {
                    let context = serde_json::json!({
                        "event": "PostToolUseFailure",
                        "tool_name": tc.name,
                        "tool_input": tc.arguments,
                        "error": result.output,
                    });
                    let _ = crate::hooks::fire_hooks_for_tool(
                        &hooks_config.post_tool_use_failure,
                        context,
                        &tc.name,
                    )
                    .await;
                } else {
                    let context = serde_json::json!({
                        "event": "PostToolUse",
                        "tool_name": tc.name,
                        "tool_input": tc.arguments,
                        "tool_output": result.output,
                    });
                    let _ = crate::hooks::fire_hooks_for_tool(
                        &hooks_config.post_tool_use,
                        context,
                        &tc.name,
                    )
                    .await;
                }
            }

            results.push(result);
        }

        // Track recently read files for post-compaction re-reading
        for (tc, result) in tool_calls.iter().zip(results.iter()) {
            if tc.name == "read_file"
                && !result.is_error
                && let Some(path_str) = tc.arguments.get("path").and_then(|v| v.as_str())
            {
                let path = PathBuf::from(path_str);
                self.recent_files.retain(|p| p != &path);
                self.recent_files.push_front(path);
                if self.recent_files.len() > 10 {
                    self.recent_files.pop_back();
                }
            }
        }

        // Add tool results to conversation
        for result in &results {
            self.conversation.push(Message {
                role: Role::User,
                content: Content::Blocks(vec![ContentBlock::ToolResult {
                    tool_use_id: result.tool_use_id.clone(),
                    content: result.output.clone(),
                    is_error: result.is_error,
                }]),
                tool_call_id: Some(result.tool_use_id.clone()),
            });
        }

        // Rotate old tool outputs to cold storage, keeping recent ones inline
        if let Some(ref store) = self.cold_store {
            let _rotated = self
                .conversation
                .rotate_to_cold(store, self.hot_tail_size)
                .unwrap_or_else(|e| {
                    tracing::warn!("Cold storage rotation failed: {e}");
                    0
                });
        }

        results
    }

    /// Continue the agent loop after tool execution — start a new stream.
    pub fn continue_after_tools(&mut self, provider: &dyn Provider, tool_defs: &[ToolDefinition]) {
        // NOTE: compaction_model override deferred to post-launch.
        if compaction::needs_compaction(
            self.last_input_tokens,
            self.context_window,
            self.compaction_threshold,
        ) {
            self.resume_after_compaction = true;
            self.stashed_tool_defs = tool_defs.to_vec();
            self.compact(provider, None);
        } else {
            self.start_stream(provider, tool_defs);
        }
    }

    /// Approve the current tool call in PendingApproval state.
    /// Returns true if all tools are now approved and execution should begin.
    pub fn approve_current(&mut self) -> bool {
        if let AgentState::PendingApproval {
            ref tool_calls,
            ref mut current_index,
        } = self.state
        {
            *current_index += 1;
            if *current_index >= tool_calls.len() {
                self.state = AgentState::ExecutingTools;
                return true;
            }
            // Check if remaining tools all auto-approve
            let remaining_all_auto = tool_calls[*current_index..].iter().all(|tc| {
                let d = check_permission(
                    &self.permission_mode,
                    &tc.name,
                    &tc.arguments,
                    &self.allow_list,
                    &self.deny_list,
                    &self.session_allows,
                    None,
                );
                matches!(d, ToolDecision::AutoExecute)
            });
            if remaining_all_auto {
                self.state = AgentState::ExecutingTools;
                return true;
            }
        }
        false
    }

    /// Deny the current tool call in PendingApproval state.
    /// Sends an error result for this tool and moves to the next.
    pub fn deny_current(&mut self) {
        if let AgentState::PendingApproval {
            ref tool_calls,
            ref mut current_index,
        } = self.state
        {
            if let Some(tc) = tool_calls.get(*current_index) {
                // Add denial as tool result
                self.conversation.push(Message {
                    role: Role::User,
                    content: Content::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: tc.id.clone(),
                        content: "Tool execution denied by user.".to_string(),
                        is_error: true,
                    }]),
                    tool_call_id: Some(tc.id.clone()),
                });
                // Remove from pending
                let id = tc.id.clone();
                self.pending_tool_calls.retain(|t| t.id != id);
            }
            *current_index += 1;
            if *current_index >= tool_calls.len() || self.pending_tool_calls.is_empty() {
                self.state = AgentState::Idle;
            }
        }
    }

    /// Always-allow the current tool name for this session.
    pub fn always_allow_current(&mut self) {
        if let AgentState::PendingApproval {
            ref tool_calls,
            current_index,
        } = self.state
            && let Some(tc) = tool_calls.get(current_index)
        {
            self.session_allows.insert(tc.name.clone());
        }
        self.approve_current();
    }

    /// Start a compaction summarization stream.
    /// Transitions to Compacting state. The app loop should call poll()
    /// as normal — CompactionComplete will be emitted when done.
    ///
    /// Two-pass compaction:
    /// 1. Mechanical pruning — remove cold-stored stubs, wasted turns, truncate outputs
    /// 2. LLM summarization — structured prompt for a handoff summary
    ///
    /// NOTE: compaction_model override is deferred to post-launch. Currently always
    /// uses the active (caller-provided) provider. When wired, accept an optional
    /// compaction provider and use it instead.
    pub fn compact(&mut self, provider: &dyn Provider, must_keep: Option<&str>) {
        self.state = AgentState::Compacting;
        self.streaming_text.clear();

        // Pass 0: prune old tool outputs (protects recent 40k tokens)
        let tool_pruned = compaction::prune_tool_outputs(&mut self.conversation);
        if tool_pruned > 0 {
            tracing::info!("Compaction pass 0: pruned {tool_pruned} old tool outputs");
        }

        // Pass 1: mechanical pruning
        let pruned = compaction::mechanically_prune(&mut self.conversation);
        if pruned > 0 {
            tracing::info!("Compaction pass 1: mechanically pruned {pruned} items");
        }

        // Pass 2: LLM summarization
        let transcript = self.conversation.serialize_transcript();
        let mut messages =
            compaction::build_compaction_messages(&self.conversation.system_prompt, &transcript);

        // Inject must_keep context from PreCompact hooks into the compaction prompt
        if let Some(context) = must_keep
            && let Some(last) = messages.last_mut()
            && let Some(text) = last.content.as_str()
        {
            last.content = serde_json::json!(format!(
                "{text}\n\nIMPORTANT — Preserve this context in the summary:\n{context}"
            ));
        }

        let (tx, rx) = mpsc::unbounded_channel();
        self.event_rx = Some(rx);

        let stream = provider.stream(&messages, &[]); // no tools for compaction

        tokio::spawn(async move {
            match compaction::collect_stream_text(stream).await {
                Ok(summary) => {
                    // Send the summary as a special text delta followed by CompactionComplete
                    let _ = tx.send(AgentEvent::TextDelta(summary));
                    let _ = tx.send(AgentEvent::CompactionComplete);
                }
                Err(e) => {
                    let _ = tx.send(AgentEvent::Error(format!("Compaction failed: {e}")));
                }
            }
        });
    }

    /// Finalize compaction — replace conversation with summary, then re-read recent files.
    fn finalize_compaction(&mut self) {
        let summary = std::mem::take(&mut self.streaming_text);
        if !summary.is_empty() {
            self.conversation.replace_with_summary(summary);
        }
        self.last_input_tokens = 0;
        self.last_output_tokens = 0;

        // Post-compaction: re-read recent files to restore working context
        let budget_chars = (self.context_window as usize) * 25 / 100; // 25% of context window in chars
        let mut chars_used = 0;
        let files_to_read: Vec<PathBuf> = self.recent_files.iter().take(5).cloned().collect();

        for path in files_to_read {
            if chars_used >= budget_chars {
                break;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                let content_len = content.len();
                if chars_used + content_len > budget_chars {
                    continue; // skip files that would bust the budget
                }
                chars_used += content_len;

                // Inject as a synthetic tool result message
                let synthetic_id = format!("reread-{}", path.display());
                self.conversation.push(Message {
                    role: Role::Assistant,
                    content: Content::Blocks(vec![ContentBlock::ToolUse {
                        id: synthetic_id.clone(),
                        name: "read_file".into(),
                        input: serde_json::json!({"path": path.display().to_string()}),
                    }]),
                    tool_call_id: None,
                });
                self.conversation.push(Message {
                    role: Role::User,
                    content: Content::Blocks(vec![ContentBlock::ToolResult {
                        tool_use_id: synthetic_id,
                        content,
                        is_error: false,
                    }]),
                    tool_call_id: None,
                });
            }
        }
    }

    /// Cancel the current agent turn.
    pub fn cancel(&mut self) {
        self.event_rx = None; // drops the receiver, stream task will stop
        self.streaming_text.clear();
        self.pending_tool_calls.clear();
        self.state = AgentState::Idle;
    }

    /// Get the current tool call requiring approval (if any).
    pub fn current_approval_prompt(&self) -> Option<(&str, &serde_json::Value)> {
        if let AgentState::PendingApproval {
            ref tool_calls,
            current_index,
        } = self.state
        {
            tool_calls
                .get(current_index)
                .map(|tc| (tc.name.as_str(), &tc.arguments))
        } else {
            None
        }
    }

    /// Convert conversation messages to provider message format.
    fn to_provider_messages(&self) -> Vec<provider::Message> {
        let mut out = Vec::new();

        // Prepend the system prompt as a system-role message
        if !self.conversation.system_prompt.is_empty() {
            out.push(provider::Message {
                role: "system".to_string(),
                content: serde_json::json!(self.conversation.system_prompt),
            });
        }

        for msg in &self.conversation.messages {
            let role = match msg.role {
                Role::User | Role::Tool => "user".to_string(),
                Role::Assistant => "assistant".to_string(),
                Role::System => "system".to_string(),
            };

            let content = match &msg.content {
                Content::Text(text) => serde_json::json!(text),
                Content::Blocks(blocks) => {
                    let json_blocks: Vec<serde_json::Value> = blocks
                        .iter()
                        .map(|b| match b {
                            ContentBlock::Text { text } => {
                                serde_json::json!({"type": "text", "text": text})
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input,
                                })
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            } => {
                                serde_json::json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_use_id,
                                    "content": content,
                                    "is_error": is_error,
                                })
                            }
                            ContentBlock::Image {
                                media_type, data, ..
                            } => {
                                serde_json::json!({
                                    "type": "image",
                                    "media_type": media_type,
                                    "data": data,
                                })
                            }
                        })
                        .collect();
                    serde_json::Value::Array(json_blocks)
                }
            };

            out.push(provider::Message { role, content });
        }

        out
    }
}
