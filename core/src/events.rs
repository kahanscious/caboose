//! Event bus foundation for the WebSocket server and multi-consumer architecture.
//!
//! Defines [`CoreEvent`] (core → consumers), [`CoreCommand`] (consumers → core),
//! and [`CoreHandle`] (clonable handle for sending commands and subscribing to events).

use tokio::sync::{broadcast, mpsc};

use crate::agent::{AgentEvent, PendingToolCall};
use crate::provider::{ModelInfo, ToolDefinition};
use crate::session::Session;
use crate::session::storage::StoredMessage;
use crate::tools::ToolResult;

// ---------------------------------------------------------------------------
// CoreEvent
// ---------------------------------------------------------------------------

/// Events flowing from core to consumers (TUI, server, mobile).
#[derive(Debug, Clone)]
pub enum CoreEvent {
    // --- Agent conversation (wraps AgentEvent) ---
    /// Partial text from the model.
    TextDelta(String),
    /// Partial thinking/reasoning from the model.
    ThinkingDelta(String),
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
        cache_read_tokens: u32,
        cache_creation_tokens: u32,
    },
    /// Stream error.
    Error(String),
    /// Structured provider error with classification.
    ProviderError {
        category: String,
        provider: String,
        message: String,
        hint: Option<String>,
    },
    /// Compaction finished — conversation has been summarized.
    CompactionComplete,

    // --- Tool lifecycle ---
    /// Tool calls awaiting user approval.
    ToolApprovalRequired {
        tool_calls: Vec<PendingToolCall>,
        current_index: usize,
    },
    /// A tool has finished executing.
    ToolExecuted(ToolResult),

    // --- Session ---
    /// A new session was created.
    SessionCreated(Session),
    /// List of sessions returned.
    SessionList(Vec<Session>),
    /// A session was loaded with its messages.
    SessionLoaded {
        session: Session,
        messages: Vec<StoredMessage>,
    },
    /// A session was deleted.
    SessionDeleted {
        session_id: String,
    },

    // --- Provider ---
    /// The active provider/model was switched.
    ProviderSwitched {
        provider: String,
        model: String,
    },
    /// List of models returned.
    ModelList(Vec<ModelInfo>),

    // --- MCP ---
    /// An MCP server connected successfully.
    McpServerConnected {
        name: String,
    },
    /// An MCP server was disconnected.
    McpServerDisconnected {
        name: String,
    },
    /// Tools discovered from an MCP server.
    McpToolsDiscovered {
        server: String,
        tools: Vec<ToolDefinition>,
    },

    // --- Background agents ---
    /// A background agent was spawned.
    BackgroundAgentStarted {
        id: String,
        prompt_summary: String,
        budget: u64,
        parent_session_id: String,
    },
    /// Progress update from a background agent.
    BackgroundAgentProgress {
        id: String,
        tokens_used: u64,
        budget_remaining: u64,
        turn_count: u32,
    },
    /// A background agent completed successfully.
    BackgroundAgentComplete {
        id: String,
        tokens_used: u64,
        session_id: String,
    },
    /// A background agent failed.
    BackgroundAgentFailed {
        id: String,
        reason: String,
        tokens_used: u64,
    },

    // --- Checkpoints ---
    /// A checkpoint was created.
    CheckpointCreated {
        name: String,
    },
    /// Rewound to a checkpoint.
    CheckpointRewound {
        name: String,
    },

    // --- Roundhouse ---
    /// Roundhouse phase changed.
    RoundhousePhaseChanged {
        phase: String,
    },
    /// Roundhouse plan complete.
    RoundhouseComplete {
        plan: String,
    },

    // --- Status ---
    /// Current status snapshot.
    Status {
        provider: String,
        model: String,
        session_id: String,
        permission_mode: String,
    },
}

// ---------------------------------------------------------------------------
// From<AgentEvent>
// ---------------------------------------------------------------------------

impl From<AgentEvent> for CoreEvent {
    fn from(event: AgentEvent) -> Self {
        match event {
            AgentEvent::TextDelta(s) => CoreEvent::TextDelta(s),
            AgentEvent::ThinkingDelta(s) => CoreEvent::ThinkingDelta(s),
            AgentEvent::ToolCall {
                id,
                name,
                arguments,
            } => CoreEvent::ToolCall {
                id,
                name,
                arguments,
            },
            AgentEvent::TurnComplete {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => CoreEvent::TurnComplete {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            },
            AgentEvent::Error(s) => CoreEvent::Error(s),
            AgentEvent::ProviderError {
                category,
                provider,
                message,
                hint,
            } => CoreEvent::ProviderError {
                category: format!("{:?}", category),
                provider,
                message,
                hint,
            },
            AgentEvent::CompactionComplete => CoreEvent::CompactionComplete,
        }
    }
}

// ---------------------------------------------------------------------------
// CoreCommand
// ---------------------------------------------------------------------------

/// Commands flowing from consumers into core.
#[derive(Debug, Clone)]
pub enum CoreCommand {
    /// Send a text message to the agent.
    SendMessage { text: String },
    /// Send a message with attached image blocks.
    SendMessageWithBlocks {
        text: String,
        image_paths: Vec<std::path::PathBuf>,
    },
    /// Cancel the current agent turn.
    CancelTurn,
    /// Approve the pending tool call.
    ApproveTool,
    /// Deny the pending tool call.
    DenyTool,
    /// Always allow the pending tool for this session.
    AlwaysAllowTool,
    /// Create a new session.
    CreateSession,
    /// List recent sessions.
    ListSessions { limit: usize },
    /// Load a session by ID.
    LoadSession { session_id: String },
    /// Delete a session by ID.
    DeleteSession { session_id: String },
    /// Search sessions by query.
    SearchSessions { query: String, limit: usize },
    /// Switch the active provider and model.
    SwitchProvider { provider: String, model: String },
    /// List available models.
    ListModels,
    /// Set the thinking/reasoning mode.
    SetThinkingMode { mode: String },
    /// Connect an MCP server by name.
    ConnectMcpServer { name: String },
    /// Disconnect an MCP server by name.
    DisconnectMcpServer { name: String },
    /// Spawn a background agent with a prompt and optional token budget.
    SpawnBackgroundAgent { prompt: String, budget: Option<u64> },
    /// Kill a running background agent.
    KillBackgroundAgent { id: String },
    /// Create a named checkpoint.
    CreateCheckpoint { name: String },
    /// Rewind to a named checkpoint.
    RewindToCheckpoint { name: String },
    /// Request current status.
    GetStatus,
}

// ---------------------------------------------------------------------------
// CoreHandle
// ---------------------------------------------------------------------------

/// Clonable handle that consumers use to send commands and subscribe to events.
#[derive(Clone)]
pub struct CoreHandle {
    /// Sender for commands into the core event loop.
    pub commands: mpsc::UnboundedSender<CoreCommand>,
    /// Broadcast sender for events from core to all subscribers.
    event_tx: broadcast::Sender<CoreEvent>,
}

impl CoreHandle {
    /// Create a new handle and the corresponding command receiver.
    ///
    /// The broadcast channel is created with a capacity of 256 events.
    pub fn new() -> (Self, mpsc::UnboundedReceiver<CoreCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(256);
        (
            Self {
                commands: cmd_tx,
                event_tx,
            },
            cmd_rx,
        )
    }

    /// Subscribe to the event stream. Each subscriber gets its own receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.event_tx.subscribe()
    }

    /// Emit an event to all subscribers. Silently drops if no subscribers.
    pub fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Send a command to the core event loop.
    pub fn send(
        &self,
        command: CoreCommand,
    ) -> Result<(), mpsc::error::SendError<CoreCommand>> {
        self.commands.send(command)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_converts_to_core_event() {
        let agent_event = AgentEvent::TextDelta("hello".to_string());
        let core_event: CoreEvent = agent_event.into();
        match core_event {
            CoreEvent::TextDelta(s) => assert_eq!(s, "hello"),
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn agent_event_turn_complete_converts() {
        let agent_event = AgentEvent::TurnComplete {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 10,
            cache_creation_tokens: 5,
        };
        let core_event: CoreEvent = agent_event.into();
        match core_event {
            CoreEvent::TurnComplete {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
            } => {
                assert_eq!(input_tokens, 100);
                assert_eq!(output_tokens, 50);
                assert_eq!(cache_read_tokens, 10);
                assert_eq!(cache_creation_tokens, 5);
            }
            other => panic!("expected TurnComplete, got {:?}", other),
        }
    }

    #[test]
    fn agent_event_provider_error_converts() {
        let agent_event = AgentEvent::ProviderError {
            category: crate::provider::error::ErrorCategory::Auth,
            provider: "anthropic".to_string(),
            message: "invalid key".to_string(),
            hint: Some("check your API key".to_string()),
        };
        let core_event: CoreEvent = agent_event.into();
        match core_event {
            CoreEvent::ProviderError {
                category,
                provider,
                message,
                hint,
            } => {
                assert_eq!(category, "Auth");
                assert_eq!(provider, "anthropic");
                assert_eq!(message, "invalid key");
                assert_eq!(hint, Some("check your API key".to_string()));
            }
            other => panic!("expected ProviderError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn core_handle_emit_and_subscribe() {
        let (handle, _cmd_rx) = CoreHandle::new();
        let mut rx = handle.subscribe();

        handle.emit(CoreEvent::TextDelta("world".to_string()));

        let event = rx.recv().await.expect("should receive event");
        match event {
            CoreEvent::TextDelta(s) => assert_eq!(s, "world"),
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn core_handle_send_command() {
        let (handle, mut cmd_rx) = CoreHandle::new();

        handle
            .send(CoreCommand::SendMessage {
                text: "hi".to_string(),
            })
            .expect("send should succeed");

        let cmd = cmd_rx.recv().await.expect("should receive command");
        match cmd {
            CoreCommand::SendMessage { text } => assert_eq!(text, "hi"),
            other => panic!("expected SendMessage, got {:?}", other),
        }
    }
}
