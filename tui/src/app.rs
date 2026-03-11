use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::agent::conversation::ContentBlock;
use crate::agent::permission::PermissionMode;
use crate::agent::{AgentLoop, AgentState};
use crate::config::Config;
use crate::config::auth::AuthStore;
use crate::provider::{Provider, ProviderRegistry};
use crate::session::SessionManager;
use crate::tools::ToolRegistry;
use crate::tui::Terminal;
use crate::tui::dialog::{DialogKind, DialogStack, Screen};
use crate::tui::key_input::KeyInputState;

/// Mouse text selection range (screen coordinates).
pub struct TextSelection {
    pub anchor_row: u16,
    pub anchor_col: u16,
    pub end_row: u16,
    pub end_col: u16,
}

/// UI-visible state, separated so it can be borrowed independently from Terminal.
pub struct State {
    pub config: Config,
    pub dialog_stack: DialogStack,
    pub input: crate::tui::input_buffer::InputBuffer,
    pub should_quit: bool,
    /// Set on first ctrl+c; second ctrl+c within 2s actually quits.
    pub quit_first_press: Option<Instant>,
    pub providers: ProviderRegistry,
    pub tools: ToolRegistry,
    pub mcp_manager: crate::mcp::McpManager,
    pub lsp_manager: Option<crate::lsp::LspManager>,
    pub sessions: SessionManager,
    pub agent: AgentLoop,
    pub chat_messages: Vec<ChatMessage>,
    pub scroll_offset: u16,
    pub user_scrolled_up: bool,
    pub total_chat_lines: Cell<u16>,
    pub chat_area_height: Cell<u16>,
    pub active_provider_name: String,
    pub active_model_name: String,
    pub auth_store: AuthStore,
    pub session_title: Option<String>,
    pub session_title_source: Option<String>,
    pub modified_files: std::collections::HashMap<String, FileStats>,
    /// Original file content before the session first touched each file.
    /// `None` value means the file did not exist before.
    pub file_baselines: std::collections::HashMap<String, Option<String>>,
    /// Tool invocation counts for handoff summary.
    pub tool_counts: std::collections::HashMap<String, u32>,
    pub commands: crate::tui::command::CommandRegistry,
    pub sidebar_visible: bool,
    /// Index of the focused tool message in chat_messages (for expand/collapse navigation).
    pub focused_tool: Option<usize>,
    /// Inline slash autocomplete state — active when input starts with `/`.
    pub slash_auto: Option<crate::tui::slash_auto::SlashAutoState>,
    /// @file autocomplete state — active when input contains `@` prefix.
    pub file_auto: Option<crate::tui::file_auto::FileAutoState>,
    /// All loaded skills (built-in + user).
    pub skills: Vec<crate::skills::Skill>,
    /// Current active session ID (for persistence).
    pub current_session_id: Option<String>,
    /// Memory store for cross-session persistence.
    pub memory: crate::memory::MemoryStore,
    /// Input history for Up/Down browsing across sessions.
    pub history: crate::tui::input_history::InputHistory,
    /// Messages expanded past truncation threshold.
    pub expanded_messages: std::collections::HashSet<usize>,
    pub pricing: crate::provider::pricing::PricingRegistry,
    pub tool_renderers: crate::tui::tools::ToolRendererRegistry,
    /// Queue of user messages to send after current agent turn completes.
    /// Max 3 messages. Input is always open; Enter queues when agent is busy.
    pub message_queue: std::collections::VecDeque<String>,
    /// Queue of tool calls waiting to be executed (one per event loop tick).
    pub tool_exec_queue: std::collections::VecDeque<crate::agent::PendingToolCall>,
    /// Saved args for tool calls (captured before pending_tool_calls is consumed).
    pub tool_exec_args: std::collections::HashMap<String, serde_json::Value>,
    /// Accumulated tool results during iterative execution.
    pub tool_exec_results: Vec<crate::agent::tools::ToolResult>,
    /// Index in chat_messages where Running placeholders start.
    pub tool_exec_running_start: usize,
    /// Receiver for a background-spawned tool execution result.
    /// When `Some`, a tool is running on a tokio task; poll with `try_recv()`.
    pub tool_exec_pending_rx:
        Option<tokio::sync::oneshot::Receiver<crate::agent::tools::ToolResult>>,
    /// Post-tool hooks pipeline (e.g., auto-inject LSP diagnostics).
    pub post_tool_hooks: crate::hooks::PostToolHooks,
    pub mode: crate::agent::permission::Mode,
    /// Whether the active model supports tool calling.
    pub model_supports_tools: bool,
    /// Whether the active model supports vision (image input).
    pub model_supports_vision: bool,
    /// Index into the tips array shown on the home screen (randomized at startup).
    pub home_tip_index: usize,
    /// Frame counter for animations — incremented every render loop iteration.
    pub tick: u64,
    /// Caboose animation position — only advances when agent is active.
    pub caboose_pos: usize,
    /// /init generation: receiver for background streaming events.
    pub init_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::init::handler::InitEvent>>,
    /// /init generation: accumulated streamed text.
    pub init_text: String,
    /// /init generation: whether an existing CABOOSE.md was found.
    pub init_had_existing: bool,
    /// /init generation: line count of previous CABOOSE.md.
    pub init_old_lines: Option<usize>,
    /// /init generation: directory to write CABOOSE.md to.
    pub init_write_root: std::path::PathBuf,
    /// Screen y → message index for clickable truncation indicators.
    pub truncation_click_zones: RefCell<Vec<(u16, usize)>>,
    /// Active mouse text selection in the chat area.
    pub text_selection: Option<TextSelection>,
    /// The Rect of the chat area, set each frame for mouse hit-testing.
    pub chat_area: Cell<Option<ratatui::prelude::Rect>>,
    /// Plain-text content of rendered chat lines (rebuilt each frame for text extraction).
    pub rendered_chat_text: RefCell<Vec<String>>,
    /// Active skill creation session (set by `/create-skill`).
    pub skill_creation: Option<crate::skills::creation::SkillCreationState>,
    /// Pending handoff summary awaiting user confirmation (y/n).
    pub pending_handoff: Option<String>,
    /// Embedded terminal panel (lazy — spawned on first Ctrl+=).
    pub terminal_panel: Option<crate::terminal::panel::TerminalPanel>,
    /// Whether the terminal panel has input focus (clicks route to PTY).
    pub terminal_focused: bool,
    /// Terminal panel screen area (for mouse hit testing).
    pub terminal_area: Cell<Option<ratatui::prelude::Rect>>,
    /// Last resized terminal dimensions (cols, rows) to avoid redundant resize calls.
    pub terminal_last_size: Option<(u16, u16)>,
    /// Active ask-user session — set when an ask_user tool call is being answered.
    pub ask_user_session: Option<crate::tui::ask_user::AskUserSession>,
    /// Receiver for background MCP connection results.
    pub mcp_connect_rx: tokio::sync::mpsc::UnboundedReceiver<(
        String,
        Result<crate::mcp::McpConnectResult, String>,
    )>,
    /// Sender cloned into background tasks.
    pub mcp_connect_tx:
        tokio::sync::mpsc::UnboundedSender<(String, Result<crate::mcp::McpConnectResult, String>)>,
    /// Accumulated session cost in USD (reset each app run).
    pub session_cost: f64,
    /// Whether the budget pause dialog is currently showing.
    pub budget_paused: bool,
    /// Checkpoint manager for file rewind.
    pub checkpoints: crate::checkpoint::CheckpointManager,
    /// Pending image attachments for the next message.
    pub attachments: Vec<crate::attachment::Attachment>,
    /// Latest version available for update (set by background check on startup).
    pub update_available: Option<String>,
    /// Receiver for background update check result.
    pub update_check_rx: Option<tokio::sync::oneshot::Receiver<String>>,
    /// Active Roundhouse (multi-LLM planning) session.
    pub roundhouse_session: Option<crate::roundhouse::RoundhouseSession>,
    /// When true, the model picker adds to roundhouse secondaries instead of switching.
    pub roundhouse_model_add: bool,
    /// Receiver for roundhouse planner status updates (parallel planning engine).
    pub roundhouse_update_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::roundhouse::PlannerUpdate>>,
    /// Receiver for roundhouse synthesis streaming deltas.
    pub roundhouse_synthesis_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    /// In-session circuit manager.
    #[allow(dead_code)]
    pub circuit_manager: crate::circuits::runner::CircuitManager,
    /// Local LLM servers discovered at startup (background probe).
    pub discovered_locals: Vec<crate::provider::local::LocalServer>,
    /// Receiver for background local server discovery result.
    pub local_discovery_rx:
        Option<tokio::sync::oneshot::Receiver<Vec<crate::provider::local::LocalServer>>>,
    /// Detected SCM provider for the current working directory.
    pub scm_provider: crate::scm::detection::ScmProvider,
    /// Active SCM watchers (each backed by a circuit).
    pub active_watchers: Vec<crate::scm::watcher::Watcher>,
}

/// Status of a tool execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// Awaiting user approval (shown with diff preview before execution).
    Pending,
    Running,
    Success,
    Failed,
}

/// Status of a task in the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

/// A single task in the outline.
#[derive(Debug, Clone)]
pub struct Task {
    pub content: String,
    pub active_form: String,
    pub status: TaskStatus,
}

/// Structured task outline displayed inline in the chat.
#[derive(Debug, Clone)]
pub struct TaskOutline {
    pub tasks: Vec<Task>,
}

impl TaskOutline {
    /// Parse from `todo_write` tool input JSON.
    pub fn from_tool_input(input: &serde_json::Value) -> Result<Self, String> {
        let todos = input
            .get("todos")
            .and_then(|v| v.as_array())
            .ok_or("Missing 'todos' array")?;

        if todos.is_empty() {
            return Err("Task list cannot be empty".to_string());
        }

        let tasks = todos
            .iter()
            .map(|t| {
                let content = t
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let active_form = t
                    .get("active_form")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&content)
                    .to_string();
                let status = match t.get("status").and_then(|v| v.as_str()) {
                    Some("in_progress") => TaskStatus::InProgress,
                    Some("completed") => TaskStatus::Completed,
                    Some("cancelled") => TaskStatus::Cancelled,
                    _ => TaskStatus::Pending,
                };
                Task {
                    content,
                    active_form,
                    status,
                }
            })
            .collect();

        Ok(Self { tasks })
    }

    /// Serialize to JSON for session persistence.
    pub fn to_json(&self) -> serde_json::Value {
        let todos: Vec<serde_json::Value> = self
            .tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "content": t.content,
                    "active_form": t.active_form,
                    "status": match t.status {
                        TaskStatus::Pending => "pending",
                        TaskStatus::InProgress => "in_progress",
                        TaskStatus::Completed => "completed",
                        TaskStatus::Cancelled => "cancelled",
                    }
                })
            })
            .collect();
        serde_json::json!({"todos": todos})
    }
}

/// Structured data for a tool message.
#[derive(Debug, Clone)]
pub struct ToolMessage {
    pub name: String,
    pub args: serde_json::Value,
    pub output: Option<String>,
    pub status: ToolStatus,
    pub expanded: bool,
    pub file_path: Option<String>,
}

/// A message in the chat display.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    User {
        content: String,
        images: Vec<(String, usize)>,
    },
    Assistant {
        content: String,
    },
    Tool(ToolMessage),
    System {
        content: String,
    },
    Error {
        content: String,
    },
    /// Structured provider error with category-specific rendering.
    ProviderError {
        category: crate::provider::error::ErrorCategory,
        provider: String,
        message: String,
        hint: Option<String>,
    },
    TaskOutline(TaskOutline),
    Skill {
        name: String,
        description: String,
    },
    /// A user message queued while the agent was busy. Rendered dimmed.
    Queued {
        content: String,
    },
    /// An interactive ask-user question block.
    AskUser {
        header: String,
        question: String,
        options: Vec<(String, String)>,
        /// Selected answer, if answered. None while waiting.
        answer: Option<String>,
        multi_select: bool,
    },
}

/// Tracks file modifications during the session.
#[derive(Debug, Clone, Default)]
pub struct FileStats {
    pub additions: usize,
    pub deletions: usize,
    pub reads: usize,
}

impl State {
    /// Update slash autocomplete state based on current input.
    /// Called after every keystroke that modifies `self.input`.
    pub fn update_slash_auto(&mut self) {
        // Don't reset when in a picker mode — picker manages its own lifecycle
        if let Some(auto) = &self.slash_auto
            && auto.is_picker()
        {
            return;
        }

        let input_text = self.input.content();
        let prefix = crate::tui::slash_auto::slash_prefix(&input_text);
        match prefix {
            Some(p) => {
                let count = crate::tui::slash_auto::total_filtered(p, &self.commands, &self.skills);
                if count == 0 {
                    self.slash_auto = None;
                } else if let Some(auto) = self.slash_auto.as_mut() {
                    // Clamp selection to valid range
                    if auto.selected >= count {
                        auto.selected = count.saturating_sub(1);
                    }
                } else {
                    self.slash_auto = Some(crate::tui::slash_auto::SlashAutoState::new());
                }
            }
            None => {
                self.slash_auto = None;
            }
        }
    }

    /// Update @file autocomplete state based on current input.
    /// Called after every keystroke that modifies `self.input`.
    pub fn update_file_auto(&mut self) {
        let input_text = self.input.content();
        match crate::tui::file_auto::extract_at_prefix(&input_text) {
            Some(partial) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                let matches = crate::tui::file_auto::scan_files(&cwd, partial, 10);
                if matches.is_empty() {
                    self.file_auto = None;
                } else {
                    self.file_auto = Some(crate::tui::file_auto::FileAutoState::new(
                        partial.to_string(),
                        matches,
                    ));
                }
            }
            None => {
                self.file_auto = None;
            }
        }
    }
}

/// Top-level application state machine.
pub struct App {
    pub state: State,
    pub terminal: Terminal,
    provider: Option<Box<dyn Provider>>,
}

impl App {
    pub async fn new(
        mut config: Config,
        model: Option<String>,
        provider_name: Option<String>,
        session_id: Option<String>,
        mode: String,
    ) -> Result<Self> {
        let terminal = Terminal::new()?;
        let providers = ProviderRegistry::new(&config);

        let prefs = crate::config::prefs::TuiPrefs::load();

        // Apply saved theme variant
        crate::tui::theme::set_active_variant(prefs.theme);

        // Resolve provider: CLI flag > saved last-used > default
        let effective_provider = provider_name.as_deref().or(prefs.last_provider.as_deref());
        let effective_model = model.as_deref().or(prefs.last_model.as_deref());

        // Try to resolve provider, but don't fail — the TUI should boot regardless
        let provider = providers
            .get_provider(effective_provider, effective_model)
            .ok();

        // Discover schemas for executable tools that lack description/args
        if let Some(ref mut tools_cfg) = config.tools
            && let Some(ref exec_tools) = tools_cfg.executable
        {
            let discovered = crate::tools::executable::discover_all(exec_tools).await;
            tools_cfg.executable = Some(discovered);
        }

        let cwd = std::env::current_dir().unwrap_or_default();
        let scm_provider = crate::scm::detection::detect_provider(&cwd);

        let cli_tools_ref = config.tools.as_ref().and_then(|t| t.registry.as_ref());
        let exec_tools_ref = config.tools.as_ref().and_then(|t| t.executable.as_ref());
        let tools = ToolRegistry::new(cli_tools_ref, exec_tools_ref, &scm_provider);
        let mcp_config = config.mcp.clone().unwrap_or_default();
        let mcp_manager = crate::mcp::McpManager::from_config(&mcp_config);
        let (mcp_connect_tx, mcp_connect_rx) = tokio::sync::mpsc::unbounded_channel();
        let lsp_manager = crate::lsp::LspManager::new(
            std::env::current_dir().unwrap_or_default(),
            config.lsp.clone(),
        );
        let sessions = SessionManager::new(&config)?;
        let auth_store = AuthStore::default_path()
            .map(AuthStore::new)
            .unwrap_or_else(|| AuthStore::new("auth.json".into()));
        let permission_mode = if mode != "default" {
            // CLI explicitly set a mode — use it
            PermissionMode::from_str_loose(&mode)
        } else if let Some(ref config_mode) = config.permission_mode {
            // Config has a mode — use it
            PermissionMode::from_str_loose(config_mode)
        } else {
            PermissionMode::Default
        };

        let mode = crate::agent::permission::Mode::from_permission_mode(&permission_mode);

        // Initialize memory system
        let memory_config = config.memory.clone().unwrap_or_default();
        let global_memory_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("caboose")
            .join("memory");
        let project_memory_dir = std::path::PathBuf::from(".caboose").join("memory");
        let memory_store = crate::memory::MemoryStore::new(
            global_memory_dir,
            project_memory_dir,
            memory_config.enabled,
        );
        if let Err(e) = memory_store.init() {
            tracing::warn!("Failed to initialize memory: {e}");
        }

        // Reindex FTS5 from memory files
        if let Err(e) = memory_store.reindex(sessions.storage().conn()) {
            tracing::warn!("Failed to index memories: {e}");
        }

        // Prune stale cold storage (24h TTL)
        let _ = crate::agent::cold_storage::ColdStore::cleanup_stale(
            std::time::Duration::from_secs(24 * 3600),
        );

        // Load skills (needed before system prompt for awareness block)
        let skills_disabled = config
            .skills
            .as_ref()
            .map(|s| s.disabled.clone())
            .unwrap_or_default();
        let skills_awareness = config.skills.as_ref().map(|s| s.awareness).unwrap_or(true);
        let skills =
            crate::skills::loader::load_all_skills(std::path::Path::new("."), &skills_disabled);

        // Build system prompt with memory context
        let memory_ctx = memory_store.load_context();
        let base_prompt = config
            .system_prompt
            .clone()
            .unwrap_or_else(|| {
                "You are a helpful AI coding assistant. You have access to tools for reading, \
                 writing, and searching files, running shell commands, and fetching URLs. \
                 Use them to help the user with their coding tasks.\n\n\
                 Use `glob` and `grep` to locate relevant files before reading them — don't guess paths. \
                 Use `read_file` with `offset`/`limit` for targeted reads. Read a small window first \
                 (50–100 lines) to orient, then target specific sections. \
                 Batch independent tool calls in a single response — multiple reads, greps, or globs \
                 can run in one turn. \
                 Don't re-read files already in context unless they've been modified. \
                 When `read_file` output is truncated, use `offset`/`limit` to read the specific section \
                 you need rather than increasing the limit.\n\n\
                 You have `todo_write` and `todo_read` tools for task management. \
                 Use `todo_write` for multi-step tasks (3+ steps) to show progress. \
                 Use `todo_read` to check current task state before updating. \
                 Each `todo_write` call replaces the entire task list. Keep task names short. \
                 Mark tasks completed immediately after finishing each one. \
                 Mark tasks cancelled if they are no longer needed. \
                 Statuses: pending, in_progress, completed, cancelled."
                    .to_string()
            });

        // Load CABOOSE.md project instructions
        let caboose_md = std::fs::read_to_string("CABOOSE.md").ok();
        let base_prompt =
            crate::init::handler::inject_caboose_md(base_prompt, caboose_md.as_deref());

        let system_prompt = if memory_ctx.project.is_some() || memory_ctx.global.is_some() {
            let mut prompt = base_prompt;
            prompt.push_str("\n\n## Memory\n\n");
            prompt.push_str(
                "You have persistent memory files. Project memories: `.caboose/memory/MEMORY.md`. \
                 Global memories: `~/.config/caboose/memory/MEMORY.md`.\n\n\
                 To save something across sessions, edit these files using your file tools. \
                 Keep MEMORY.md concise (~200 lines max). Create topic files for detailed notes.\n\n\
                 ### What to remember\n\
                 - Project structure, build commands, test setup\n\
                 - User preferences (tools, style, workflow)\n\
                 - Architectural decisions and rationale\n\
                 - Solutions to recurring problems\n\n\
                 ### What NOT to remember\n\
                 - Session-specific context (current task, in-progress work)\n\
                 - Unverified assumptions\n\
                 - Anything already in project docs\n\n"
            );
            if let Some(ref project) = memory_ctx.project {
                prompt.push_str("<project-memories>\n");
                prompt.push_str(project);
                prompt.push_str("\n</project-memories>\n\n");
            }
            if let Some(ref global) = memory_ctx.global {
                prompt.push_str("<global-memories>\n");
                prompt.push_str(global);
                prompt.push_str("\n</global-memories>\n\n");
            }
            prompt
        } else {
            base_prompt
        };

        // Inject skill awareness block into system prompt if enabled
        let system_prompt = if skills_awareness && !skills.is_empty() {
            let mut prompt = system_prompt;
            prompt.push_str(&crate::skills::awareness::build_awareness_block(&skills));
            prompt
        } else {
            system_prompt
        };

        let mut agent = AgentLoop::new(system_prompt, permission_mode);

        // Wire tools config (allow/deny commands, additional secrets) into agent
        if let Some(ref tools_cfg) = config.tools {
            if let Some(ref allow) = tools_cfg.allow_commands {
                agent.allow_list = allow.clone();
            }
            if let Some(ref deny) = tools_cfg.deny_commands {
                agent.deny_list = deny.clone();
            }
            if let Some(ref secrets) = tools_cfg.additional_secret_names {
                agent.additional_secrets = secrets.clone();
            }
        }

        // Wire behavior config into agent
        if let Some(ref behavior) = config.behavior {
            if let Some(size) = behavior.hot_tail_size {
                agent.hot_tail_size = size as usize;
            }
            if let Some(threshold) = behavior.compaction_threshold {
                agent.compaction_threshold = threshold.clamp(0.1, 1.0);
            }
        }

        // Set context window for compaction and sidebar display
        if let Some(ref p) = provider {
            let model_id = p.model();

            // If the static table doesn't know this model, fetch from provider API
            if crate::provider::models_dev::context_window(model_id).is_none()
                && let Ok(model_list) = p.list_models().await
            {
                let cw_entries: Vec<(String, Option<u32>)> = model_list
                    .iter()
                    .map(|m| (m.id.clone(), m.context_window))
                    .collect();
                crate::provider::models_dev::cache_from_model_list(&cw_entries);
            }

            agent.context_window = crate::provider::models_dev::context_window_or_default(model_id);
        }

        let active_provider_name = provider
            .as_ref()
            .map(|p| p.name().to_string())
            .unwrap_or_else(|| "none".to_string());
        let active_model_name = provider
            .as_ref()
            .map(|p| p.model().to_string())
            .unwrap_or_else(|| "no key configured".to_string());

        let mut app = Self {
            state: State {
                config,
                dialog_stack: DialogStack::new(Screen::Home),
                input: crate::tui::input_buffer::InputBuffer::new(),
                should_quit: false,
                quit_first_press: None,
                providers,
                tools,
                mcp_manager,
                lsp_manager: Some(lsp_manager),
                sessions,
                agent,
                chat_messages: Vec::new(),
                scroll_offset: 0,
                user_scrolled_up: false,
                total_chat_lines: Cell::new(0),
                chat_area_height: Cell::new(0),
                active_provider_name,
                active_model_name,
                auth_store,
                session_title: None,
                session_title_source: None,
                modified_files: std::collections::HashMap::new(),
                file_baselines: std::collections::HashMap::new(),
                tool_counts: std::collections::HashMap::new(),
                commands: crate::tui::command::build_default_registry(),
                sidebar_visible: prefs.sidebar_visible,
                focused_tool: None,
                slash_auto: None,
                file_auto: None,
                skills,
                current_session_id: None,
                memory: memory_store,
                history: crate::tui::input_history::InputHistory::load(),
                expanded_messages: std::collections::HashSet::new(),
                pricing: crate::provider::pricing::PricingRegistry::new(),
                tool_renderers: crate::tui::tools::ToolRendererRegistry::new(),
                message_queue: std::collections::VecDeque::new(),
                tool_exec_queue: std::collections::VecDeque::new(),
                tool_exec_args: std::collections::HashMap::new(),
                tool_exec_results: Vec::new(),
                tool_exec_running_start: 0,
                tool_exec_pending_rx: None,
                post_tool_hooks: crate::hooks::PostToolHooks::new(),
                mode,
                model_supports_tools: true,
                model_supports_vision: true, // default true for Anthropic models
                home_tip_index: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as usize % crate::tui::home::TIPS.len())
                    .unwrap_or(0),
                tick: 0,
                caboose_pos: 1,
                init_rx: None,
                init_text: String::new(),
                init_had_existing: false,
                init_old_lines: None,
                init_write_root: std::path::PathBuf::new(),
                truncation_click_zones: RefCell::new(Vec::new()),
                text_selection: None,
                chat_area: Cell::new(None),
                rendered_chat_text: RefCell::new(Vec::new()),
                skill_creation: None,
                pending_handoff: None,
                terminal_panel: None,
                terminal_focused: false,
                terminal_area: Cell::new(None),
                terminal_last_size: None,
                ask_user_session: None,
                mcp_connect_rx,
                mcp_connect_tx,
                session_cost: 0.0,
                budget_paused: false,
                checkpoints: crate::checkpoint::CheckpointManager::new(),
                attachments: Vec::new(),
                update_available: None,
                update_check_rx: None,
                roundhouse_session: None,
                roundhouse_model_add: false,
                roundhouse_update_rx: None,
                roundhouse_synthesis_rx: None,
                circuit_manager: crate::circuits::runner::CircuitManager::new(5),
                discovered_locals: vec![],
                local_discovery_rx: None,
                scm_provider,
                active_watchers: Vec::new(),
            },
            terminal,
            provider,
        };

        // Fetch OpenRouter pricing at startup so sidebar shows costs immediately
        if app.state.active_provider_name == "openrouter"
            && let Some(api_key) = app.state.config.keys.get("openrouter")
        {
            let or_provider = crate::provider::openrouter::OpenRouterProvider::new(
                api_key.to_string(),
                app.state.active_model_name.clone(),
            );
            if let Ok((_models, pricing_entries)) = or_provider.list_models_with_pricing().await {
                for (model_id, model_pricing) in pricing_entries {
                    app.state.pricing.insert(model_id, model_pricing);
                }
            }
        }

        // If --session was provided, restore that session
        if let Some(ref sid) = session_id {
            app.restore_session(sid);
        }

        Ok(app)
    }

    /// Restore a session from the database, loading messages into the chat.
    fn restore_session(&mut self, session_id: &str) {
        let session = match self.state.sessions.get(session_id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Session {session_id} not found"),
                });
                return;
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to load session: {e}"),
                });
                return;
            }
        };

        self.state.current_session_id = Some(session.id.clone());
        self.state.agent.init_cold_store(&session.id);
        self.state.session_title = session.title.clone();
        self.state.agent.session_allows.clear();
        self.state.agent.handoff_prompted = false;

        // Load messages from storage
        let messages = match self.state.sessions.load_messages(session_id) {
            Ok(m) => m,
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to load messages: {e}"),
                });
                return;
            }
        };

        // Restore chat messages for display
        for msg in &messages {
            let chat_msg = match msg.role.as_str() {
                "user" => ChatMessage::User {
                    content: msg.content.clone(),
                    images: vec![],
                },
                "assistant" => ChatMessage::Assistant {
                    content: msg.content.clone(),
                },
                "system" => ChatMessage::System {
                    content: msg.content.clone(),
                },
                "provider_error" => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                        ChatMessage::ProviderError {
                            category: serde_json::from_value(
                                json.get("category").cloned().unwrap_or_default(),
                            )
                            .unwrap_or(crate::provider::error::ErrorCategory::Unknown),
                            provider: json
                                .get("provider")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            message: json
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            hint: json
                                .get("hint")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                        }
                    } else {
                        ChatMessage::Error {
                            content: msg.content.clone(),
                        }
                    }
                }
                "error" => ChatMessage::Error {
                    content: msg.content.clone(),
                },
                "task_outline" => {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                        if let Ok(outline) = TaskOutline::from_tool_input(&json) {
                            ChatMessage::TaskOutline(outline)
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };
            self.state.chat_messages.push(chat_msg);
        }

        // If there were messages, go directly to chat screen
        if !self.state.chat_messages.is_empty() {
            self.state.dialog_stack.base = Screen::Chat;
        }
    }

    /// Ensure a session exists (create if needed) and persist a chat message.
    fn persist_message(&mut self, role: &str, content: &str) {
        // Create session on first message
        if self.state.current_session_id.is_none() {
            let model = if self.state.active_model_name == "no key configured" {
                None
            } else {
                Some(self.state.active_model_name.as_str())
            };
            let provider = if self.state.active_provider_name == "none" {
                None
            } else {
                Some(self.state.active_provider_name.as_str())
            };
            match self.state.sessions.create(model, provider) {
                Ok(session) => {
                    self.state.agent.init_cold_store(&session.id);
                    self.state.current_session_id = Some(session.id);
                }
                Err(e) => {
                    tracing::warn!("Failed to create session: {e}");
                    return;
                }
            }
        }

        if let Some(ref sid) = self.state.current_session_id
            && let Err(e) = self.state.sessions.save_message(sid, role, content)
        {
            tracing::warn!("Failed to save message: {e}");
        }
    }

    /// Update the session metadata (title, turn count) in the database.
    fn update_session_meta(&mut self) {
        if let Some(ref sid) = self.state.current_session_id {
            let session = crate::session::Session {
                id: sid.clone(),
                title: self.state.session_title.clone(),
                model: Some(self.state.active_model_name.clone()),
                provider: Some(self.state.active_provider_name.clone()),
                turn_count: self.state.agent.turn_count,
                cwd: std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string()),
                created_at: chrono::Utc::now(), // not updated — SQL UPDATE doesn't touch it
                updated_at: chrono::Utc::now(),
            };
            if let Err(e) = self.state.sessions.update(&session) {
                tracing::warn!("Failed to update session: {e}");
            }
        }
    }

    /// Handle a quit request (ctrl+c). Requires two presses within 2 seconds.
    /// On second press, force-exits immediately to avoid cleanup lag.
    fn request_quit(&mut self) {
        const QUIT_TIMEOUT: Duration = Duration::from_secs(2);
        if let Some(first) = self.state.quit_first_press
            && first.elapsed() < QUIT_TIMEOUT
        {
            // Force-exit: restore terminal immediately and bail out.
            // Skips async cleanup (memory extraction, MCP disconnect) to
            // avoid the multi-second lag the user experiences.
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::event::DisableMouseCapture,
                crossterm::terminal::LeaveAlternateScreen,
                crossterm::event::DisableBracketedPaste,
                crossterm::event::PopKeyboardEnhancementFlags,
                crossterm::cursor::Show
            );
            std::process::exit(0);
        }
        self.state.quit_first_press = Some(Instant::now());
    }

    /// Extract plain text from the rendered chat lines within the given selection.
    fn extract_selected_text(&self, sel: &TextSelection) -> String {
        let (start_row, start_col, end_row, end_col) =
            if (sel.anchor_row, sel.anchor_col) <= (sel.end_row, sel.end_col) {
                (sel.anchor_row, sel.anchor_col, sel.end_row, sel.end_col)
            } else {
                (sel.end_row, sel.end_col, sel.anchor_row, sel.anchor_col)
            };

        let chat_area = match self.state.chat_area.get() {
            Some(a) => a,
            None => return String::new(),
        };
        let rendered = self.state.rendered_chat_text.borrow();
        let effective_offset = if self.state.user_scrolled_up {
            let max_scroll = self
                .state
                .total_chat_lines
                .get()
                .saturating_sub(self.state.chat_area_height.get());
            self.state.scroll_offset.min(max_scroll)
        } else {
            self.state
                .total_chat_lines
                .get()
                .saturating_sub(self.state.chat_area_height.get())
        };

        let chat_width = chat_area.width as usize;
        if chat_width == 0 {
            return String::new();
        }

        // Map rendered lines to wrapped screen rows, accounting for scroll offset
        let mut result = Vec::new();
        let mut logical_row: u16 = 0; // absolute wrapped row index
        for text in rendered.iter() {
            let wrapped_rows = if text.is_empty() {
                1
            } else {
                text.len().div_ceil(chat_width) as u16
            };
            for wrap_idx in 0..wrapped_rows {
                // Screen row = chat_area.y + (logical_row - effective_offset)
                if logical_row >= effective_offset {
                    let screen_row = chat_area.y + (logical_row - effective_offset);
                    if screen_row >= chat_area.y + chat_area.height {
                        break;
                    }
                    if screen_row >= start_row && screen_row <= end_row {
                        let line_start = (wrap_idx as usize) * chat_width;
                        let line_end = (((wrap_idx as usize) + 1) * chat_width).min(text.len());
                        let row_text = if line_start < text.len() {
                            &text[line_start..line_end]
                        } else {
                            ""
                        };

                        let col_start = if screen_row == start_row {
                            (start_col.saturating_sub(chat_area.x)) as usize
                        } else {
                            0
                        };
                        let col_end = if screen_row == end_row {
                            (end_col.saturating_sub(chat_area.x)) as usize + 1
                        } else {
                            row_text.len()
                        };

                        let clamped_start = col_start.min(row_text.len());
                        let clamped_end = col_end.min(row_text.len());
                        if clamped_start < clamped_end {
                            result.push(row_text[clamped_start..clamped_end].to_string());
                        }
                    }
                }
                logical_row += 1;
            }
        }

        result.join("\n")
    }

    /// Try to get the active provider, or attempt to resolve one.
    /// Returns None and pushes an error chat message if no provider is available.
    fn require_provider(&mut self) -> bool {
        if self.provider.is_some() {
            return true;
        }
        // Try to resolve again (user may have set env var)
        match self.state.providers.get_provider(None, None) {
            Ok(p) => {
                self.state.active_provider_name = p.name().to_string();
                self.state.active_model_name = p.model().to_string();
                self.provider = Some(p);
                true
            }
            Err(_) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: "No API key configured. Set ANTHROPIC_API_KEY, OPENAI_API_KEY, \
                              or another provider key in your environment, then restart."
                        .to_string(),
                });
                false
            }
        }
    }

    /// Connect all configured MCP servers (non-blocking, called after App::new).
    pub async fn connect_mcp_servers(&mut self) {
        if self.state.mcp_manager.servers.is_empty() {
            return;
        }

        // Connect enabled MCP servers in background (non-blocking)
        {
            let names: Vec<String> = self
                .state
                .mcp_manager
                .servers
                .iter()
                .filter(|(_, s)| !s.config.disabled)
                .map(|(n, _)| n.clone())
                .collect();
            for name in names {
                let tx = self.state.mcp_connect_tx.clone();
                let _ = self.state.mcp_manager.connect_server_background(&name, tx);
            }
        }
    }

    /// Main event loop.
    pub async fn run(&mut self) -> Result<()> {
        self.terminal.enter()?;
        // Enable bracketed paste for API key input
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste)?;

        // Fire SessionStart lifecycle hooks
        if let Some(ref hooks_config) = self.state.config.hooks
            && !hooks_config.session_start.is_empty()
        {
            let hooks = hooks_config.session_start.clone();
            let context = serde_json::json!({
                "event": "SessionStart",
                "session_id": self.state.current_session_id,
            });
            tokio::spawn(async move {
                crate::hooks::fire_hooks(&hooks, context).await;
            });
        }

        // Background update check
        {
            let current_version = env!("CARGO_PKG_VERSION").to_string();
            let (tx, rx) = tokio::sync::oneshot::channel::<String>();
            tokio::spawn(async move {
                if let Ok(latest) = crate::update::fetch_latest_version().await {
                    let latest_bare = latest.strip_prefix('v').unwrap_or(&latest);
                    if crate::update::is_newer(latest_bare, &current_version) {
                        let _ = tx.send(latest_bare.to_string());
                    }
                }
            });
            self.state.update_check_rx = Some(rx);
        }

        // Background local LLM discovery
        {
            let (tx, rx) =
                tokio::sync::oneshot::channel::<Vec<crate::provider::local::LocalServer>>();
            tokio::spawn(async move {
                let servers = crate::provider::local::discover_local_servers().await;
                let _ = tx.send(servers);
            });
            self.state.local_discovery_rx = Some(rx);
        }

        loop {
            // Expire quit confirmation after 2 seconds
            if let Some(first) = self.state.quit_first_press
                && first.elapsed() >= Duration::from_secs(2)
            {
                self.state.quit_first_press = None;
            }

            // Advance animation tick
            self.state.tick = self.state.tick.wrapping_add(1);

            // Advance caboose position when agent or /init is active (every other tick for ~10 chars/sec)
            let agent_active = matches!(
                self.state.agent.state,
                crate::agent::AgentState::Streaming
                    | crate::agent::AgentState::ExecutingTools
                    | crate::agent::AgentState::PendingApproval { .. }
                    | crate::agent::AgentState::Compacting
            );
            let init_active = self.state.init_rx.is_some();
            if (agent_active || init_active) && self.state.tick.is_multiple_of(2) {
                self.state.caboose_pos = self.state.caboose_pos.wrapping_add(1);
            }

            // Check for update check result
            if let Some(ref mut rx) = self.state.update_check_rx {
                match rx.try_recv() {
                    Ok(version) => {
                        self.state.update_available = Some(version);
                        self.state.update_check_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.state.update_check_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Poll background local LLM discovery
            if let Some(ref mut rx) = self.state.local_discovery_rx {
                match rx.try_recv() {
                    Ok(servers) => {
                        self.state.discovered_locals = servers;
                        self.state.local_discovery_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.state.local_discovery_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Poll local provider probe result
            if let Some(DialogKind::LocalProviderConnect(lpc)) = self.state.dialog_stack.top_mut()
                && let Some(rx) = &mut lpc.probe_rx
            {
                match rx.try_recv() {
                    Ok(Ok(models)) => {
                        if models.is_empty() {
                            lpc.error = Some("Server responded but no models found".to_string());
                            lpc.phase = crate::tui::dialog::LocalConnectPhase::Address;
                        } else {
                            lpc.models = models;
                            lpc.selected_model = 0;
                            lpc.phase = crate::tui::dialog::LocalConnectPhase::ModelSelect;
                        }
                        lpc.probe_rx = None;
                    }
                    Ok(Err(msg)) => {
                        lpc.error = Some(msg);
                        lpc.phase = crate::tui::dialog::LocalConnectPhase::Address;
                        lpc.probe_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        lpc.error = Some("Probe failed unexpectedly".to_string());
                        lpc.phase = crate::tui::dialog::LocalConnectPhase::Address;
                        lpc.probe_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Draw UI
            let state = &self.state;
            self.terminal.draw(|frame| {
                crate::tui::layout::render(frame, state);
            })?;

            // Keep scroll_offset tracking max_scroll whenever auto-following.
            // This ensures that if the user scrolls down later, their offset
            // is already at the right position (not stuck at some old value).
            if !self.state.user_scrolled_up {
                let max_scroll = self
                    .state
                    .total_chat_lines
                    .get()
                    .saturating_sub(self.state.chat_area_height.get());
                self.state.scroll_offset = max_scroll;
            }

            // Poll for keyboard/paste/mouse events — drain all pending to prevent
            // mouse tracking events from delaying key events
            if event::poll(Duration::from_millis(50))? {
                loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            self.handle_key(key.code, key.modifiers).await;
                        }
                        Event::Paste(text) => {
                            self.handle_paste(&text);
                        }
                        Event::Mouse(mouse) => {
                            let in_terminal = self
                                .state
                                .terminal_area
                                .get()
                                .map(|area| {
                                    mouse.row >= area.y
                                        && mouse.row < area.y + area.height
                                        && mouse.column >= area.x
                                        && mouse.column < area.x + area.width
                                })
                                .unwrap_or(false);

                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    self.state.text_selection = None;
                                    // Route to menus/dropdowns first
                                    if !self.handle_menu_scroll(true) {
                                        if in_terminal {
                                            if let Some(panel) = &mut self.state.terminal_panel {
                                                panel.scroll_up(3);
                                            }
                                        } else {
                                            let scroll_lines: u16 = 3;
                                            self.state.scroll_offset = self
                                                .state
                                                .scroll_offset
                                                .saturating_sub(scroll_lines);
                                            self.state.user_scrolled_up = true;
                                        }
                                    }
                                }
                                MouseEventKind::ScrollDown => {
                                    self.state.text_selection = None;
                                    // Route to menus/dropdowns first
                                    if !self.handle_menu_scroll(false) {
                                        if in_terminal {
                                            if let Some(panel) = &mut self.state.terminal_panel {
                                                panel.scroll_down(3);
                                            }
                                        } else {
                                            let scroll_lines: u16 = 3;
                                            self.state.scroll_offset = self
                                                .state
                                                .scroll_offset
                                                .saturating_add(scroll_lines);
                                            let max_scroll =
                                                self.state.total_chat_lines.get().saturating_sub(
                                                    self.state.chat_area_height.get(),
                                                );
                                            if self.state.scroll_offset >= max_scroll {
                                                self.state.scroll_offset = max_scroll;
                                                self.state.user_scrolled_up = false;
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::Down(_) => {
                                    self.state.text_selection = None;
                                    if in_terminal {
                                        // Check for [x] close button click (header row, last 5 cols)
                                        if let Some(area) = self.state.terminal_area.get()
                                            && mouse.row == area.y
                                            && mouse.column >= area.x + area.width.saturating_sub(5)
                                        {
                                            if let Some(panel) = &mut self.state.terminal_panel {
                                                panel.visible = false;
                                                self.state.terminal_focused = false;
                                            }
                                            continue;
                                        }
                                        self.state.terminal_focused = true;
                                    } else {
                                        self.state.terminal_focused = false;

                                        // Truncation click zone logic
                                        let zones = self.state.truncation_click_zones.borrow();
                                        let mut truncation_handled = false;
                                        for &(zone_y, msg_idx) in zones.iter() {
                                            if mouse.row == zone_y {
                                                if self.state.expanded_messages.contains(&msg_idx) {
                                                    self.state.expanded_messages.remove(&msg_idx);
                                                } else {
                                                    self.state.expanded_messages.insert(msg_idx);
                                                }
                                                truncation_handled = true;
                                                break;
                                            }
                                        }

                                        if !truncation_handled {
                                            self.state.text_selection = Some(TextSelection {
                                                anchor_row: mouse.row,
                                                anchor_col: mouse.column,
                                                end_row: mouse.row,
                                                end_col: mouse.column,
                                            });
                                        }
                                    }
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    if let Some(ref mut sel) = self.state.text_selection {
                                        sel.end_row = mouse.row;
                                        sel.end_col = mouse.column;

                                        // Auto-scroll when dragging near viewport edges
                                        if let Some(chat_rect) = self.state.chat_area.get() {
                                            let scroll_margin: u16 = 2;
                                            let scroll_speed: u16 = 2;

                                            if mouse.row < chat_rect.y + scroll_margin {
                                                // Near top edge — scroll up
                                                self.state.scroll_offset = self
                                                    .state
                                                    .scroll_offset
                                                    .saturating_sub(scroll_speed);
                                                self.state.user_scrolled_up = true;
                                            } else if mouse.row
                                                >= chat_rect.y + chat_rect.height - scroll_margin
                                            {
                                                // Near bottom edge — scroll down
                                                self.state.scroll_offset = self
                                                    .state
                                                    .scroll_offset
                                                    .saturating_add(scroll_speed);
                                                let max_scroll = self
                                                    .state
                                                    .total_chat_lines
                                                    .get()
                                                    .saturating_sub(
                                                        self.state.chat_area_height.get(),
                                                    );
                                                if self.state.scroll_offset >= max_scroll {
                                                    self.state.scroll_offset = max_scroll;
                                                    self.state.user_scrolled_up = false;
                                                }
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::Moved => {
                                    // Mouse hover selects items in command palette
                                    let palette_hit =
                                        if let Some(DialogKind::CommandPalette(palette)) =
                                            self.state.dialog_stack.top()
                                        {
                                            let (tw, th) =
                                                crossterm::terminal::size().unwrap_or((80, 24));
                                            crate::tui::command_palette::hit_test(
                                                palette,
                                                &self.state,
                                                mouse.row,
                                                th,
                                                tw,
                                            )
                                        } else {
                                            None
                                        };
                                    if let Some(idx) = palette_hit
                                        && let Some(DialogKind::CommandPalette(palette)) =
                                            self.state.dialog_stack.top_mut()
                                    {
                                        palette.selected = idx;
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    // Drain remaining pending events without waiting
                    if !event::poll(Duration::from_millis(0))? {
                        break;
                    }
                }
            }

            // Drain agent events
            let events = self.state.agent.poll();
            for event in &events {
                match event {
                    crate::agent::AgentEvent::TextDelta(_) => {
                        // Text accumulates in agent.streaming_text,
                        // which layout.rs reads during render
                    }
                    crate::agent::AgentEvent::TurnComplete { .. } => {
                        // finalize_turn() already ran inside poll().
                        // Check if we need to execute tools or show approval.
                        self.handle_turn_complete().await;
                    }
                    crate::agent::AgentEvent::ProviderError {
                        category,
                        provider,
                        message,
                        hint,
                    } => {
                        let json = serde_json::json!({
                            "category": category,
                            "provider": provider,
                            "message": message,
                            "hint": hint,
                        });
                        self.persist_message("provider_error", &json.to_string());
                        self.state.chat_messages.push(ChatMessage::ProviderError {
                            category: category.clone(),
                            provider: provider.to_string(),
                            message: message.to_string(),
                            hint: hint.clone(),
                        });
                    }
                    crate::agent::AgentEvent::Error(e) => {
                        self.state
                            .chat_messages
                            .push(ChatMessage::Error { content: e.clone() });
                    }
                    crate::agent::AgentEvent::CompactionComplete => {
                        self.state.chat_messages.push(ChatMessage::System {
                            content: "Context compacted — conversation summarized.".to_string(),
                        });

                        // Re-inject active task outline so the agent retains awareness
                        if let Some(outline) = self.state.chat_messages.iter().rev().find_map(|m| {
                            if let ChatMessage::TaskOutline(o) = m {
                                Some(o.clone())
                            } else {
                                None
                            }
                        }) {
                            let active: Vec<_> = outline
                                .tasks
                                .iter()
                                .filter(|t| {
                                    matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)
                                })
                                .collect();
                            if !active.is_empty() {
                                let mut task_text = String::from(
                                    "[Active task list (preserved across compaction)]\n",
                                );
                                for t in &active {
                                    let marker = match t.status {
                                        TaskStatus::InProgress => "[in_progress]",
                                        _ => "[pending]",
                                    };
                                    task_text.push_str(&format!("- {marker} {}\n", t.content));
                                }
                                self.state.agent.conversation.push(
                                    crate::agent::conversation::Message {
                                        role: crate::agent::conversation::Role::User,
                                        content: crate::agent::conversation::Content::Text(
                                            task_text,
                                        ),
                                        tool_call_id: None,
                                    },
                                );
                            }
                        }

                        // If compaction was auto-triggered, resume the stream
                        if !self.state.agent.stashed_tool_defs.is_empty()
                            && let Some(ref provider) = self.provider
                        {
                            let tool_defs: Vec<_> =
                                std::mem::take(&mut self.state.agent.stashed_tool_defs);
                            self.state.agent.start_stream(provider.as_ref(), &tool_defs);
                        }
                    }
                    _ => {}
                }
            }

            // Non-blocking tool execution: poll spawned tool results and
            // kick off the next tool when the previous one finishes.
            self.poll_tool_execution().await;
            self.poll_mcp_connections();

            // Poll terminal panel output
            if let Some(panel) = &mut self.state.terminal_panel
                && panel.visible
            {
                panel.poll_output();

                // Resize PTY only when dimensions actually change
                if let Some(area) = self.state.terminal_area.get() {
                    let body_h = area.height.saturating_sub(1);
                    if body_h > 0 {
                        let new_size = (area.width, body_h);
                        if self.state.terminal_last_size != Some(new_size) {
                            let _ = panel.resize(area.width, body_h);
                            self.state.terminal_last_size = Some(new_size);
                        }
                    }
                }

                // Respawn if shell exited
                if !panel.is_alive() {
                    let was_focused = panel.focused;
                    let (cols, rows) = self
                        .state
                        .terminal_area
                        .get()
                        .map(|a| (a.width, a.height.saturating_sub(1).max(1)))
                        .unwrap_or((80, 24));
                    let cwd =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let cwd_str = cwd.to_string_lossy();
                    if let Ok(mut new_panel) =
                        crate::terminal::panel::TerminalPanel::new(cols, rows, &cwd_str)
                    {
                        new_panel.visible = true;
                        new_panel.focused = was_focused;
                        self.state.terminal_panel = Some(new_panel);
                    }
                }
            }

            // Drain /init generation events (non-blocking)
            if let Some(ref mut rx) = self.state.init_rx {
                let mut done = false;
                while let Ok(event) = rx.try_recv() {
                    match event {
                        crate::init::handler::InitEvent::TextDelta(text) => {
                            self.state.init_text.push_str(&text);
                        }
                        crate::init::handler::InitEvent::Done {
                            input_tokens,
                            output_tokens,
                        } => {
                            self.state.agent.last_input_tokens = input_tokens;
                            self.state.agent.last_output_tokens = output_tokens;
                            done = true;
                            break;
                        }
                        crate::init::handler::InitEvent::Error(e) => {
                            self.state
                                .chat_messages
                                .push(ChatMessage::Error { content: e });
                            done = true;
                            break;
                        }
                    }
                }
                if done {
                    self.state.init_rx = None;
                    self.finalize_init();
                }
            }

            // Poll roundhouse planner updates (non-blocking)
            if let Some(ref mut rx) = self.state.roundhouse_update_rx {
                let mut all_done = false;
                let mut cancelled = false;
                while let Ok(update) = rx.try_recv() {
                    match update {
                        crate::roundhouse::PlannerUpdate::StatusChanged {
                            planner_index,
                            status,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                let tick = self.state.tick;
                                if planner_index == 0 {
                                    if matches!(status, crate::roundhouse::PlannerStatus::Streaming)
                                    {
                                        session.primary_streaming_text.clear();
                                    }
                                    session.primary_status = status;
                                    session.primary_status_tick = tick;
                                } else if let Some(s) =
                                    session.secondaries.get_mut(planner_index - 1)
                                {
                                    s.status = status;
                                    s.status_tick = tick;
                                }
                            }
                        }
                        crate::roundhouse::PlannerUpdate::StreamingDelta {
                            planner_index,
                            text,
                        } => {
                            if planner_index == 0
                                && let Some(ref mut session) = self.state.roundhouse_session
                            {
                                session.primary_streaming_text.push_str(&text);
                            }
                        }
                        crate::roundhouse::PlannerUpdate::TokensUsed {
                            planner_index: _,
                            input_tokens: _,
                            output_tokens: _,
                        } => {
                            // Token tracking — rolled up for future cost display
                        }
                        crate::roundhouse::PlannerUpdate::PlanComplete {
                            planner_index,
                            result,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                match result {
                                    Ok(plan) => {
                                        if planner_index == 0 {
                                            session.primary_plan = Some(plan);
                                            session.primary_status =
                                                crate::roundhouse::PlannerStatus::Done;
                                        } else if let Some(s) =
                                            session.secondaries.get_mut(planner_index - 1)
                                        {
                                            s.plan = Some(plan);
                                            s.status = crate::roundhouse::PlannerStatus::Done;
                                        }
                                    }
                                    Err(e) => {
                                        let provider_name = if planner_index == 0 {
                                            session.primary_provider.clone()
                                        } else {
                                            session
                                                .secondaries
                                                .get(planner_index - 1)
                                                .map(|s| s.provider_name.clone())
                                                .unwrap_or_else(|| {
                                                    format!("planner-{planner_index}")
                                                })
                                        };

                                        if planner_index == 0 {
                                            session.primary_status =
                                                crate::roundhouse::PlannerStatus::Failed(e.clone());
                                        } else if let Some(s) =
                                            session.secondaries.get_mut(planner_index - 1)
                                        {
                                            s.status =
                                                crate::roundhouse::PlannerStatus::Failed(e.clone());
                                        }

                                        // Any planner failure cancels the entire roundhouse
                                        self.state.chat_messages.push(ChatMessage::System {
                                            content: format!(
                                                "Roundhouse cancelled: {} failed — {e}",
                                                provider_name
                                            ),
                                        });
                                        self.state.roundhouse_session = None;
                                        self.state.roundhouse_model_add = false;
                                        cancelled = true;
                                        break;
                                    }
                                }

                                if session.all_planners_done() {
                                    session.phase =
                                        crate::roundhouse::RoundhousePhase::Synthesizing;
                                    let plan_count = session.successful_plans().len();
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: format!(
                                            "All planners complete ({plan_count} plans). Synthesizing..."
                                        ),
                                    });
                                    all_done = true;
                                }
                            }
                        }
                    }
                }
                if cancelled {
                    self.state.roundhouse_update_rx = None;
                    self.state.roundhouse_synthesis_rx = None;
                } else if all_done {
                    self.state.roundhouse_update_rx = None;
                    self.start_roundhouse_synthesis();
                }
            }

            // Poll roundhouse synthesis streaming deltas (non-blocking)
            if let Some(ref mut rx) = self.state.roundhouse_synthesis_rx {
                let mut synthesis_done = false;
                loop {
                    match rx.try_recv() {
                        Ok(delta) => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                session.synthesis_streaming_text.push_str(&delta);
                            }
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            synthesis_done = true;
                            break;
                        }
                    }
                }
                if synthesis_done {
                    if let Some(session) = &mut self.state.roundhouse_session {
                        let plan_text = session.synthesis_streaming_text.clone();
                        let prompt = session.prompt.clone().unwrap_or_default();
                        let individual_plans: Vec<(String, String)> = session
                            .successful_plans()
                            .iter()
                            .map(|(p, t)| (p.to_string(), t.to_string()))
                            .collect();
                        let individual_refs: Vec<(&str, &str)> = individual_plans
                            .iter()
                            .map(|(p, t)| (p.as_str(), t.as_str()))
                            .collect();

                        session.synthesized_plan = Some(plan_text.clone());
                        session.phase = crate::roundhouse::RoundhousePhase::Reviewing;

                        // Write plan file
                        let cwd = std::env::current_dir().unwrap_or_default();
                        let full_doc = crate::roundhouse::output::format_plans_document(
                            &prompt,
                            &individual_refs,
                            &plan_text,
                        );
                        match crate::roundhouse::output::write_plan_file(&cwd, &full_doc, &prompt) {
                            Ok(path) => {
                                session.plan_file = Some(path.clone());
                                self.state.chat_messages.push(ChatMessage::Assistant {
                                    content: format!(
                                        "## Roundhouse Plan\n\n{}\n\n---\n*Plan saved to `{}`*\n\nUse `/roundhouse execute` to implement or `/roundhouse cancel` to abort.",
                                        plan_text,
                                        path.display()
                                    ),
                                });
                            }
                            Err(e) => {
                                self.state.chat_messages.push(ChatMessage::Assistant {
                                    content: format!(
                                        "## Roundhouse Plan\n\n{}\n\n---\n*Failed to save plan file: {}*\n\nUse `/roundhouse execute` to implement or `/roundhouse cancel` to abort.",
                                        plan_text, e
                                    ),
                                });
                            }
                        }
                    }
                    self.state.roundhouse_synthesis_rx = None;
                }
            }

            // Poll circuit events (non-blocking)
            self.poll_circuit_events().await;

            // Roundhouse: transition Executing → Complete when agent goes idle
            // and there are no queued messages waiting to be sent
            if matches!(self.state.agent.state, AgentState::Idle)
                && self.state.message_queue.is_empty()
                && self
                    .state
                    .roundhouse_session
                    .as_ref()
                    .is_some_and(|rh| {
                        rh.phase == crate::roundhouse::RoundhousePhase::Executing
                    })
            {
                if let Some(ref mut rh) = self.state.roundhouse_session {
                    rh.phase = crate::roundhouse::RoundhousePhase::Complete;
                }
            }

            if self.state.should_quit {
                break;
            }
        }

        // Fire SessionEnd hooks
        if let Some(ref hooks_config) = self.state.config.hooks
            && !hooks_config.session_end.is_empty()
        {
            let context = serde_json::json!({
                "event": "SessionEnd",
                "session_id": self.state.current_session_id.as_deref().unwrap_or(""),
                "message_count": self.state.agent.conversation.messages.len(),
            });
            // Fire-and-forget — SessionEnd hooks are non-blocking
            let hooks = hooks_config.session_end.clone();
            tokio::spawn(async move {
                let _ = crate::hooks::fire_hooks(&hooks, context).await;
            });
            // Give hooks a brief moment to start
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Extract memories before exit
        self.extract_session_memories().await;

        // Clean up terminal panel
        if let Some(mut panel) = self.state.terminal_panel.take() {
            panel.kill();
        }

        // Clean up MCP servers
        self.state.mcp_manager.disconnect_all().await;

        // Gracefully shut down LSP servers
        if let Some(lsp) = self.state.lsp_manager.take() {
            lsp.shutdown_all().await;
        }

        crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste)?;
        self.terminal.exit()?;
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // If terminal panel is focused, Esc closes/minimizes it
        if self.state.terminal_focused && key == KeyCode::Esc {
            if let Some(panel) = &mut self.state.terminal_panel {
                panel.visible = false;
            }
            self.state.terminal_focused = false;
            return;
        }

        // Escape cancels active agent operations before any UI dismissal
        if key == KeyCode::Esc && !matches!(self.state.agent.state, AgentState::Idle) {
            self.cancel_all_operations();
            return;
        }

        // Escape with empty input and non-empty queue → clear queue
        if key == KeyCode::Esc
            && self.state.input.is_empty()
            && !self.state.message_queue.is_empty()
        {
            self.state
                .chat_messages
                .retain(|m| !matches!(m, ChatMessage::Queued { .. }));
            self.state.message_queue.clear();
            return;
        }

        // If terminal panel is focused, forward keys to PTY
        if self.state.terminal_focused
            && let Some(panel) = &mut self.state.terminal_panel
            && let Some(bytes) = crate::terminal::input::key_to_bytes(key, modifiers)
        {
            let _ = panel.write_input(&bytes);
            return;
        }

        // Check command registry for keybind match (only when no overlay captures input)
        if !self.state.dialog_stack.has_overlay()
            && let Some(cmd) = self.state.commands.find_keybind(key, modifiers)
            && (cmd.available)(&self.state)
        {
            // /model needs async model loading — handle specially
            if cmd.id == "model.open" {
                self.open_model_dropdown().await;
                return;
            }
            let action = (cmd.execute)(&mut self.state);
            self.process_action(action).await;
            return;
        }

        // Inline approval bar — intercept y/n/a before dialog dispatch
        if matches!(self.state.agent.state, AgentState::PendingApproval { .. }) {
            match key {
                KeyCode::Char('y') | KeyCode::Char('n') | KeyCode::Char('a') => {
                    self.handle_approval_key(key).await;
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+C dismisses any overlay and starts quit timer
        if key == KeyCode::Char('c')
            && modifiers.contains(KeyModifiers::CONTROL)
            && self.state.dialog_stack.has_overlay()
        {
            self.state.dialog_stack.pop();
            self.request_quit();
            return;
        }

        match self.state.dialog_stack.top() {
            Some(DialogKind::ApiKeyInput(_)) => self.handle_key_input_key(key).await,
            Some(DialogKind::LocalProviderConnect(_)) => self.handle_local_connect_key(key).await,
            Some(DialogKind::FileBrowser(_)) => self.handle_file_browser_key(key),
            Some(DialogKind::McpServerInput(_)) => self.handle_mcp_input_key(key),
            Some(DialogKind::CommandPalette(_)) => self.handle_command_palette_key(key).await,
            Some(DialogKind::PasteConfirm { .. }) => match key {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(DialogKind::PasteConfirm { text, .. }) =
                        self.state.dialog_stack.pop()
                    {
                        self.state.input.push_str(&text);
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.state.dialog_stack.pop();
                }
                _ => {}
            },
            Some(DialogKind::RoundhouseProviderPicker(_)) => {
                self.handle_roundhouse_picker_key(key, modifiers).await;
            }
            Some(DialogKind::CircuitsList(_)) => {
                self.handle_circuits_list_key(key, modifiers);
            }
            Some(DialogKind::MigrationChecklist(_)) => {
                self.handle_migration_checklist_key(key);
            }
            None => match self.state.dialog_stack.base {
                Screen::Home => self.handle_home_key(key, modifiers).await,
                Screen::Chat => self.handle_chat_key(key, modifiers).await,
            },
        }
    }

    async fn process_action(&mut self, action: crate::tui::command::Action) {
        use crate::tui::command::Action;
        match action {
            Action::None => {}
            Action::PushDialog(dialog) => self.state.dialog_stack.push(dialog),
            Action::EnterPickerMode(auto_state) => {
                self.state.input.clear();
                self.state.slash_auto = Some(auto_state);
            }
            Action::Quit => self.state.should_quit = true,
        }
    }

    async fn handle_home_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // Ctrl+C always goes to quit logic, even when a picker/dropdown is open
        if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(sel) = self.state.text_selection.take() {
                let text = self.extract_selected_text(&sel);
                if !text.is_empty() {
                    let _ = crate::clipboard::copy_to_clipboard(&text);
                }
            } else {
                self.request_quit();
            }
            return;
        }

        // Picker mode has its own key handling
        if self
            .state
            .slash_auto
            .as_ref()
            .map(|a| a.is_picker())
            .unwrap_or(false)
        {
            self.handle_picker_key(key).await;
            return;
        }

        // File autocomplete interception
        if let Some(ref mut auto) = self.state.file_auto {
            match (key, modifiers) {
                (KeyCode::Tab, _) | (KeyCode::Enter, KeyModifiers::NONE) => {
                    if let Some(path) = auto.selected_path().map(|s| s.to_string()) {
                        let content = self.state.input.content();
                        if let Some(at_pos) = content.rfind('@') {
                            let before = &content[..at_pos];
                            let new_content = format!("{before}@{path} ");
                            self.state.input.set(&new_content);
                        }
                        self.state.file_auto = None;
                    }
                    return;
                }
                (KeyCode::Up, _) => {
                    auto.select_up();
                    return;
                }
                (KeyCode::Down, _) => {
                    auto.select_down();
                    return;
                }
                (KeyCode::Esc, _) => {
                    self.state.file_auto = None;
                    return;
                }
                _ => {
                    // Fall through to normal handling
                }
            }
        }

        // Slash autocomplete interception
        if let Some(auto_ref) = self.state.slash_auto.as_ref() {
            let selected = auto_ref.selected;
            let input_text = self.state.input.content();
            let (_result, completion) = crate::tui::slash_auto::handle_slash_key(
                key,
                &input_text,
                selected,
                &self.state.commands,
                &self.state.skills,
            );
            match key {
                KeyCode::Up => {
                    if let Some(auto) = self.state.slash_auto.as_mut() {
                        auto.selected = auto.selected.saturating_sub(1);
                    }
                    return;
                }
                KeyCode::Down => {
                    let prefix = crate::tui::slash_auto::slash_prefix(&input_text).unwrap_or("");
                    let count = crate::tui::slash_auto::total_filtered(
                        prefix,
                        &self.state.commands,
                        &self.state.skills,
                    );
                    if let Some(auto) = self.state.slash_auto.as_mut()
                        && auto.selected + 1 < count
                    {
                        auto.selected += 1;
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.state.slash_auto = None;
                    return;
                }
                KeyCode::Tab => {
                    if let Some(completed) = completion {
                        self.state.input.set(&completed);
                    }
                    self.state.slash_auto = None;
                    return;
                }
                KeyCode::Enter => {
                    // Only apply autocomplete if the input has no arguments
                    // (no space after the slash command prefix). This lets
                    // `/circuit 1m "hello"` fall through without being
                    // replaced by `/circuits`.
                    let has_args = input_text.trim_start().find(' ').is_some();
                    if !has_args && let Some(completed) = completion {
                        self.state.input.set(&completed);
                    }
                    self.state.slash_auto = None;
                    // Fall through to normal Enter handler to execute the command
                }
                _ => {
                    // Fallthrough — let normal handler process Char/Backspace,
                    // then update slash_auto after.
                }
            }
        }

        match (key, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                if let Some(sel) = self.state.text_selection.take() {
                    let text = self.extract_selected_text(&sel);
                    if !text.is_empty() {
                        let _ = crate::clipboard::copy_to_clipboard(&text);
                    }
                } else {
                    self.request_quit();
                }
            }
            (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new()
                    && let Ok(text) = clipboard.get_text()
                {
                    self.handle_paste(&text);
                }
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                self.state.dialog_stack.push(DialogKind::FileBrowser(
                    crate::tui::file_browser::FileBrowserState::new(cwd),
                ));
            }
            (KeyCode::Enter, KeyModifiers::SHIFT)
            | (KeyCode::Enter, KeyModifiers::ALT)
            | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                self.state.input.insert_newline();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if !self.state.input.is_empty() {
                    let mut message = self.state.input.content();
                    self.state.history.push(message.clone());
                    self.state.history.save();
                    self.state.input.clear();

                    // Handle slash commands via registry
                    let trimmed = message.trim();
                    if let Some(slash) = trimmed.strip_prefix('/') {
                        if let Some(title_rest) = slash.strip_prefix("title ") {
                            let new_title = title_rest.trim().to_string();
                            if !new_title.is_empty() {
                                self.state.session_title = Some(new_title.clone());
                                self.update_session_meta();
                                self.state.chat_messages.push(ChatMessage::System {
                                    content: format!("Session renamed to \"{new_title}\""),
                                });
                            }
                            return;
                        }
                        if slash == "init" {
                            self.handle_init_command();
                            return;
                        }
                        if slash == "mcp" {
                            self.open_mcp_picker();
                            return;
                        }
                        if slash.starts_with("mcp ") {
                            self.handle_mcp_command(slash).await;
                            return;
                        }
                        // /model needs async model loading — handle specially
                        if slash == "model" {
                            self.open_model_dropdown().await;
                            return;
                        }
                        // /memories — display current memory contents
                        if slash == "memories" {
                            self.handle_memories_command();
                            return;
                        }
                        // /forget — list memories for removal
                        if slash == "forget" {
                            self.handle_forget_command();
                            return;
                        }
                        // /create-skill — LLM-guided skill creation
                        if slash.starts_with("create-skill") {
                            let args = slash.strip_prefix("create-skill").unwrap_or("").trim();
                            self.handle_create_skill_command(args);
                            return;
                        }
                        // /settings — open settings picker
                        if slash == "settings" {
                            self.open_settings_picker();
                            return;
                        }
                        // /rewind — open checkpoint picker
                        if slash == "rewind" {
                            self.open_rewind_picker();
                            return;
                        }
                        // /roundhouse execute|cancel — subcommands
                        if let Some(sub) = slash.strip_prefix("roundhouse ") {
                            self.handle_roundhouse_subcommand(sub.trim());
                            return;
                        }
                        // /circuit [--persist] <interval> "prompt" | stop <id> | stop-all
                        if let Some(args) = slash.strip_prefix("circuit ") {
                            self.handle_circuit_command(args.trim()).await;
                            return;
                        }
                        // /watch pr <number> [--persist] | /watch mr <number> [--persist]
                        if let Some(args) = slash.strip_prefix("watch ") {
                            self.handle_watch_command(args.trim()).await;
                            return;
                        }
                        // /new — extract memories and clean up cold storage before clearing session
                        if slash == "new" {
                            self.extract_session_memories().await;
                            if let Some(ref store) = self.state.agent.cold_store {
                                let _ = store.cleanup();
                            }
                        }
                        if let Some(cmd) = self.state.commands.find_slash(slash)
                            && (cmd.available)(&self.state)
                        {
                            let action = (cmd.execute)(&mut self.state);
                            self.process_action(action).await;
                            return;
                        }

                        // Try skill resolution after command registry check
                        {
                            // Reload skills from disk before resolution (picks up external changes)
                            let skills_disabled = self
                                .state
                                .config
                                .skills
                                .as_ref()
                                .map(|s| s.disabled.clone())
                                .unwrap_or_default();
                            self.state.skills = crate::skills::loader::load_all_skills(
                                std::path::Path::new("."),
                                &skills_disabled,
                            );

                            let slash_name = slash.split_whitespace().next().unwrap_or(slash);
                            let args = slash.strip_prefix(slash_name).unwrap_or("").trim();
                            let command_names: Vec<&str> = self
                                .state
                                .commands
                                .slash_commands()
                                .filter_map(|c| c.slash)
                                .collect();
                            let resolution = crate::skills::resolver::resolve_slash_name(
                                slash_name,
                                &command_names,
                                &self.state.skills,
                            );
                            if let crate::skills::SlashResolution::Skill(skill) = resolution {
                                let cwd = std::env::current_dir()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string();
                                let expanded =
                                    crate::skills::expand::expand_skill(&skill, args, &cwd);
                                // Show inline skill marker
                                self.state.chat_messages.push(ChatMessage::Skill {
                                    name: skill.name.clone(),
                                    description: skill.description.clone(),
                                });
                                self.persist_message(
                                    "skill",
                                    &serde_json::json!({
                                        "name": skill.name,
                                        "description": skill.description,
                                    })
                                    .to_string(),
                                );
                                // Require provider
                                if !self.require_provider() {
                                    return;
                                }
                                // Send expanded template as user message
                                self.state.chat_messages.push(ChatMessage::User {
                                    content: expanded.clone(),
                                    images: vec![],
                                });
                                self.state.user_scrolled_up = false;
                                self.persist_message("user", &expanded);
                                self.state.dialog_stack.base = Screen::Chat;
                                self.state.dialog_stack.clear();
                                self.state.checkpoints.create(&expanded);
                                let tool_defs = self.build_tool_defs();
                                self.state.agent.send_message(
                                    expanded,
                                    self.provider.as_ref().unwrap().as_ref(),
                                    &tool_defs,
                                );
                                return;
                            }
                        }
                    }

                    // Require a provider before sending
                    if !self.require_provider() {
                        self.state.input.set(&message);
                        return;
                    }

                    // Set session title from first message (truncated at word boundary)
                    self.state.session_title_source = Some(message.clone());
                    let truncated =
                        crate::tui::session_picker::truncate_at_word_boundary(&message, 60);
                    self.state.session_title = Some(truncated);

                    // Transition to chat and submit
                    self.state.dialog_stack.base = Screen::Chat;
                    self.state.dialog_stack.clear();
                    self.state.chat_messages.push(ChatMessage::User {
                        content: message.clone(),
                        images: vec![],
                    });
                    self.state.user_scrolled_up = false;
                    self.persist_message("user", &message);

                    // Fire UserPromptSubmit lifecycle hooks
                    if let Some(ref hooks_config) = self.state.config.hooks
                        && !hooks_config.user_prompt_submit.is_empty()
                    {
                        let context = serde_json::json!({
                            "event": "UserPromptSubmit",
                            "prompt": message,
                            "session_id": self.state.current_session_id,
                        });
                        let results =
                            crate::hooks::fire_hooks(&hooks_config.user_prompt_submit, context)
                                .await;
                        let denied = results.iter().find_map(|r| {
                            if let Some(crate::hooks::HookAction::Deny(reason)) = &r.action {
                                Some(reason.clone())
                            } else {
                                None
                            }
                        });
                        if let Some(reason) = denied {
                            self.state.chat_messages.push(ChatMessage::System {
                                content: format!("Message blocked by hook: {reason}"),
                            });
                            return;
                        }

                        // Collect context injections from hooks
                        let injected_context: Vec<String> = results
                            .iter()
                            .filter_map(|r| crate::hooks::parse_context(&r.stdout))
                            .collect();
                        if !injected_context.is_empty() {
                            let ctx = injected_context.join("\n");
                            message = format!("[Hook context: {ctx}]\n\n{message}");
                        }
                    }

                    self.state.checkpoints.create(&message);
                    let tool_defs = self.build_tool_defs();
                    self.state.agent.send_message(
                        message,
                        self.provider.as_ref().unwrap().as_ref(),
                        &tool_defs,
                    );
                }
            }
            (KeyCode::Left, KeyModifiers::NONE) if !self.state.input.is_empty() => {
                self.state.input.move_left();
            }
            (KeyCode::Right, KeyModifiers::NONE) if !self.state.input.is_empty() => {
                self.state.input.move_right();
            }
            (KeyCode::Home, KeyModifiers::NONE) if !self.state.input.is_empty() => {
                self.state.input.cursor_col = 0;
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                if self.state.input.cursor_row > 0 {
                    self.state.input.move_up();
                } else if let Some(entry) =
                    self.state.history.browse_up(&self.state.input.content())
                {
                    self.state.input.set(&entry);
                }
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                if self.state.input.cursor_row < self.state.input.line_count() - 1 {
                    self.state.input.move_down();
                } else if let Some(entry) = self.state.history.browse_down() {
                    self.state.input.set(&entry);
                }
            }
            (KeyCode::Tab, KeyModifiers::NONE) if self.state.input.is_empty() => {
                // Cycle mode: Plan → Create → Chug → Plan
                self.state.mode = self.state.mode.next();
                self.state.agent.permission_mode = self.state.mode.to_permission_mode();
            }
            (KeyCode::Char(c), _) => {
                self.state.history.reset();
                self.state.input.insert_char(c);
                self.state.update_slash_auto();
                self.state.update_file_auto();
            }
            (KeyCode::Backspace, _) => {
                if self.state.input.is_empty() && !self.state.attachments.is_empty() {
                    self.state.attachments.pop();
                } else {
                    self.state.input.backspace();
                    self.state.update_slash_auto();
                    self.state.update_file_auto();
                }
            }
            _ => {}
        }
    }

    async fn handle_chat_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // Ctrl+C always goes to quit/cancel logic, even when a picker/dropdown is open
        if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(sel) = self.state.text_selection.take() {
                let text = self.extract_selected_text(&sel);
                if !text.is_empty() {
                    let _ = crate::clipboard::copy_to_clipboard(&text);
                }
            } else if matches!(self.state.agent.state, AgentState::PendingApproval { .. }) {
                self.cancel_all_operations();
            } else if !matches!(self.state.agent.state, AgentState::Idle) {
                self.cancel_all_operations();
                self.request_quit();
            } else {
                self.request_quit();
            }
            return;
        }

        // Picker mode has its own key handling
        if self
            .state
            .slash_auto
            .as_ref()
            .map(|a| a.is_picker())
            .unwrap_or(false)
        {
            self.handle_picker_key(key).await;
            return;
        }

        // If an ask_user session is active, route keys there
        if self.state.ask_user_session.is_some() {
            self.handle_ask_user_key(key);
            return;
        }

        // File autocomplete interception
        if let Some(ref mut auto) = self.state.file_auto {
            match (key, modifiers) {
                (KeyCode::Tab, _) | (KeyCode::Enter, KeyModifiers::NONE) => {
                    if let Some(path) = auto.selected_path().map(|s| s.to_string()) {
                        let content = self.state.input.content();
                        if let Some(at_pos) = content.rfind('@') {
                            let before = &content[..at_pos];
                            let new_content = format!("{before}@{path} ");
                            self.state.input.set(&new_content);
                        }
                        self.state.file_auto = None;
                    }
                    return;
                }
                (KeyCode::Up, _) => {
                    auto.select_up();
                    return;
                }
                (KeyCode::Down, _) => {
                    auto.select_down();
                    return;
                }
                (KeyCode::Esc, _) => {
                    self.state.file_auto = None;
                    return;
                }
                _ => {
                    // Fall through to normal handling
                }
            }
        }

        // Slash autocomplete interception
        if let Some(auto_ref) = self.state.slash_auto.as_ref() {
            let selected = auto_ref.selected;
            let input_text = self.state.input.content();
            let (_result, completion) = crate::tui::slash_auto::handle_slash_key(
                key,
                &input_text,
                selected,
                &self.state.commands,
                &self.state.skills,
            );
            match key {
                KeyCode::Up => {
                    if let Some(auto) = self.state.slash_auto.as_mut() {
                        auto.selected = auto.selected.saturating_sub(1);
                    }
                    return;
                }
                KeyCode::Down => {
                    let prefix = crate::tui::slash_auto::slash_prefix(&input_text).unwrap_or("");
                    let count = crate::tui::slash_auto::total_filtered(
                        prefix,
                        &self.state.commands,
                        &self.state.skills,
                    );
                    if let Some(auto) = self.state.slash_auto.as_mut()
                        && auto.selected + 1 < count
                    {
                        auto.selected += 1;
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.state.slash_auto = None;
                    return;
                }
                KeyCode::Tab => {
                    if let Some(completed) = completion {
                        self.state.input.set(&completed);
                    }
                    self.state.slash_auto = None;
                    return;
                }
                KeyCode::Enter => {
                    // Only apply autocomplete if the input has no arguments
                    // (no space after the slash command prefix). This lets
                    // `/circuit 1m "hello"` fall through without being
                    // replaced by `/circuits`.
                    let has_args = input_text.trim_start().find(' ').is_some();
                    if !has_args && let Some(completed) = completion {
                        self.state.input.set(&completed);
                    }
                    self.state.slash_auto = None;
                    // Fall through to normal Enter handler to execute the command
                }
                _ => {
                    // Fallthrough — let normal handler process Char/Backspace,
                    // then update slash_auto after.
                }
            }
        }

        // Handle pending handoff confirmation
        if self.state.pending_handoff.is_some() {
            match key {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let summary = self.state.pending_handoff.take().unwrap();

                    // Extract memories before clearing
                    self.extract_session_memories().await;

                    // Clear current session (same as /new)
                    self.state.chat_messages.clear();
                    self.state.input.clear();
                    self.state.scroll_offset = 0;
                    self.state.user_scrolled_up = false;
                    self.state.session_title = None;
                    self.state.session_title_source = None;
                    self.state.current_session_id = None;
                    self.state.modified_files.clear();
                    self.state.file_baselines.clear();
                    self.state.tool_counts.clear();
                    self.state.focused_tool = None;
                    self.state.pending_handoff = None;
                    self.state.agent.cancel();
                    self.state.agent.conversation.messages.clear();
                    self.state.agent.turn_count = 0;
                    self.state.agent.session_allows.clear();
                    self.state.agent.handoff_prompted = false;

                    // Stay on chat screen and send handoff as first message
                    self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
                    self.state.dialog_stack.clear();

                    // Send the handoff summary as the first user message in the new session
                    let handoff_msg = format!(
                        "Here is a handoff summary from my previous session. \
                         Please review it and continue where I left off.\n\n{}",
                        summary
                    );

                    // Follow the same flow as normal message submission
                    self.state.chat_messages.push(ChatMessage::User {
                        content: handoff_msg.clone(),
                        images: vec![],
                    });
                    self.state.user_scrolled_up = false;
                    self.persist_message("user", &handoff_msg);

                    if self.require_provider() {
                        let tool_defs = self.build_tool_defs();
                        self.state.agent.send_message(
                            handoff_msg,
                            self.provider.as_ref().unwrap().as_ref(),
                            &tool_defs,
                        );
                    }

                    return;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.state.pending_handoff = None;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "Handoff cancelled. Summary remains in chat.".into(),
                    });
                    return;
                }
                _ => return, // Ignore other keys while confirming
            }
        }

        // Handle budget pause confirmation
        if self.state.budget_paused {
            match key {
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    // Continue — dismiss pause, allow next request (will pause again next turn)
                    self.state.budget_paused = false;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "Budget pause dismissed. Continuing...".into(),
                    });
                    // Resume the agent loop
                    if let Some(ref provider) = self.provider {
                        let tool_defs = self.build_tool_defs();
                        self.state
                            .agent
                            .continue_after_tools(provider.as_ref(), &tool_defs);
                    }
                    return;
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    // Raise limit — set a new budget (double the current)
                    let current_max = self
                        .state
                        .config
                        .behavior
                        .as_ref()
                        .and_then(|b| b.max_session_cost)
                        .unwrap_or(0.0);
                    let new_max = (current_max * 2.0).max(self.state.session_cost + 1.0);
                    self.state
                        .config
                        .behavior
                        .get_or_insert_with(Default::default)
                        .max_session_cost = Some(new_max);
                    self.state.budget_paused = false;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!("Budget raised to ${:.2}. Continuing...", new_max),
                    });
                    // Resume the agent loop
                    if let Some(ref provider) = self.provider {
                        let tool_defs = self.build_tool_defs();
                        self.state
                            .agent
                            .continue_after_tools(provider.as_ref(), &tool_defs);
                    }
                    return;
                }
                KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Esc => {
                    // Stop — return to idle
                    self.state.budget_paused = false;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "Stopped. You can still type — the agent won't auto-continue."
                            .into(),
                    });
                    return;
                }
                _ => return, // Ignore other keys while budget paused
            }
        }

        match (key, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                // Priority 1: Copy text selection
                if let Some(sel) = self.state.text_selection.take() {
                    let text = self.extract_selected_text(&sel);
                    if !text.is_empty() {
                        let _ = crate::clipboard::copy_to_clipboard(&text);
                    }
                }
                // During tool approval, Ctrl+C = deny (no quit timer)
                else if matches!(self.state.agent.state, AgentState::PendingApproval { .. }) {
                    self.cancel_all_operations();
                }
                // Priority 2: Cancel active operation + start quit timer
                // (so next Ctrl+C quits immediately)
                else if !matches!(self.state.agent.state, AgentState::Idle) {
                    self.cancel_all_operations();
                    self.request_quit();
                }
                // Priority 3: Quit (two-press)
                else {
                    self.request_quit();
                }
            }
            (KeyCode::Char('v'), KeyModifiers::CONTROL) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new()
                    && let Ok(text) = clipboard.get_text()
                {
                    self.handle_paste(&text);
                }
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                self.state.dialog_stack.push(DialogKind::FileBrowser(
                    crate::tui::file_browser::FileBrowserState::new(cwd),
                ));
            }
            // Skill creation preview keys (p/g/e/c) — intercept before normal input
            (KeyCode::Char(c @ ('p' | 'g' | 'e' | 'c')), KeyModifiers::NONE)
                if self.state.input.is_empty()
                    && matches!(self.state.agent.state, AgentState::Idle)
                    && self.handle_skill_creation_key(KeyCode::Char(c)) =>
            {
                // Consumed by handle_skill_creation_key
            }
            (KeyCode::Enter, KeyModifiers::SHIFT)
            | (KeyCode::Enter, KeyModifiers::ALT)
            | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                self.state.input.insert_newline();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if !self.state.input.is_empty()
                    && !matches!(self.state.agent.state, AgentState::Idle)
                    && self.state.message_queue.len() < 3
                {
                    // Agent is busy — queue the message
                    let content = self.state.input.content().to_string();
                    self.state.history.push(content.clone());
                    self.state.input.clear();
                    self.state.slash_auto = None;
                    self.state.file_auto = None;

                    self.state.chat_messages.push(ChatMessage::Queued {
                        content: content.clone(),
                    });
                    self.state.message_queue.push_back(content);
                    self.state.user_scrolled_up = false;
                } else if !self.state.input.is_empty()
                    && matches!(self.state.agent.state, AgentState::Idle)
                {
                    // Skill creation conversational phases
                    if let Some(ref creation) = self.state.skill_creation {
                        match creation.phase {
                            crate::skills::creation::SkillCreationPhase::AwaitingName => {
                                let input = self.state.input.content().trim().to_string();
                                self.state.input.clear();
                                self.state.chat_messages.push(ChatMessage::User {
                                    content: input.clone(),
                                    images: vec![],
                                });
                                self.state.user_scrolled_up = false;
                                let name = input.to_lowercase().replace(' ', "-");
                                if name.is_empty() {
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content:
                                            "Name can't be empty. What do you want to call it?"
                                                .into(),
                                    });
                                    return;
                                }
                                if crate::skills::creation::is_reserved_name(&name) {
                                    self.state.chat_messages.push(ChatMessage::Error {
                                        content: format!(
                                            "'{name}' is reserved. Try a different name."
                                        ),
                                    });
                                    return;
                                }
                                self.state.skill_creation.as_mut().unwrap().name = name;
                                self.state.skill_creation.as_mut().unwrap().phase =
                                    crate::skills::creation::SkillCreationPhase::AwaitingGoal;
                                self.state.chat_messages.push(ChatMessage::System {
                                    content: "What should this skill do? Describe the goal in a sentence or two.".into(),
                                });
                                return;
                            }
                            crate::skills::creation::SkillCreationPhase::AwaitingGoal => {
                                let goal = self.state.input.content().trim().to_string();
                                self.state.input.clear();
                                self.state.chat_messages.push(ChatMessage::User {
                                    content: goal.clone(),
                                    images: vec![],
                                });
                                self.state.user_scrolled_up = false;
                                if goal.is_empty() {
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: "Goal can't be empty. What should the skill do?"
                                            .into(),
                                    });
                                    return;
                                }
                                let name = self.state.skill_creation.as_ref().unwrap().name.clone();
                                self.start_skill_creation(name, goal);
                                return;
                            }
                            _ => {} // Gathering/Preview — fall through to normal handling
                        }
                    }

                    // Roundhouse: intercept Enter when awaiting planning prompt
                    // Roundhouse: intercept Enter when awaiting planning prompt
                    if self.state.roundhouse_session.as_ref().is_some_and(|rh| {
                        rh.phase == crate::roundhouse::types::RoundhousePhase::AwaitingPrompt
                    }) {
                        let prompt = self.state.input.content().trim().to_string();
                        self.state.input.clear();
                        self.state.user_scrolled_up = false;
                        if !prompt.is_empty() {
                            if let Some(ref mut rh) = self.state.roundhouse_session {
                                rh.prompt = Some(prompt.clone());
                                rh.phase = crate::roundhouse::types::RoundhousePhase::Planning;
                            }
                            self.state.chat_messages.push(ChatMessage::User {
                                content: format!("[Roundhouse] {prompt}"),
                                images: vec![],
                            });
                            self.state.chat_messages.push(ChatMessage::System {
                                content: "Roundhouse planning started...".to_string(),
                            });
                            self.start_roundhouse_planning();
                        }
                        return;
                    }

                    let message = self.state.input.content();
                    self.state.history.push(message.clone());
                    self.state.history.save();
                    self.state.input.clear();
                    self.state.user_scrolled_up = false;

                    // Handle slash commands via registry
                    let trimmed = message.trim();
                    if let Some(slash) = trimmed.strip_prefix('/') {
                        // Special handling for /compact (needs provider access)
                        if slash == "compact" {
                            if !self.require_provider() {
                                self.state.input.set(&message);
                                return;
                            }
                            // NOTE: compaction_model override deferred to post-launch.
                            // When wired, resolve compaction_model to a Provider via ProviderRegistry.
                            if let Some(ref model) = self
                                .state
                                .config
                                .behavior
                                .as_ref()
                                .and_then(|b| b.compaction_model.clone())
                            {
                                tracing::info!(
                                    compaction_model = %model,
                                    "compaction_model configured but not yet wired — using active provider"
                                );
                            }
                            self.state.chat_messages.push(ChatMessage::System {
                                content: "Compacting conversation...".to_string(),
                            });
                            // Fire PreCompact hooks and collect must_keep context
                            let must_keep_context = if let Some(ref hooks_config) =
                                self.state.config.hooks
                                && !hooks_config.pre_compact.is_empty()
                            {
                                let context = serde_json::json!({
                                    "event": "PreCompact",
                                    "session_id": self.state.current_session_id.as_deref().unwrap_or(""),
                                    "message_count": self.state.agent.conversation.messages.len(),
                                });
                                let results =
                                    crate::hooks::fire_hooks(&hooks_config.pre_compact, context)
                                        .await;
                                let must_keep: Vec<String> = results
                                    .iter()
                                    .filter_map(|r| crate::hooks::parse_must_keep(&r.stdout))
                                    .collect();
                                if must_keep.is_empty() {
                                    None
                                } else {
                                    Some(must_keep.join("\n"))
                                }
                            } else {
                                None
                            };
                            self.state.agent.compact(
                                self.provider.as_ref().unwrap().as_ref(),
                                must_keep_context.as_deref(),
                            );
                            return;
                        }
                        if let Some(title_rest) = slash.strip_prefix("title ") {
                            let new_title = title_rest.trim().to_string();
                            if !new_title.is_empty() {
                                self.state.session_title = Some(new_title.clone());
                                self.update_session_meta();
                                self.state.chat_messages.push(ChatMessage::System {
                                    content: format!("Session renamed to \"{new_title}\""),
                                });
                            }
                            return;
                        }
                        if slash == "init" {
                            self.handle_init_command();
                            return;
                        }
                        // /create-skill — LLM-guided skill creation
                        if slash.starts_with("create-skill") {
                            let args = slash.strip_prefix("create-skill").unwrap_or("").trim();
                            self.handle_create_skill_command(args);
                            return;
                        }
                        // /cancel — cancel skill creation
                        if slash == "cancel" && self.state.skill_creation.is_some() {
                            self.state.skill_creation = None;
                            self.state.chat_messages.push(ChatMessage::System {
                                content: "Skill creation cancelled.".into(),
                            });
                            return;
                        }
                        if slash == "mcp" {
                            self.open_mcp_picker();
                            return;
                        }
                        if slash.starts_with("mcp ") {
                            self.handle_mcp_command(slash).await;
                            return;
                        }
                        // /model needs async model loading — handle specially
                        if slash == "model" {
                            self.open_model_dropdown().await;
                            return;
                        }
                        // /memories — display current memory contents
                        if slash == "memories" {
                            self.handle_memories_command();
                            return;
                        }
                        // /forget — list memories for removal
                        if slash == "forget" {
                            self.handle_forget_command();
                            return;
                        }
                        // /settings — open settings picker
                        if slash == "settings" {
                            self.open_settings_picker();
                            return;
                        }
                        // /rewind — open checkpoint picker
                        if slash == "rewind" {
                            self.open_rewind_picker();
                            return;
                        }
                        // /handoff — build handoff summary
                        if slash == "handoff" || slash.starts_with("handoff ") {
                            let args = slash.strip_prefix("handoff").unwrap_or("").trim();
                            self.handle_handoff_command(args).await;
                            return;
                        }
                        // /roundhouse execute|cancel — subcommands
                        if let Some(sub) = slash.strip_prefix("roundhouse ") {
                            self.handle_roundhouse_subcommand(sub.trim());
                            return;
                        }
                        // /circuit [--persist] <interval> "prompt" | stop <id> | stop-all
                        if let Some(args) = slash.strip_prefix("circuit ") {
                            self.handle_circuit_command(args.trim()).await;
                            return;
                        }
                        // /watch pr <number> [--persist] | /watch mr <number> [--persist]
                        if let Some(args) = slash.strip_prefix("watch ") {
                            self.handle_watch_command(args.trim()).await;
                            return;
                        }
                        // /new — extract memories and clean up cold storage before clearing session
                        if slash == "new" {
                            self.extract_session_memories().await;
                            if let Some(ref store) = self.state.agent.cold_store {
                                let _ = store.cleanup();
                            }
                        }
                        if let Some(cmd) = self.state.commands.find_slash(slash)
                            && (cmd.available)(&self.state)
                        {
                            let action = (cmd.execute)(&mut self.state);
                            self.process_action(action).await;
                            return;
                        }
                    }

                    if !self.require_provider() {
                        self.state.input.set(&message);
                        return;
                    }

                    self.persist_message("user", &message);
                    self.state.checkpoints.create(&message);

                    // During skill creation at question limit, append force-generate directive
                    let mut msg_to_send = message.clone();
                    if let Some(ref creation) = self.state.skill_creation
                        && creation.question_count
                            >= crate::skills::creation::MAX_CREATION_QUESTIONS
                    {
                        msg_to_send.push_str("\n\nPlease generate the skill now based on what you know. Use the generate_skill tool.");
                    }

                    // Resolve @file image references
                    let image_paths = crate::attachment::extract_at_image_paths(&msg_to_send);
                    for path_str in &image_paths {
                        let path = std::path::Path::new(path_str);
                        let full_path = if path.is_absolute() {
                            path.to_path_buf()
                        } else {
                            std::env::current_dir().unwrap_or_default().join(path)
                        };
                        match crate::attachment::read_image_attachment(&full_path) {
                            Ok(att) => self.state.attachments.push(att),
                            Err(e) => {
                                self.state.chat_messages.push(ChatMessage::Error {
                                    content: format!("Failed to attach {path_str}: {e}"),
                                });
                            }
                        }
                    }

                    // Collect image metadata for chat display before draining
                    let image_info: Vec<(String, usize)> = self
                        .state
                        .attachments
                        .iter()
                        .map(|att| (att.display_name.clone(), att.data.len()))
                        .collect();

                    self.state.chat_messages.push(ChatMessage::User {
                        content: message,
                        images: image_info,
                    });
                    self.state.user_scrolled_up = false;

                    // Check vision support before sending images
                    if !self.state.attachments.is_empty() && !self.state.model_supports_vision {
                        self.state.chat_messages.push(ChatMessage::System {
                            content: "Current model does not support images. Attachments removed."
                                .into(),
                        });
                        self.state.attachments.clear();
                    }

                    // Build content blocks from text + attachments
                    let has_attachments = !self.state.attachments.is_empty();
                    if has_attachments {
                        use base64::Engine;
                        let engine = base64::engine::general_purpose::STANDARD;
                        let mut blocks = vec![ContentBlock::Text { text: msg_to_send }];
                        for att in self.state.attachments.drain(..) {
                            blocks.push(ContentBlock::Image {
                                media_type: att.media_type,
                                data: engine.encode(&att.data),
                                source_path: Some(att.path.display().to_string()),
                            });
                        }
                        let tool_defs = self.build_tool_defs();
                        self.state.agent.send_message_with_blocks(
                            blocks,
                            self.provider.as_ref().unwrap().as_ref(),
                            &tool_defs,
                        );
                    } else {
                        let tool_defs = self.build_tool_defs();
                        self.state.agent.send_message(
                            msg_to_send,
                            self.provider.as_ref().unwrap().as_ref(),
                            &tool_defs,
                        );
                    }
                }
            }
            (KeyCode::PageUp, _) => {
                let page = self.state.chat_area_height.get().max(1);
                self.state.scroll_offset = self.state.scroll_offset.saturating_sub(page);
                self.state.user_scrolled_up = true;
            }
            (KeyCode::PageDown, _) => {
                let page = self.state.chat_area_height.get().max(1);
                self.state.scroll_offset = self.state.scroll_offset.saturating_add(page);
                let max_scroll = self
                    .state
                    .total_chat_lines
                    .get()
                    .saturating_sub(self.state.chat_area_height.get());
                if self.state.scroll_offset >= max_scroll {
                    self.state.scroll_offset = max_scroll;
                    self.state.user_scrolled_up = false;
                }
            }
            (KeyCode::Tab, KeyModifiers::NONE) if self.state.input.is_empty() => {
                // Cycle mode: Plan → Create → Chug → Plan
                // Only when agent is idle (not streaming/executing/pending)
                if matches!(self.state.agent.state, AgentState::Idle) {
                    self.state.mode = self.state.mode.next();
                    self.state.agent.permission_mode = self.state.mode.to_permission_mode();
                }
            }
            (KeyCode::Esc, KeyModifiers::NONE) if self.state.focused_tool.is_some() => {
                self.state.focused_tool = None;
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                if self.state.input.is_empty() && self.state.focused_tool.is_some() {
                    // 1. Tool focus navigation
                    if let Some(current) = self.state.focused_tool {
                        let prev = self.state.chat_messages[..current]
                            .iter()
                            .rposition(|m| matches!(m, ChatMessage::Tool(_)));
                        if let Some(prev_idx) = prev {
                            self.state.focused_tool = Some(prev_idx);
                        }
                    }
                } else if self.state.input.cursor_row > 0 {
                    // 2. Multi-line cursor movement
                    self.state.input.move_up();
                } else if let Some(entry) =
                    self.state.history.browse_up(&self.state.input.content())
                {
                    // 3. History browsing
                    self.state.input.set(&entry);
                } else if self.state.input.is_empty() {
                    // 4. Chat scrolling
                    self.state.scroll_offset = self.state.scroll_offset.saturating_sub(1);
                    self.state.user_scrolled_up = true;
                }
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                if self.state.input.is_empty() && self.state.focused_tool.is_some() {
                    // 1. Tool focus navigation
                    if let Some(current) = self.state.focused_tool {
                        let next = self.state.chat_messages[current + 1..]
                            .iter()
                            .position(|m| matches!(m, ChatMessage::Tool(_)))
                            .map(|i| i + current + 1);
                        if let Some(next_idx) = next {
                            self.state.focused_tool = Some(next_idx);
                        }
                    }
                } else if self.state.input.cursor_row < self.state.input.line_count() - 1 {
                    // 2. Multi-line cursor movement
                    self.state.input.move_down();
                } else if let Some(entry) = self.state.history.browse_down() {
                    // 3. History browsing
                    self.state.input.set(&entry);
                } else if self.state.input.is_empty() {
                    // 4. Chat scrolling
                    self.state.scroll_offset = self.state.scroll_offset.saturating_add(1);
                    let max_scroll = self
                        .state
                        .total_chat_lines
                        .get()
                        .saturating_sub(self.state.chat_area_height.get());
                    if self.state.scroll_offset >= max_scroll {
                        self.state.scroll_offset = max_scroll;
                        self.state.user_scrolled_up = false;
                    }
                }
            }
            (KeyCode::End, _) => {
                let max_scroll = self
                    .state
                    .total_chat_lines
                    .get()
                    .saturating_sub(self.state.chat_area_height.get());
                self.state.scroll_offset = max_scroll;
                self.state.user_scrolled_up = false;
            }
            (KeyCode::Left, KeyModifiers::NONE) if !self.state.input.is_empty() => {
                self.state.input.move_left();
            }
            (KeyCode::Right, KeyModifiers::NONE) if !self.state.input.is_empty() => {
                self.state.input.move_right();
            }
            (KeyCode::Home, KeyModifiers::NONE) if !self.state.input.is_empty() => {
                self.state.input.cursor_col = 0;
            }
            (KeyCode::Char('e'), KeyModifiers::NONE) if self.state.input.is_empty() => {
                // Toggle expand on last truncated assistant message
                if let Some(idx) = self.state.chat_messages.iter().rposition(|m| {
                    matches!(m, ChatMessage::Assistant { content, .. } if content.lines().count() > 100)
                }) {
                    if self.state.expanded_messages.contains(&idx) {
                        self.state.expanded_messages.remove(&idx);
                    } else {
                        self.state.expanded_messages.insert(idx);
                    }
                }
            }
            (KeyCode::Char(c), _) => {
                self.state.focused_tool = None;
                self.state.history.reset();
                self.state.input.insert_char(c);
                self.state.update_slash_auto();
                self.state.update_file_auto();
            }
            (KeyCode::Backspace, _) => {
                if self.state.input.is_empty() && !self.state.attachments.is_empty() {
                    self.state.attachments.pop();
                } else {
                    self.state.input.backspace();
                    self.state.update_slash_auto();
                    self.state.update_file_auto();
                }
            }
            _ => {}
        }
    }

    async fn handle_approval_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('y') => {
                let should_execute = self.state.agent.approve_current();
                if should_execute {
                    self.start_tool_execution();
                }
            }
            KeyCode::Char('n') => {
                // Capture info before deny mutates state
                let rejection_msg = self.pending_tool_rejection_msg();
                self.state.agent.deny_current();
                // Replace pending placeholder with rejection message
                self.replace_pending_with_rejection(&rejection_msg);
                if matches!(self.state.agent.state, AgentState::Idle) {
                    self.flush_assistant_text();
                }
            }
            KeyCode::Char('a') => {
                self.state.agent.always_allow_current();
                if matches!(self.state.agent.state, AgentState::ExecutingTools) {
                    self.start_tool_execution();
                }
            }
            _ => {}
        }
    }

    /// Build a rejection message for the current pending tool.
    fn pending_tool_rejection_msg(&self) -> String {
        if let AgentState::PendingApproval {
            ref tool_calls,
            current_index,
        } = self.state.agent.state
            && let Some(tc) = tool_calls.get(current_index)
        {
            let detail = crate::tui::approval::format_tool_summary_pub(&tc.name, &tc.arguments);
            return format!("User rejected {detail}");
        }
        "User rejected tool call".to_string()
    }

    /// Replace the last Pending tool placeholder with a system rejection message.
    fn replace_pending_with_rejection(&mut self, msg: &str) {
        // Find the last Pending tool message and replace it
        if let Some(pos) =
            self.state.chat_messages.iter().rposition(
                |m| matches!(m, ChatMessage::Tool(tm) if tm.status == ToolStatus::Pending),
            )
        {
            self.state.chat_messages[pos] = ChatMessage::System {
                content: msg.to_string(),
            };
        }
    }

    /// Handle a key press when the dropdown is in a picker mode.
    async fn handle_picker_key(&mut self, key: KeyCode) {
        use crate::tui::slash_auto::DropdownMode;

        let Some(auto) = &self.state.slash_auto else {
            return;
        };
        if !auto.is_picker() {
            return;
        }

        // Session delete confirmation sub-state
        if let DropdownMode::Sessions { confirm_delete, .. } = &auto.mode
            && confirm_delete.is_some()
        {
            self.handle_session_picker_confirm(key).await;
            return;
        }

        match key {
            KeyCode::Esc => {
                self.state.slash_auto = None;
                self.state.input.clear();
                self.state.roundhouse_model_add = false;
            }
            KeyCode::Up => {
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.selected = auto.selected.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                let max = self.picker_item_count().saturating_sub(1);
                if let Some(auto) = self.state.slash_auto.as_mut()
                    && auto.selected < max
                {
                    auto.selected += 1;
                }
            }
            KeyCode::Enter => {
                self.handle_picker_select().await;
            }
            KeyCode::Tab => {
                self.handle_mcp_tab().await;
            }
            KeyCode::Char('d') => {
                let mode_kind = self
                    .state
                    .slash_auto
                    .as_ref()
                    .map(|a| match &a.mode {
                        DropdownMode::Sessions { .. } => 1,
                        DropdownMode::Skills => 2,
                        _ => 0,
                    })
                    .unwrap_or(0);
                match mode_kind {
                    1 => {
                        // Sessions: request delete confirmation
                        if let Some(auto) = self.state.slash_auto.as_mut()
                            && let DropdownMode::Sessions {
                                results,
                                confirm_delete,
                            } = &mut auto.mode
                        {
                            let filtered = crate::tui::session_picker::filter_search_results(
                                results,
                                &auto.filter,
                            );
                            let can_delete = filtered
                                .get(auto.selected)
                                .map(|f| {
                                    self.state
                                        .current_session_id
                                        .as_ref()
                                        .map(|id| id != &f.session.id)
                                        .unwrap_or(true)
                                })
                                .unwrap_or(false);
                            if can_delete {
                                *confirm_delete = Some(auto.selected);
                            }
                        }
                        return;
                    }
                    2 => {
                        // Skills: toggle disable/enable
                        self.toggle_skill_disabled();
                        return;
                    }
                    _ => {}
                }
                // For other modes, treat 'd' as a filter character
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.filter.push('d');
                    auto.selected = 0;
                }
            }
            KeyCode::Backspace => {
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.filter.pop();
                    auto.selected = 0;
                }
            }
            KeyCode::Delete => {
                // Skills mode: delete user skill (not built-in)
                let is_skills = self
                    .state
                    .slash_auto
                    .as_ref()
                    .map(|a| matches!(a.mode, DropdownMode::Skills))
                    .unwrap_or(false);
                if is_skills {
                    self.delete_user_skill();
                }
            }
            KeyCode::Char(c) => {
                // 'x' in Skills mode is an alias for Delete
                let is_skills = self
                    .state
                    .slash_auto
                    .as_ref()
                    .map(|a| matches!(a.mode, DropdownMode::Skills))
                    .unwrap_or(false);
                if c == 'x' && is_skills {
                    self.delete_user_skill();
                    return;
                }
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.filter.push(c);
                    auto.selected = 0;
                }
            }
            _ => {}
        }
    }

    /// Handle confirm/cancel for session delete.
    async fn handle_session_picker_confirm(&mut self, key: KeyCode) {
        use crate::tui::slash_auto::DropdownMode;

        match key {
            KeyCode::Char('y') => {
                let delete_id = if let Some(auto) = &self.state.slash_auto {
                    if let DropdownMode::Sessions {
                        results,
                        confirm_delete,
                    } = &auto.mode
                    {
                        confirm_delete.and_then(|idx| {
                            let filtered = crate::tui::session_picker::filter_search_results(
                                results,
                                &auto.filter,
                            );
                            filtered.get(idx).map(|f| f.session.id.clone())
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(id) = delete_id {
                    if let Err(e) = self.state.sessions.delete(&id) {
                        self.state.chat_messages.push(ChatMessage::Error {
                            content: format!("Failed to delete session: {e}"),
                        });
                    }
                    if let Some(auto) = self.state.slash_auto.as_mut()
                        && let DropdownMode::Sessions {
                            results,
                            confirm_delete,
                        } = &mut auto.mode
                    {
                        results.retain(|r| r.session.id != id);
                        *confirm_delete = None;
                        let filtered = crate::tui::session_picker::filter_search_results(
                            results,
                            &auto.filter,
                        );
                        let max = filtered.len().saturating_sub(1);
                        if auto.selected > max {
                            auto.selected = max;
                        }
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                if let Some(auto) = self.state.slash_auto.as_mut()
                    && let DropdownMode::Sessions { confirm_delete, .. } = &mut auto.mode
                {
                    *confirm_delete = None;
                }
            }
            _ => {}
        }
    }

    /// Handle Enter in picker mode — select the current item.
    async fn handle_picker_select(&mut self) {
        use crate::tui::slash_auto::DropdownMode;

        let Some(auto) = &self.state.slash_auto else {
            return;
        };
        match &auto.mode {
            DropdownMode::Sessions { results, .. } => {
                let filtered =
                    crate::tui::session_picker::filter_search_results(results, &auto.filter);
                let selected_id = filtered.get(auto.selected).map(|f| f.session.id.clone());
                self.state.slash_auto = None;
                self.state.input.clear();
                if let Some(id) = selected_id {
                    self.state.chat_messages.clear();
                    self.state.scroll_offset = 0;
                    self.state.user_scrolled_up = false;
                    self.state.modified_files.clear();
                    self.state.file_baselines.clear();
                    self.state.focused_tool = None;
                    self.state.agent.cancel();
                    self.state.agent.conversation.messages.clear();
                    self.state.agent.turn_count = 0;
                    self.restore_session(&id);
                }
            }
            DropdownMode::Models { models, recent, .. } => {
                let selection = crate::tui::slash_auto::resolve_model_selection(
                    models,
                    recent,
                    &auto.filter,
                    auto.selected,
                );
                // Look up capabilities before clearing slash_auto (borrows models/recent)
                let (supports_tools, supports_vision) = selection
                    .as_ref()
                    .and_then(|(_, model_id)| {
                        models
                            .iter()
                            .chain(recent.iter())
                            .find(|(_, m)| m.id == *model_id)
                            .map(|(_, m)| (m.supports_tools, m.supports_vision))
                    })
                    .unwrap_or((true, false));
                // Build display name for roundhouse before clearing slash_auto
                let display_for_roundhouse = selection.as_ref().map(|(provider, model_id)| {
                    let display = crate::provider::catalog::by_id(provider)
                        .map(|e| e.display_name.to_string())
                        .unwrap_or_else(|| provider.clone());
                    (provider.clone(), display, model_id.clone())
                });
                self.state.slash_auto = None;
                self.state.input.clear();
                if self.state.roundhouse_model_add {
                    self.state.roundhouse_model_add = false;
                    if let Some((provider_id, display_name, model_id)) = display_for_roundhouse
                        && let Some(DialogKind::RoundhouseProviderPicker(picker)) =
                            self.state.dialog_stack.top_mut()
                    {
                        picker
                            .secondaries
                            .push(crate::tui::dialog::RoundhouseSecondary {
                                provider_id,
                                display_name,
                                model: model_id,
                            });
                    }
                } else if let Some((provider, model_id)) = selection {
                    self.state.model_supports_tools = supports_tools;
                    self.state.model_supports_vision = supports_vision;
                    self.select_model(&provider, &model_id);
                }
            }
            DropdownMode::Providers => {
                use crate::provider::catalog;
                let needle = auto.filter.to_lowercase();
                let filtered: Vec<&catalog::ProviderEntry> = catalog::CATALOG
                    .iter()
                    .filter(|e| {
                        needle.is_empty()
                            || e.display_name.to_lowercase().contains(&needle)
                            || e.id.to_lowercase().contains(&needle)
                    })
                    .collect();
                let selected_id = filtered.get(auto.selected).map(|e| e.id.to_string());
                if let Some(provider_id) = selected_id {
                    self.state.slash_auto = None;
                    self.state.input.clear();

                    // Local providers use address+probe flow instead of API key
                    if crate::provider::catalog::by_id(&provider_id)
                        .map(|p| p.is_local())
                        .unwrap_or(false)
                    {
                        let entry = crate::provider::catalog::by_id(&provider_id).unwrap();
                        let server_type = match provider_id.as_str() {
                            "ollama" => crate::provider::local::LocalServerType::Ollama,
                            "lmstudio" => crate::provider::local::LocalServerType::LmStudio,
                            "llamacpp" => crate::provider::local::LocalServerType::LlamaCpp,
                            _ => crate::provider::local::LocalServerType::Custom,
                        };
                        self.state
                            .dialog_stack
                            .push(DialogKind::LocalProviderConnect(
                                crate::tui::dialog::LocalProviderConnectState {
                                    provider_id: provider_id.clone(),
                                    provider_name: entry.display_name.to_string(),
                                    address: server_type.default_address().to_string(),
                                    models: vec![],
                                    selected_model: 0,
                                    phase: crate::tui::dialog::LocalConnectPhase::Address,
                                    error: None,
                                    probe_rx: None,
                                },
                            ));
                    } else {
                        // Always show key input so user can add, update, or clear their key
                        let has_existing = self.state.config.keys.get(&provider_id).is_some();
                        self.state
                            .dialog_stack
                            .push(DialogKind::ApiKeyInput(KeyInputState::new(
                                provider_id,
                                has_existing,
                            )));
                    }
                }
            }
            DropdownMode::McpServers { servers } => {
                let selected = auto.selected;
                if selected == 0 {
                    // "Add new server"
                    self.state.slash_auto = None;
                    self.state.input.clear();
                    self.state.dialog_stack.push(DialogKind::McpServerInput(
                        crate::tui::mcp_input::McpServerInputState::new(),
                    ));
                } else {
                    // Selected an existing server
                    let idx = selected - 1;
                    if let Some((
                        name,
                        _status,
                        _count,
                        _is_connected,
                        is_preset,
                        _is_enabled,
                        _desc,
                    )) = servers.get(idx).cloned()
                    {
                        self.state.slash_auto = Some(
                            crate::tui::slash_auto::SlashAutoState::with_mcp_server_actions(
                                name, is_preset,
                            ),
                        );
                    }
                }
            }
            DropdownMode::McpServerActions {
                server_name,
                is_preset,
            } => {
                let name = server_name.clone();
                let preset = *is_preset;
                let selected = auto.selected;
                self.state.slash_auto = None;
                self.state.input.clear();

                match selected {
                    0 => {
                        // Restart — disconnect + background reconnect
                        self.state.mcp_manager.disconnect_server(&name).await;
                        let tx = self.state.mcp_connect_tx.clone();
                        let _ = self.state.mcp_manager.connect_server_background(&name, tx);
                        self.state.chat_messages.push(ChatMessage::System {
                            content: format!("MCP: Restarting \"{name}\"..."),
                        });
                    }
                    1 => {
                        // Remove
                        self.state.mcp_manager.disconnect_server(&name).await;
                        self.state.mcp_manager.servers.remove(&name);
                        if preset {
                            // Save removed: true so preset doesn't reappear
                            if let Some(preset_info) = crate::mcp::find_preset(&name) {
                                let mut config = preset_info.config.clone();
                                config.removed = true;
                                crate::config::save_mcp_server_toggle(&name, &config);
                            }
                        } else {
                            crate::config::remove_mcp_server(&name);
                        }
                        self.state.chat_messages.push(ChatMessage::System {
                            content: format!("MCP: Removed \"{name}\""),
                        });
                    }
                    _ => {}
                }
            }
            DropdownMode::Settings { .. } => {
                // Grab the selected index, then take mutable access to toggle
                let selected = auto.selected;
                let auto_mut = self.state.slash_auto.as_mut().unwrap();
                if let DropdownMode::Settings { ref mut items } = auto_mut.mode
                    && let Some(item) = items.get_mut(selected)
                {
                    match item.kind {
                        crate::tui::slash_auto::SettingsKind::Toggle => {
                            let new_val = if item.value == "on" { "off" } else { "on" };
                            item.value = new_val.to_string();
                            let enabled = new_val == "on";

                            match item.key.as_str() {
                                "memory.enabled" => {
                                    self.state.memory.set_enabled(enabled);
                                    let mem_config = self
                                        .state
                                        .config
                                        .memory
                                        .get_or_insert_with(Default::default);
                                    mem_config.enabled = enabled;
                                }
                                "memory.auto_extract" => {
                                    let mem_config = self
                                        .state
                                        .config
                                        .memory
                                        .get_or_insert_with(Default::default);
                                    mem_config.auto_extract = enabled;
                                }
                                _ => {}
                            }
                        }
                        crate::tui::slash_auto::SettingsKind::Choice(ref choices) => {
                            // Cycle to next choice
                            if let Some(idx) = choices.iter().position(|c| c == &item.value) {
                                let next = (idx + 1) % choices.len();
                                item.value = choices[next].clone();
                            } else if let Some(first) = choices.first() {
                                item.value = first.clone();
                            }

                            match item.key.as_str() {
                                "theme" => {
                                    let variant = crate::tui::theme::ThemeVariant::ALL
                                        .iter()
                                        .find(|v| v.label() == item.value)
                                        .copied()
                                        .unwrap_or_default();
                                    crate::tui::theme::set_active_variant(variant);
                                    let mut prefs = crate::config::prefs::TuiPrefs::load();
                                    prefs.theme = variant;
                                    prefs.save();
                                }
                                "behavior.max_session_cost" => {
                                    let clean = item.value.trim_end_matches(" (custom)");
                                    let new_max = if clean == "off" {
                                        None
                                    } else {
                                        clean.trim_start_matches('$').parse::<f64>().ok()
                                    };
                                    self.state
                                        .config
                                        .behavior
                                        .get_or_insert_with(Default::default)
                                        .max_session_cost = new_max;
                                    crate::config::save_behavior_max_session_cost(new_max);
                                }
                                "migrate" => {
                                    if item.value != "(none)" {
                                        let platform_label = item.value.clone();
                                        let platform = crate::migrate::SourcePlatform::all()
                                            .into_iter()
                                            .find(|p| p.label() == platform_label);
                                        if let Some(platform) = platform {
                                            let checklist =
                                                crate::tui::dialog::build_migration_checklist(
                                                    platform,
                                                );
                                            if checklist.items.is_empty() {
                                                self.state.chat_messages.push(
                                                    ChatMessage::System {
                                                        content: format!(
                                                            "No importable items found for {}.",
                                                            platform_label
                                                        ),
                                                    },
                                                );
                                            } else {
                                                self.state.dialog_stack.push(
                                                    DialogKind::MigrationChecklist(checklist),
                                                );
                                            }
                                        }
                                        item.value = "(none)".to_string();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // Don't close the picker — let user toggle multiple settings
            }
            DropdownMode::Skills => {
                let filtered =
                    crate::tui::slash_auto::filter_skills(&self.state.skills, &auto.filter);
                let selected_name = filtered
                    .get(auto.selected)
                    .and_then(|&idx| self.state.skills.get(idx))
                    .map(|s| s.name.clone());
                self.state.slash_auto = None;
                self.state.input.clear();
                if let Some(name) = selected_name {
                    // Populate input with /<skillname> so user can add args
                    self.state.input.set(&format!("/{name} "));
                }
            }
            DropdownMode::Checkpoints { items } => {
                if let Some((id, preview, _, _)) = items.get(auto.selected) {
                    let checkpoint_id = *id;
                    let preview = preview.clone();
                    self.state.slash_auto = None;
                    self.state.input.clear();
                    match self.state.checkpoints.rewind(checkpoint_id) {
                        Ok(summary) => {
                            // Recompute modified_files from baselines (files are now restored on disk)
                            self.recompute_modified_files();
                            self.state.chat_messages.push(ChatMessage::System {
                                content: format!("Rewound to before \"{preview}\". {summary}"),
                            });
                        }
                        Err(e) => {
                            self.state.chat_messages.push(ChatMessage::System {
                                content: format!("Rewind failed: {e}"),
                            });
                        }
                    }
                }
            }
            DropdownMode::Commands => {} // Should not happen — Commands mode uses normal flow
        }
    }

    /// Handle scroll wheel in menus/dropdowns. Returns `true` if a menu consumed the event.
    fn handle_menu_scroll(&mut self, up: bool) -> bool {
        // 1. Command palette
        if let Some(DialogKind::CommandPalette(palette)) = self.state.dialog_stack.top_mut() {
            if up {
                palette.selected = palette.selected.saturating_sub(1);
            } else {
                // Need count — drop mutable borrow, get count, re-borrow
                let selected = palette.selected;
                let count = match self.state.dialog_stack.top() {
                    Some(DialogKind::CommandPalette(p)) => {
                        crate::tui::command_palette::filtered_count(p, &self.state)
                    }
                    _ => 0,
                };
                if let Some(DialogKind::CommandPalette(p)) = self.state.dialog_stack.top_mut()
                    && selected + 1 < count
                {
                    p.selected += 1;
                }
            }
            return true;
        }

        // 2. Picker (sessions, models, MCP, providers)
        if self
            .state
            .slash_auto
            .as_ref()
            .map(|a| a.is_picker())
            .unwrap_or(false)
        {
            if up {
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.selected = auto.selected.saturating_sub(1);
                }
            } else {
                let max = self.picker_item_count().saturating_sub(1);
                if let Some(auto) = self.state.slash_auto.as_mut()
                    && auto.selected < max
                {
                    auto.selected += 1;
                }
            }
            return true;
        }

        // 3. File autocomplete
        if let Some(ref mut auto) = self.state.file_auto {
            if up {
                auto.select_up();
            } else {
                auto.select_down();
            }
            return true;
        }

        // 4. Slash autocomplete
        if self.state.slash_auto.is_some() {
            if up {
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.selected = auto.selected.saturating_sub(1);
                }
            } else {
                let input_text = self.state.input.content();
                let prefix = crate::tui::slash_auto::slash_prefix(&input_text).unwrap_or("");
                let count = crate::tui::slash_auto::total_filtered(
                    prefix,
                    &self.state.commands,
                    &self.state.skills,
                );
                if let Some(auto) = self.state.slash_auto.as_mut()
                    && auto.selected + 1 < count
                {
                    auto.selected += 1;
                }
            }
            return true;
        }

        false
    }

    /// Count of selectable items in current picker mode.
    fn picker_item_count(&self) -> usize {
        use crate::tui::slash_auto::DropdownMode;

        let Some(auto) = &self.state.slash_auto else {
            return 0;
        };
        match &auto.mode {
            DropdownMode::Sessions { results, .. } => {
                crate::tui::session_picker::filter_search_results(results, &auto.filter).len()
            }
            DropdownMode::Models { models, recent, .. } => {
                crate::tui::slash_auto::filtered_model_count(models, recent, &auto.filter)
            }
            DropdownMode::Providers => {
                use crate::provider::catalog;
                let needle = auto.filter.to_lowercase();
                catalog::CATALOG
                    .iter()
                    .filter(|e| {
                        needle.is_empty()
                            || e.display_name.to_lowercase().contains(&needle)
                            || e.id.to_lowercase().contains(&needle)
                    })
                    .count()
            }
            DropdownMode::McpServers { servers } => {
                servers.len() + 1 // +1 for "Add new server"
            }
            DropdownMode::McpServerActions { .. } => 2, // Restart, Remove
            DropdownMode::Settings { items } => items.len(),
            DropdownMode::Skills => {
                crate::tui::slash_auto::filtered_skill_count(&self.state.skills, &auto.filter)
            }
            DropdownMode::Checkpoints { items } => items.len(),
            DropdownMode::Commands => 0,
        }
    }

    /// Connect to a provider (resolve + activate + save as last-used).
    async fn connect_provider(&mut self, provider_id: &str) {
        self.state.providers = ProviderRegistry::new(&self.state.config);
        match self.state.providers.get_provider(Some(provider_id), None) {
            Ok(p) => {
                self.state.active_provider_name = p.name().to_string();
                self.state.active_model_name = p.model().to_string();

                // If the static table doesn't know this model, fetch from provider API
                if crate::provider::models_dev::context_window(&self.state.active_model_name)
                    .is_none()
                    && let Ok(model_list) = p.list_models().await
                {
                    let cw_entries: Vec<(String, Option<u32>)> = model_list
                        .iter()
                        .map(|m| (m.id.clone(), m.context_window))
                        .collect();
                    crate::provider::models_dev::cache_from_model_list(&cw_entries);
                }

                // Update context window for compaction and sidebar display
                self.state.agent.context_window =
                    crate::provider::models_dev::context_window_or_default(
                        &self.state.active_model_name,
                    );

                let cw_display =
                    crate::provider::models_dev::context_window(&self.state.active_model_name)
                        .map(|cw| format!(" ({}k context)", cw / 1000))
                        .unwrap_or_default();
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Connected to {}/{}{}",
                        self.state.active_provider_name, self.state.active_model_name, cw_display,
                    ),
                });
                self.provider = Some(p);

                // Persist last-used provider so we reconnect on restart
                let mut prefs = crate::config::prefs::TuiPrefs::load();
                prefs.last_provider = Some(provider_id.to_string());
                prefs.last_model = None; // use provider's default
                prefs.save();
            }
            Err(_) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "API key saved for {provider_id}. Provider not yet supported \u{2014} coming soon."
                    ),
                });
            }
        }
    }

    fn handle_file_browser_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.state.dialog_stack.pop();
            }
            KeyCode::Up => {
                if let Some(DialogKind::FileBrowser(state)) = self.state.dialog_stack.top_mut() {
                    state.select_up();
                }
            }
            KeyCode::Down => {
                if let Some(DialogKind::FileBrowser(state)) = self.state.dialog_stack.top_mut() {
                    state.select_down();
                }
            }
            KeyCode::Enter => {
                // Determine what action to take based on the selected entry
                enum BrowseAction {
                    NavigateDir(std::path::PathBuf),
                    AttachImage(std::path::PathBuf),
                    InsertRef(String),
                    Close,
                }

                let action =
                    if let Some(DialogKind::FileBrowser(state)) = self.state.dialog_stack.top() {
                        if let Some(entry) = state.selected_entry() {
                            if entry.is_dir {
                                BrowseAction::NavigateDir(entry.path.clone())
                            } else if crate::attachment::is_image_path(&entry.path) {
                                BrowseAction::AttachImage(entry.path.clone())
                            } else {
                                // Non-image file: insert as @path reference (relative if possible)
                                let path_str = std::env::current_dir()
                                    .ok()
                                    .and_then(|cwd| {
                                        entry
                                            .path
                                            .strip_prefix(&cwd)
                                            .ok()
                                            .map(|rel| rel.to_string_lossy().to_string())
                                    })
                                    .unwrap_or_else(|| entry.path.to_string_lossy().to_string());
                                BrowseAction::InsertRef(path_str)
                            }
                        } else {
                            BrowseAction::Close
                        }
                    } else {
                        BrowseAction::Close
                    };

                match action {
                    BrowseAction::NavigateDir(dir) => {
                        if let Some(DialogKind::FileBrowser(state)) =
                            self.state.dialog_stack.top_mut()
                        {
                            state.navigate_into(dir);
                        }
                    }
                    BrowseAction::AttachImage(path) => {
                        match crate::attachment::read_image_attachment(&path) {
                            Ok(att) => self.state.attachments.push(att),
                            Err(e) => {
                                self.state.chat_messages.push(ChatMessage::Error {
                                    content: format!("Failed to attach: {e}"),
                                });
                            }
                        }
                        self.state.dialog_stack.pop();
                    }
                    BrowseAction::InsertRef(path_str) => {
                        let content = self.state.input.content();
                        let separator = if content.is_empty() || content.ends_with(' ') {
                            ""
                        } else {
                            " "
                        };
                        self.state
                            .input
                            .push_str(&format!("{separator}@{path_str} "));
                        self.state.dialog_stack.pop();
                    }
                    BrowseAction::Close => {
                        self.state.dialog_stack.pop();
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(DialogKind::FileBrowser(state)) = self.state.dialog_stack.top_mut() {
                    if state.filter.is_empty() {
                        // Navigate up
                        if let Some(parent) = state.cwd.parent().map(|p| p.to_path_buf()) {
                            state.navigate_into(parent);
                        }
                    } else {
                        state.pop_filter();
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(DialogKind::FileBrowser(state)) = self.state.dialog_stack.top_mut() {
                    state.push_filter(c);
                }
            }
            _ => {}
        }
    }

    async fn handle_roundhouse_picker_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // If the model dropdown is open (from pressing 'a'), route keys there first
        if self
            .state
            .slash_auto
            .as_ref()
            .map(|a| a.is_picker())
            .unwrap_or(false)
        {
            self.handle_picker_key(key).await;
            return;
        }

        match key {
            KeyCode::Esc => {
                self.state.roundhouse_session = None;
                self.state.roundhouse_model_add = false;
                self.state.dialog_stack.pop();
            }
            KeyCode::Up if modifiers == KeyModifiers::NONE => {
                if let Some(DialogKind::RoundhouseProviderPicker(picker)) =
                    self.state.dialog_stack.top_mut()
                    && picker.selected > 0
                {
                    picker.selected -= 1;
                }
            }
            KeyCode::Down if modifiers == KeyModifiers::NONE => {
                if let Some(DialogKind::RoundhouseProviderPicker(picker)) =
                    self.state.dialog_stack.top_mut()
                {
                    let count = picker.secondaries.len();
                    if count > 0 && picker.selected + 1 < count {
                        picker.selected += 1;
                    }
                }
            }
            KeyCode::Char('a') => {
                // Open model dropdown — when a model is selected, add it as a secondary
                self.state.roundhouse_model_add = true;
                self.open_model_dropdown().await;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(DialogKind::RoundhouseProviderPicker(picker)) =
                    self.state.dialog_stack.top_mut()
                    && !picker.secondaries.is_empty()
                {
                    picker.secondaries.remove(picker.selected);
                    if picker.selected > 0 && picker.selected >= picker.secondaries.len() {
                        picker.selected = picker.secondaries.len().saturating_sub(1);
                    }
                }
            }
            KeyCode::Enter => {
                // Collect secondaries before mutating
                let secondaries: Vec<(String, String)> =
                    if let Some(DialogKind::RoundhouseProviderPicker(picker)) =
                        self.state.dialog_stack.top()
                    {
                        picker
                            .secondaries
                            .iter()
                            .map(|s| (s.provider_id.clone(), s.model.clone()))
                            .collect()
                    } else {
                        Vec::new()
                    };

                if !secondaries.is_empty() {
                    if let Some(session) = &mut self.state.roundhouse_session {
                        for (id, model) in &secondaries {
                            session.add_secondary(id.clone(), model.clone());
                        }
                        session.phase = crate::roundhouse::types::RoundhousePhase::AwaitingPrompt;
                    }
                    self.state.dialog_stack.pop();
                    self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!(
                            "Roundhouse: {} secondary model(s) selected. Enter your planning prompt.",
                            secondaries.len()
                        ),
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_circuits_list_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        match key {
            KeyCode::Esc => {
                self.state.dialog_stack.pop();
            }
            KeyCode::Up if modifiers == KeyModifiers::NONE => {
                if let Some(DialogKind::CircuitsList(list_state)) =
                    self.state.dialog_stack.top_mut()
                    && list_state.selected > 0
                {
                    list_state.selected -= 1;
                }
            }
            KeyCode::Down if modifiers == KeyModifiers::NONE => {
                let count = self.state.circuit_manager.active_count();
                if let Some(DialogKind::CircuitsList(list_state)) =
                    self.state.dialog_stack.top_mut()
                    && list_state.selected + 1 < count
                {
                    list_state.selected += 1;
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let selected = if let Some(DialogKind::CircuitsList(list_state)) =
                    self.state.dialog_stack.top()
                {
                    list_state.selected
                } else {
                    return;
                };
                let circuit_id = self
                    .state
                    .circuit_manager
                    .circuits
                    .get(selected)
                    .map(|h| h.circuit.id.clone());
                if let Some(id) = circuit_id {
                    self.state.circuit_manager.stop_circuit(&id);
                    if let Some(DialogKind::CircuitsList(list_state)) =
                        self.state.dialog_stack.top_mut()
                        && list_state.selected > 0
                    {
                        list_state.selected -= 1;
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_migration_checklist_key(&mut self, key: KeyCode) {
        use crate::tui::dialog::MigrationPhase;

        let checklist = match self.state.dialog_stack.top_mut() {
            Some(DialogKind::MigrationChecklist(c)) => c,
            _ => return,
        };

        match &checklist.phase {
            MigrationPhase::Checklist => match key {
                KeyCode::Up => {
                    if checklist.selected > 0 {
                        checklist.selected -= 1;
                    }
                }
                KeyCode::Down => {
                    if !checklist.items.is_empty() && checklist.selected < checklist.items.len() - 1
                    {
                        checklist.selected += 1;
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(item) = checklist.items.get_mut(checklist.selected) {
                        item.toggled = !item.toggled;
                    }
                }
                KeyCode::Enter => {
                    let any_toggled = checklist.items.iter().any(|i| i.toggled);
                    if any_toggled {
                        checklist.phase = MigrationPhase::Preview;
                    }
                }
                KeyCode::Esc => {
                    self.state.dialog_stack.pop();
                }
                _ => {}
            },
            MigrationPhase::Preview => match key {
                KeyCode::Enter => {
                    let result = crate::migrate::converter::apply_migration(&checklist.items);
                    checklist.phase = MigrationPhase::Done(result.format_summary());
                }
                KeyCode::Esc => {
                    checklist.phase = MigrationPhase::Checklist;
                }
                _ => {}
            },
            MigrationPhase::Done(_) => {
                self.state.dialog_stack.pop();
            }
            MigrationPhase::Applying => {}
        }
    }

    async fn handle_key_input_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                // Pop ApiKeyInput to reveal ProviderPicker underneath
                self.state.dialog_stack.pop();
            }
            KeyCode::Enter => {
                // Extract data from dialog state before mutating
                let (provider_id, api_key, has_existing) = match self.state.dialog_stack.top() {
                    Some(DialogKind::ApiKeyInput(state)) => (
                        state.provider_id.clone(),
                        state.input.clone(),
                        state.has_existing,
                    ),
                    _ => return,
                };

                if api_key.is_empty() && !has_existing {
                    // No key typed and none stored — nothing to do
                    return;
                }

                if api_key.is_empty() {
                    // Empty submit with existing key → clear it
                    self.state.config.keys.clear(&provider_id);
                    self.state.auth_store.remove(&provider_id);
                    if let Err(e) = self.state.auth_store.save() {
                        self.state.chat_messages.push(ChatMessage::Error {
                            content: format!("Failed to save: {e}"),
                        });
                    }
                    self.provider = None;
                    self.state.active_provider_name = String::new();
                    self.state.active_model_name = String::new();
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!("API key cleared for {provider_id}."),
                    });
                } else {
                    // Save new key
                    self.state.config.keys.set(&provider_id, api_key.clone());
                    self.state.auth_store.set(&provider_id, &api_key);
                    if let Err(e) = self.state.auth_store.save() {
                        self.state.chat_messages.push(ChatMessage::Error {
                            content: format!("Failed to save API key: {e}"),
                        });
                    }
                    self.connect_provider(&provider_id).await;
                }

                // Close all overlays — back to base screen
                self.state.dialog_stack.clear();
            }
            _ => {
                if let Some(DialogKind::ApiKeyInput(state)) = self.state.dialog_stack.top_mut() {
                    match key {
                        KeyCode::Backspace => {
                            state.input.pop();
                        }
                        KeyCode::Char(c) => {
                            state.input.push(c);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn handle_local_connect_key(&mut self, key: KeyCode) {
        use crate::tui::dialog::LocalConnectPhase;

        let phase = match self.state.dialog_stack.top() {
            Some(DialogKind::LocalProviderConnect(state)) => match state.phase {
                LocalConnectPhase::Address => 0u8,
                LocalConnectPhase::Probing => 1,
                LocalConnectPhase::ModelSelect => 2,
            },
            _ => return,
        };

        match phase {
            // Address phase
            0 => match key {
                KeyCode::Esc => {
                    self.state.dialog_stack.pop();
                }
                KeyCode::Enter => {
                    // Spawn async probe, transition to Probing
                    let address = match self.state.dialog_stack.top() {
                        Some(DialogKind::LocalProviderConnect(s)) => s.address.clone(),
                        _ => return,
                    };
                    if address.is_empty() {
                        if let Some(DialogKind::LocalProviderConnect(s)) =
                            self.state.dialog_stack.top_mut()
                        {
                            s.error = Some("Address cannot be empty".to_string());
                        }
                        return;
                    }
                    let provider_id = match self.state.dialog_stack.top() {
                        Some(DialogKind::LocalProviderConnect(s)) => s.provider_id.clone(),
                        _ => return,
                    };
                    let server_type = match provider_id.as_str() {
                        "ollama" => crate::provider::local::LocalServerType::Ollama,
                        "lmstudio" => crate::provider::local::LocalServerType::LmStudio,
                        "llamacpp" => crate::provider::local::LocalServerType::LlamaCpp,
                        _ => crate::provider::local::LocalServerType::Custom,
                    };
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let addr = address.clone();
                    tokio::spawn(async move {
                        match crate::provider::local::probe_server(&addr, &server_type).await {
                            Some(models) => {
                                let _ = tx.send(Ok(models));
                            }
                            None => {
                                let _ = tx.send(Err(format!("Could not connect to {addr}")));
                            }
                        }
                    });
                    if let Some(DialogKind::LocalProviderConnect(s)) =
                        self.state.dialog_stack.top_mut()
                    {
                        s.phase = LocalConnectPhase::Probing;
                        s.error = None;
                        s.probe_rx = Some(rx);
                    }
                }
                _ => {
                    if let Some(DialogKind::LocalProviderConnect(s)) =
                        self.state.dialog_stack.top_mut()
                    {
                        match key {
                            KeyCode::Backspace => {
                                s.address.pop();
                            }
                            KeyCode::Char(c) => {
                                s.address.push(c);
                            }
                            _ => {}
                        }
                    }
                }
            },
            // Probing phase
            1 => {
                if key == KeyCode::Esc
                    && let Some(DialogKind::LocalProviderConnect(s)) =
                        self.state.dialog_stack.top_mut()
                {
                    s.phase = LocalConnectPhase::Address;
                    s.probe_rx = None;
                }
            }
            // ModelSelect phase
            2 => match key {
                KeyCode::Esc => {
                    if let Some(DialogKind::LocalProviderConnect(s)) =
                        self.state.dialog_stack.top_mut()
                    {
                        s.phase = LocalConnectPhase::Address;
                        s.models.clear();
                        s.selected_model = 0;
                    }
                }
                KeyCode::Up => {
                    if let Some(DialogKind::LocalProviderConnect(s)) =
                        self.state.dialog_stack.top_mut()
                        && s.selected_model > 0
                    {
                        s.selected_model -= 1;
                    }
                }
                KeyCode::Down => {
                    if let Some(DialogKind::LocalProviderConnect(s)) =
                        self.state.dialog_stack.top_mut()
                        && s.selected_model + 1 < s.models.len()
                    {
                        s.selected_model += 1;
                    }
                }
                KeyCode::Enter => {
                    // Extract data before mutating
                    let (provider_id, address, model_name, provider_name) =
                        match self.state.dialog_stack.top() {
                            Some(DialogKind::LocalProviderConnect(s)) => {
                                let model =
                                    s.models.get(s.selected_model).cloned().unwrap_or_default();
                                (
                                    s.provider_id.clone(),
                                    s.address.clone(),
                                    model,
                                    s.provider_name.clone(),
                                )
                            }
                            _ => return,
                        };

                    if model_name.is_empty() {
                        return;
                    }

                    // Save local provider config
                    let local_config = crate::config::schema::LocalProviderConfig {
                        provider_type: provider_id.clone(),
                        address: address.clone(),
                        model: Some(model_name.clone()),
                        display_name: Some(provider_name.clone()),
                    };
                    crate::config::save_local_provider(&provider_id, &local_config);

                    // Update in-memory config so connect_provider can find it
                    self.state
                        .config
                        .local_providers
                        .insert(provider_id.clone(), local_config);

                    // Connect provider
                    self.connect_provider(&provider_id).await;

                    // Close all overlays
                    self.state.dialog_stack.clear();
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn handle_mcp_input_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.state.dialog_stack.pop();
            }
            KeyCode::Tab => {
                if let Some(DialogKind::McpServerInput(state)) = self.state.dialog_stack.top_mut() {
                    state.focused = state.focused.next();
                }
            }
            KeyCode::BackTab => {
                if let Some(DialogKind::McpServerInput(state)) = self.state.dialog_stack.top_mut() {
                    state.focused = state.focused.prev();
                }
            }
            KeyCode::Enter => {
                self.handle_mcp_input_submit();
            }
            KeyCode::Backspace => {
                if let Some(DialogKind::McpServerInput(state)) = self.state.dialog_stack.top_mut() {
                    state.focused_input_mut().pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(DialogKind::McpServerInput(state)) = self.state.dialog_stack.top_mut() {
                    state.focused_input_mut().push(c);
                }
            }
            _ => {}
        }
    }

    fn handle_mcp_input_submit(&mut self) {
        let (name, command, args_str) = match self.state.dialog_stack.top() {
            Some(DialogKind::McpServerInput(state)) => (
                state.name.clone(),
                state.command.clone(),
                state.args.clone(),
            ),
            _ => return,
        };

        // Validate
        let name = name.trim().to_string();
        let command = command.trim().to_string();
        if name.is_empty() || command.is_empty() {
            self.state.chat_messages.push(ChatMessage::Error {
                content: "Name and command are required.".to_string(),
            });
            return;
        }

        if self.state.mcp_manager.servers.contains_key(&name) {
            self.state.chat_messages.push(ChatMessage::Error {
                content: format!("MCP server \"{name}\" already exists."),
            });
            return;
        }

        // Parse args
        let args: Vec<String> = if args_str.trim().is_empty() {
            Vec::new()
        } else {
            args_str.split_whitespace().map(|s| s.to_string()).collect()
        };

        // Create config
        let server_config = crate::config::schema::McpServerConfig {
            command: command.clone(),
            args,
            env: std::collections::HashMap::new(),
            disabled: false,
            removed: false,
        };

        // Add to manager
        self.state.mcp_manager.servers.insert(
            name.clone(),
            crate::mcp::McpServer {
                name: name.clone(),
                config: server_config,
                status: crate::mcp::ServerStatus::Disconnected,
                is_preset: false,
                tools: Vec::new(),
                service: None,
            },
        );

        self.state.dialog_stack.pop();
        self.state.chat_messages.push(ChatMessage::System {
            content: format!("MCP: Added server \"{name}\". Use /mcp to connect."),
        });
    }

    async fn handle_command_palette_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.state.dialog_stack.pop();
            }
            KeyCode::Enter => {
                // Look up selected command and execute it
                let cmd_id = {
                    match self.state.dialog_stack.top() {
                        Some(DialogKind::CommandPalette(palette)) => {
                            crate::tui::command_palette::selected_command_id(palette, &self.state)
                        }
                        _ => None,
                    }
                };
                // Pop palette first, then execute the command
                self.state.dialog_stack.pop();
                if let Some(id) = cmd_id
                    && let Some(cmd) = self.state.commands.find_by_id(id)
                    && (cmd.available)(&self.state)
                {
                    let action = (cmd.execute)(&mut self.state);
                    self.process_action(action).await;
                }
            }
            KeyCode::Up => {
                if let Some(DialogKind::CommandPalette(palette)) = self.state.dialog_stack.top_mut()
                {
                    palette.selected = palette.selected.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                // Compute count first with immutable borrow, then mutate
                let count = match self.state.dialog_stack.top() {
                    Some(DialogKind::CommandPalette(palette)) => {
                        crate::tui::command_palette::filtered_count(palette, &self.state)
                    }
                    _ => 0,
                };
                if let Some(DialogKind::CommandPalette(palette)) = self.state.dialog_stack.top_mut()
                    && palette.selected + 1 < count
                {
                    palette.selected += 1;
                }
            }
            KeyCode::Backspace => {
                if let Some(DialogKind::CommandPalette(palette)) = self.state.dialog_stack.top_mut()
                {
                    palette.filter.pop();
                    palette.selected = 0;
                }
            }
            KeyCode::Char(c) => {
                if let Some(DialogKind::CommandPalette(palette)) = self.state.dialog_stack.top_mut()
                {
                    palette.filter.push(c);
                    palette.selected = 0;
                }
            }
            _ => {}
        }
    }

    fn handle_paste(&mut self, text: &str) {
        const PASTE_THRESHOLD_LINES: usize = 20;
        const PASTE_THRESHOLD_CHARS: usize = 2000;

        match self.state.dialog_stack.top_mut() {
            Some(DialogKind::ApiKeyInput(state)) => {
                // Strip newlines — API keys are single-line
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                state.input.push_str(&clean);
            }
            Some(DialogKind::McpServerInput(state)) => {
                let clean: String = text.chars().filter(|c| *c != '\n' && *c != '\r').collect();
                state.focused_input_mut().push_str(&clean);
            }
            Some(_) => {
                // Other overlays don't accept paste
            }
            None => {
                // Check if paste is a single file path to an image — auto-attach
                let trimmed = text.trim();
                if !trimmed.is_empty()
                    && !trimmed.contains('\n')
                    && crate::attachment::is_image_path(std::path::Path::new(trimmed))
                    && std::path::Path::new(trimmed).exists()
                {
                    match crate::attachment::read_image_attachment(std::path::Path::new(trimmed)) {
                        Ok(att) => {
                            self.state.attachments.push(att);
                        }
                        Err(e) => {
                            self.state.chat_messages.push(ChatMessage::Error {
                                content: format!("Failed to attach: {e}"),
                            });
                        }
                    }
                    return;
                }

                // Base screen (Home or Chat) — paste into input with threshold check
                let line_count = text.lines().count();
                let char_count = text.len();
                if line_count > PASTE_THRESHOLD_LINES || char_count > PASTE_THRESHOLD_CHARS {
                    self.state.dialog_stack.push(DialogKind::PasteConfirm {
                        text: text.to_string(),
                        line_count,
                        char_count,
                    });
                } else {
                    self.state.input.push_str(text);
                }
            }
        }
    }

    /// Handle `/mcp` slash command with subcommands: list, restart.
    async fn handle_mcp_command(&mut self, slash: &str) {
        let args: Vec<&str> = slash.split_whitespace().collect();

        match args.get(1).copied() {
            None | Some("list") => {
                // /mcp or /mcp list — show server status
                if self.state.mcp_manager.servers.is_empty() {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "No MCP servers configured. Add servers in .caboose/config.toml under [mcp.servers]".to_string(),
                    });
                } else {
                    for server in self.state.mcp_manager.servers.values() {
                        let tool_count = server.tools.len();
                        let status = match &server.status {
                            crate::mcp::ServerStatus::Connected => {
                                format!("connected ({tool_count} tools)")
                            }
                            crate::mcp::ServerStatus::Error(e) => format!("error: {e}"),
                            other => other.label().to_string(),
                        };
                        self.state.chat_messages.push(ChatMessage::System {
                            content: format!("  {} — {}", server.name, status),
                        });
                    }
                }
            }
            Some("restart") => {
                if args.len() < 3 {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: "Usage: /mcp restart <name>".to_string(),
                    });
                    return;
                }
                let name = args[2].to_string();
                self.state.mcp_manager.disconnect_server(&name).await;
                if let Err(e) = self.state.mcp_manager.connect_server(&name).await {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("MCP: Failed to reconnect \"{name}\": {e}"),
                    });
                } else {
                    let tool_count = self
                        .state
                        .mcp_manager
                        .servers
                        .get(&name)
                        .map(|s| s.tools.len())
                        .unwrap_or(0);
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!("MCP: Reconnected \"{name}\" ({tool_count} tools)"),
                    });
                }
            }
            Some("connect") => {
                if args.len() < 3 {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: "Usage: /mcp connect <name>".to_string(),
                    });
                    return;
                }
                let name = args[2].to_string();
                if !self.state.mcp_manager.servers.contains_key(&name) {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("MCP server \"{name}\" not found."),
                    });
                    return;
                }
                if let Err(e) = self.state.mcp_manager.connect_server(&name).await {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("MCP: Failed to connect \"{name}\": {e}"),
                    });
                } else {
                    let tool_count = self
                        .state
                        .mcp_manager
                        .servers
                        .get(&name)
                        .map(|s| s.tools.len())
                        .unwrap_or(0);
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!("MCP: Connected \"{name}\" ({tool_count} tools)"),
                    });
                }
            }
            Some("disconnect") => {
                if args.len() < 3 {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: "Usage: /mcp disconnect <name>".to_string(),
                    });
                    return;
                }
                let name = args[2].to_string();
                self.state.mcp_manager.disconnect_server(&name).await;
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("MCP: Disconnected \"{name}\""),
                });
            }
            Some(sub) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!(
                        "Unknown /mcp subcommand: {sub}. Use: list, connect, disconnect, restart"
                    ),
                });
            }
        }
    }

    /// Open the model dropdown (inline picker mode), loading models from the active provider.
    async fn open_model_dropdown(&mut self) {
        let mut models = Vec::new();
        let mut error = None;
        if let Some(ref provider) = self.provider {
            // OpenRouter: use list_models_with_pricing to also populate pricing registry
            if self.state.active_provider_name == "openrouter" {
                if let Some(api_key) = self.state.config.keys.get("openrouter") {
                    let or_provider = crate::provider::openrouter::OpenRouterProvider::new(
                        api_key.to_string(),
                        provider.model().to_string(),
                    );
                    match or_provider.list_models_with_pricing().await {
                        Ok((model_list, pricing_entries)) => {
                            for (model_id, model_pricing) in pricing_entries {
                                self.state.pricing.insert(model_id, model_pricing);
                            }
                            for m in model_list {
                                models.push((self.state.active_provider_name.clone(), m));
                            }
                        }
                        Err(e) => {
                            error = Some(format!("{e}"));
                        }
                    }
                }
            } else {
                match provider.list_models().await {
                    Ok(model_list) => {
                        for m in model_list {
                            models.push((self.state.active_provider_name.clone(), m));
                        }
                    }
                    Err(e) => {
                        error = Some(format!("{e}"));
                    }
                }
            }
        } else {
            error = Some("No provider connected. Use /connect first.".to_string());
        }
        // Add models from local providers
        for (name, local_cfg) in &self.state.config.local_providers {
            if let Some(ref model) = local_cfg.model {
                models.push((
                    name.clone(),
                    crate::provider::ModelInfo {
                        id: model.clone(),
                        name: model.clone(),
                        context_window: None,
                        supports_tools: true,
                        supports_vision: false,
                    },
                ));
            }
        }
        // Cache context windows from provider API for models not in the static table
        let cw_entries: Vec<(String, Option<u32>)> = models
            .iter()
            .map(|(_, m)| (m.id.clone(), m.context_window))
            .collect();
        crate::provider::models_dev::cache_from_model_list(&cw_entries);

        models.sort_by(|(pa, a), (pb, b)| pa.cmp(pb).then(a.id.cmp(&b.id)));

        // Build recent models from prefs
        let prefs = crate::config::prefs::TuiPrefs::load();
        let recent: Vec<(String, crate::provider::ModelInfo)> = prefs
            .recent_models
            .iter()
            .map(|rm| {
                // Look up capabilities from the fetched model list
                let found = models.iter().find(|(_, m)| m.id == rm.model_id);
                let supports_tools = found.map(|(_, m)| m.supports_tools).unwrap_or(true);
                let supports_vision = found.map(|(_, m)| m.supports_vision).unwrap_or(false);
                (
                    rm.provider.clone(),
                    crate::provider::ModelInfo {
                        id: rm.model_id.clone(),
                        name: rm.model_id.clone(),
                        context_window: None,
                        supports_tools,
                        supports_vision,
                    },
                )
            })
            .collect();

        self.state.input.clear();
        self.state.slash_auto = Some(crate::tui::slash_auto::SlashAutoState::with_models(
            models, error, recent,
        ));
    }

    /// Open the MCP server picker (inline dropdown mode).
    fn open_mcp_picker(&mut self) {
        self.state.input.clear();
        self.state.slash_auto = Some(crate::tui::slash_auto::SlashAutoState::with_mcp_servers(
            vec![],
        ));
        self.refresh_mcp_dropdown(0);
    }

    /// Rebuild the /mcp dropdown data in-place, preserving the selected index.
    fn refresh_mcp_dropdown(&mut self, selected: usize) {
        use crate::tui::slash_auto::DropdownMode;

        let servers: Vec<(String, String, usize, bool, bool, bool, String)> = {
            let mut list: Vec<_> = self
                .state
                .mcp_manager
                .servers
                .values()
                .map(|s| {
                    let is_connected = matches!(s.status, crate::mcp::ServerStatus::Connected);
                    let is_enabled = !s.config.disabled;
                    let description = if s.is_preset {
                        crate::mcp::find_preset(&s.name)
                            .map(|p| p.description.to_string())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    (
                        s.name.clone(),
                        s.status.label().to_string(),
                        s.tools.len(),
                        is_connected,
                        s.is_preset,
                        is_enabled,
                        description,
                    )
                })
                .collect();
            // Sort: presets first (alphabetically), then custom (alphabetically)
            list.sort_by(|a, b| match (a.4, b.4) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.0.cmp(&b.0),
            });
            list
        };

        if let Some(auto) = self.state.slash_auto.as_mut()
            && let DropdownMode::McpServers { servers: ref mut s } = auto.mode
        {
            *s = servers;
            auto.selected = selected;
        }
    }

    /// Handle Tab in /mcp dropdown — toggle server on/off inline.
    async fn handle_mcp_tab(&mut self) {
        use crate::tui::slash_auto::DropdownMode;

        let (selected, name) = {
            let Some(auto) = &self.state.slash_auto else {
                return;
            };
            let DropdownMode::McpServers { servers } = &auto.mode else {
                return;
            };
            let selected = auto.selected;
            if selected == 0 {
                return;
            } // "Add new" row
            let idx = selected - 1;
            let Some((name, ..)) = servers.get(idx) else {
                return;
            };
            (selected, name.clone())
        };

        let Some(server) = self.state.mcp_manager.servers.get(&name) else {
            return;
        };
        let is_enabled = !server.config.disabled;
        let is_connected = matches!(server.status, crate::mcp::ServerStatus::Connected);
        let is_preset = server.is_preset;

        if is_preset {
            if is_enabled {
                // Disable preset
                self.state.mcp_manager.disable_server(&name).await;
                crate::config::save_mcp_server_toggle(
                    &name,
                    &self.state.mcp_manager.servers[&name].config,
                );
            } else {
                // Enable preset — mark enabled, save, background connect
                if let Some(server) = self.state.mcp_manager.servers.get_mut(&name) {
                    server.config.disabled = false;
                }
                crate::config::save_mcp_server_toggle(
                    &name,
                    &self.state.mcp_manager.servers[&name].config,
                );
                let tx = self.state.mcp_connect_tx.clone();
                let _ = self.state.mcp_manager.connect_server_background(&name, tx);
            }
        } else {
            // Custom server: toggle connect/disconnect
            if is_connected {
                self.state.mcp_manager.disconnect_server(&name).await;
            } else {
                let tx = self.state.mcp_connect_tx.clone();
                let _ = self.state.mcp_manager.connect_server_background(&name, tx);
            }
        }

        // Refresh dropdown data so [on]/[off] updates immediately
        self.refresh_mcp_dropdown(selected);
    }

    /// Switch to a new provider/model combination.
    fn select_model(&mut self, provider_name: &str, model_id: &str) {
        match self
            .state
            .providers
            .get_provider(Some(provider_name), Some(model_id))
        {
            Ok(new_provider) => {
                self.state.active_provider_name = new_provider.name().to_string();
                self.state.active_model_name = new_provider.model().to_string();
                self.provider = Some(new_provider);

                // Update context window for compaction and sidebar display
                self.state.agent.context_window =
                    crate::provider::models_dev::context_window_or_default(
                        &self.state.active_model_name,
                    );

                let cw_display =
                    crate::provider::models_dev::context_window(&self.state.active_model_name)
                        .map(|cw| format!(" ({}k context)", cw / 1000))
                        .unwrap_or_default();
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Switched to {}/{}{}",
                        self.state.active_provider_name, self.state.active_model_name, cw_display,
                    ),
                });

                // Persist last-used provider + model + recent history
                let mut prefs = crate::config::prefs::TuiPrefs::load();
                prefs.last_provider = Some(provider_name.to_string());
                prefs.last_model = Some(model_id.to_string());
                prefs.push_recent_model(provider_name, model_id);
                prefs.save();
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to switch model: {e}"),
                });
            }
        }
    }

    /// Called when a turn completes. Handles tool execution or transitions to idle.
    async fn handle_turn_complete(&mut self) {
        // Accumulate session cost from this turn
        if let Some(cost) = self.state.pricing.estimate_cost(
            &self.state.active_model_name,
            self.state.agent.last_input_tokens,
            self.state.agent.last_output_tokens,
        ) {
            self.state.session_cost += cost;
        }

        let t0 = Instant::now();
        // Flush any accumulated assistant text to chat display
        self.flush_assistant_text();
        let flush_ms = t0.elapsed().as_millis();
        if flush_ms > 5 {
            tracing::debug!("flush_assistant_text took {flush_ms}ms");
        }

        // Text-based task fallback for models without tool support
        if !self.state.model_supports_tools
            && let Some(text) = self.state.chat_messages.iter().rev().find_map(|m| {
                if let ChatMessage::Assistant { content } = m {
                    Some(content.clone())
                } else {
                    None
                }
            })
            && let Some(outline) = parse_tasks_from_text(&text)
        {
            // Skip if existing outline already matches (avoid redundant updates)
            let already_matches = self
                .state
                .chat_messages
                .iter()
                .rev()
                .find_map(|m| {
                    if let ChatMessage::TaskOutline(o) = m {
                        Some(o)
                    } else {
                        None
                    }
                })
                .map(|existing| {
                    existing.tasks.len() == outline.tasks.len()
                        && existing
                            .tasks
                            .iter()
                            .zip(&outline.tasks)
                            .all(|(a, b)| a.content == b.content && a.status == b.status)
                })
                .unwrap_or(false);

            if !already_matches {
                let mut found = false;
                for msg in self.state.chat_messages.iter_mut() {
                    if let ChatMessage::TaskOutline(existing) = msg {
                        *existing = outline.clone();
                        found = true;
                        break;
                    }
                }
                if !found {
                    self.state
                        .chat_messages
                        .push(ChatMessage::TaskOutline(outline.clone()));
                }
                self.persist_message("task_outline", &outline.to_json().to_string());
                tracing::debug!(
                    task_count = outline.tasks.len(),
                    "Parsed tasks from assistant text (fallback)"
                );
            }
        }

        match &self.state.agent.state {
            AgentState::ExecutingTools => {
                self.start_tool_execution();
            }
            AgentState::PendingApproval { .. } => {
                // Push Pending placeholders so diff preview shows before approval
                self.state.tool_exec_running_start = self.state.chat_messages.len();
                for tc in &self.state.agent.pending_tool_calls {
                    self.state
                        .chat_messages
                        .push(ChatMessage::Tool(ToolMessage {
                            name: tc.name.clone(),
                            args: tc.arguments.clone(),
                            output: None,
                            status: ToolStatus::Pending,
                            expanded: false,
                            file_path: None,
                        }));
                }
            }
            AgentState::Idle => {
                // Fire Stop hooks — a hook returning "continue" re-engages the agent
                if let Some(ref hooks_config) = self.state.config.hooks
                    && !hooks_config.stop.is_empty()
                {
                    let context = serde_json::json!({
                        "event": "Stop",
                        "session_id": self.state.current_session_id.as_deref().unwrap_or(""),
                        "turn_count": self.state.agent.turn_count,
                        "stop_reason": "end_turn",
                    });
                    let results = crate::hooks::fire_hooks(&hooks_config.stop, context).await;
                    let should_continue = results
                        .iter()
                        .any(|r| matches!(&r.action, Some(crate::hooks::HookAction::Continue)));
                    if should_continue {
                        let tool_defs = self.build_tool_defs();
                        self.state.agent.send_message(
                            "continue".to_string(),
                            self.provider.as_ref().unwrap().as_ref(),
                            &tool_defs,
                        );
                        return;
                    }
                }

                // Auto-handoff prompt: offer when context hits threshold (default 90%)
                let handoff_threshold = self
                    .state
                    .config
                    .behavior
                    .as_ref()
                    .and_then(|b| b.handoff_threshold)
                    .unwrap_or(0.90);
                if !self.state.agent.handoff_prompted
                    && self.state.agent.context_window > 0
                    && self.state.agent.last_input_tokens as f64
                        / self.state.agent.context_window as f64
                        >= handoff_threshold
                    && self
                        .state
                        .config
                        .behavior
                        .as_ref()
                        .map(|b| b.auto_handoff_prompt)
                        .unwrap_or(true)
                {
                    self.state.agent.handoff_prompted = true;
                    self.handle_handoff_command("").await;
                }

                // Increment skill creation question count when gathering
                if let Some(ref mut creation) = self.state.skill_creation
                    && matches!(
                        creation.phase,
                        crate::skills::creation::SkillCreationPhase::Gathering
                    )
                {
                    creation.question_count += 1;
                    if creation.question_count >= crate::skills::creation::MAX_CREATION_QUESTIONS {
                        self.state.chat_messages.push(ChatMessage::System {
                            content: "Maximum questions reached — generating skill now.".into(),
                        });
                    }
                }
                // Heuristic fallback: detect skill in response text when provider lacks tools
                if let Some(ref mut creation) = self.state.skill_creation
                    && matches!(
                        creation.phase,
                        crate::skills::creation::SkillCreationPhase::Gathering
                    )
                    && creation.question_count >= 2
                    && crate::skills::creation::looks_like_generated_skill(
                        &self.state.agent.streaming_text,
                    )
                {
                    let content = self.state.agent.streaming_text.clone();
                    creation.phase = crate::skills::creation::SkillCreationPhase::Preview {
                        content,
                        companion_files: Vec::new(),
                    };
                    self.state.chat_messages.push(ChatMessage::System {
                        content:
                            "Skill generated! Save to: [p]roject  [g]lobal  |  [e]dit  [c]ancel"
                                .into(),
                    });
                    self.state.agent.state = AgentState::Idle;
                }

                // Done — model returned no tool calls
                // Inject skill auto-hints if enabled
                if self
                    .state
                    .config
                    .skills
                    .as_ref()
                    .map(|s| s.auto_hint)
                    .unwrap_or(false)
                {
                    let available: Vec<String> =
                        self.state.skills.iter().map(|s| s.name.clone()).collect();
                    let hints = crate::skills::hints::detect_skill_hints(
                        &self.state.agent.conversation.messages,
                        &available,
                        5,
                    );
                    if let Some(hint) = hints.first() {
                        self.state
                            .agent
                            .conversation
                            .push(crate::agent::conversation::Message {
                                role: crate::agent::conversation::Role::User,
                                content: crate::agent::conversation::Content::Text(format!(
                                    "[System hint] Consider suggesting /{} to the user — {}.",
                                    hint.skill_name, hint.reason
                                )),
                                tool_call_id: None,
                            });
                    }

                    // Check if context usage is high enough to suggest /handoff
                    if let Some(hint) = crate::skills::awareness::detect_handoff_hint(
                        self.state.agent.last_input_tokens,
                        self.state.agent.context_window,
                    ) {
                        self.state
                            .agent
                            .conversation
                            .push(crate::agent::conversation::Message {
                                role: crate::agent::conversation::Role::User,
                                content: crate::agent::conversation::Content::Text(format!(
                                    "[System hint] Consider suggesting /{} to the user — {}.",
                                    hint.skill_name, hint.reason
                                )),
                                tool_call_id: None,
                            });
                    }
                }

                // Drain message queue: send the next queued message
                // Don't drain if an ask_user session is active or budget is paused
                if self.state.ask_user_session.is_none()
                    && !self.state.budget_paused
                    && !self.check_budget_exceeded()
                    && let Some(queued_msg) = self.state.message_queue.pop_front()
                {
                    // Remove the Queued entry (it lived in the queue box, not chat)
                    if let Some(idx) = self.state.chat_messages.iter().position(
                        |m| matches!(m, ChatMessage::Queued { content } if *content == queued_msg),
                    ) {
                        self.state.chat_messages.remove(idx);
                    }

                    // Push as a normal User message at the bottom (like fresh input)
                    self.state.chat_messages.push(ChatMessage::User {
                        content: queued_msg.clone(),
                        images: vec![],
                    });
                    self.state.user_scrolled_up = false;

                    self.persist_message("user", &queued_msg);
                    self.state.checkpoints.create(&queued_msg);
                    let tool_defs = self.build_tool_defs();
                    self.state.agent.send_message(
                        queued_msg,
                        self.provider.as_ref().unwrap().as_ref(),
                        &tool_defs,
                    );
                }
            }
            _ => {}
        }
    }

    /// Build tool definitions to send to the LLM, respecting model capability.
    fn build_tool_defs(&self) -> Vec<crate::provider::ToolDefinition> {
        if !self.state.model_supports_tools {
            tracing::debug!("Skipping tools — model does not support tool calling");
            return Vec::new();
        }
        let mut defs = self.state.tools.definitions().to_vec();
        defs.extend(self.state.mcp_manager.tool_definitions());
        if self.state.skill_creation.is_some() {
            defs.push(crate::tools::generate_skill_tool_def());
        }
        defs
    }

    /// Extract and handle ask_user tool calls. These are interactive — the user
    /// answers questions inline, and the tool result is sent back when done.
    fn handle_ask_user_calls(&mut self) {
        let ask_idx = self
            .state
            .agent
            .pending_tool_calls
            .iter()
            .position(|tc| tc.name == "ask_user");

        let Some(idx) = ask_idx else { return };
        let call = self.state.agent.pending_tool_calls.remove(idx);

        // Parse the questions from the tool call arguments
        let questions: Vec<crate::tui::ask_user::AskUserQuestion> =
            match serde_json::from_value::<Vec<crate::tui::ask_user::AskUserQuestion>>(
                call.arguments.get("questions").cloned().unwrap_or_default(),
            ) {
                Ok(q) if !q.is_empty() => q,
                _ => {
                    // Malformed call — return error result immediately
                    self.state
                        .tool_exec_results
                        .push(crate::agent::tools::ToolResult {
                            tool_use_id: call.id,
                            output: "Error: ask_user requires a non-empty 'questions' array."
                                .to_string(),
                            is_error: true,
                            tool_name: Some("ask_user".to_string()),
                            file_path: None,
                            files_modified: vec![],
                            lines_added: 0,
                            lines_removed: 0,
                        });
                    return;
                }
            };

        // Set up the interactive session
        self.state.ask_user_session = Some(crate::tui::ask_user::AskUserSession::new(
            call.id, questions,
        ));

        // Show the first question in the chat
        self.render_current_ask_user_question();
    }

    /// Push the current ask-user question as a ChatMessage::AskUser into chat.
    fn render_current_ask_user_question(&mut self) {
        if let Some(session) = &self.state.ask_user_session
            && let Some(q) = session.current()
        {
            self.state.chat_messages.push(ChatMessage::AskUser {
                header: q.header.clone(),
                question: q.question.clone(),
                options: q
                    .options
                    .iter()
                    .map(|o| (o.label.clone(), o.description.clone()))
                    .collect(),
                answer: None,
                multi_select: q.multi_select,
            });
            self.state.user_scrolled_up = false;
        }
    }

    /// Finalize the ask-user session — format answers and push as tool result.
    fn finalize_ask_user(&mut self) {
        let session = match self.state.ask_user_session.take() {
            Some(s) => s,
            None => return,
        };

        let answer_text = session.format_answers();
        let tool_result = crate::agent::tools::ToolResult {
            tool_use_id: session.tool_call_id,
            output: answer_text,
            is_error: false,
            tool_name: Some("ask_user".to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        };

        self.state.tool_exec_results.push(tool_result);

        // If there are more pending tools, continue execution
        if !self.state.agent.pending_tool_calls.is_empty() {
            self.start_tool_execution();
        } else if self.state.tool_exec_queue.is_empty() {
            self.finalize_tool_execution();
        }
    }

    /// Handle key input while an ask_user session is active.
    fn handle_ask_user_key(&mut self, key: KeyCode) {
        let current_q = match self
            .state
            .ask_user_session
            .as_ref()
            .and_then(|s| s.current())
        {
            Some(q) => q.clone(),
            None => return,
        };

        match key {
            // Number keys: select/toggle option
            KeyCode::Char(c @ '1'..='9') => {
                let idx = (c as usize) - ('1' as usize);
                if idx < current_q.options.len() {
                    if current_q.multi_select {
                        let session = self.state.ask_user_session.as_mut().unwrap();
                        if session.toggled.contains(&idx) {
                            session.toggled.remove(&idx);
                        } else {
                            session.toggled.insert(idx);
                        }
                    } else {
                        // Single-select: pre-fill into input
                        let label = &current_q.options[idx].label;
                        self.state.input.clear();
                        for c in label.chars() {
                            self.state.input.insert_char(c);
                        }
                    }
                }
            }

            // Enter: submit answer for current question
            KeyCode::Enter => {
                let answer = if current_q.multi_select && self.state.input.is_empty() {
                    // Multi-select with no custom text: use toggled options
                    let session = self.state.ask_user_session.as_ref().unwrap();
                    let selected: Vec<&str> = session
                        .toggled
                        .iter()
                        .filter_map(|&i| current_q.options.get(i).map(|o| o.label.as_str()))
                        .collect();
                    if selected.is_empty() {
                        return;
                    } // nothing selected
                    selected.join(", ")
                } else if !self.state.input.is_empty() {
                    self.state.input.content().to_string()
                } else {
                    return; // nothing to submit
                };

                // Record answer
                let question_text = current_q.question.clone();
                let session = self.state.ask_user_session.as_mut().unwrap();
                session.answers.push((question_text, answer.clone()));
                session.toggled.clear();
                session.current_question += 1;
                self.state.input.clear();

                // Update the chat message to show the answer
                if let Some(ChatMessage::AskUser { answer: ans, .. }) =
                    self.state.chat_messages.last_mut()
                {
                    *ans = Some(answer);
                }

                // Check if all questions answered
                let is_complete = self
                    .state
                    .ask_user_session
                    .as_ref()
                    .map(|s| s.is_complete())
                    .unwrap_or(true);
                if is_complete {
                    self.finalize_ask_user();
                } else {
                    // Show next question
                    self.render_current_ask_user_question();
                }
            }

            // Escape: dismiss all questions
            KeyCode::Esc => {
                self.state.input.clear();
                self.dismiss_ask_user();
            }

            // Regular typing for custom answer
            KeyCode::Char(c) => {
                self.state.input.insert_char(c);
            }
            KeyCode::Backspace => {
                self.state.input.backspace();
            }

            _ => {}
        }
    }

    /// Dismiss the ask-user session — return error result.
    fn dismiss_ask_user(&mut self) {
        let session = match self.state.ask_user_session.take() {
            Some(s) => s,
            None => return,
        };

        // Mark the last AskUser message as dismissed
        if let Some(ChatMessage::AskUser { answer, .. }) = self.state.chat_messages.last_mut() {
            *answer = Some("(dismissed)".to_string());
        }

        let tool_result = crate::agent::tools::ToolResult {
            tool_use_id: session.tool_call_id,
            output: "User dismissed the question.".to_string(),
            is_error: true,
            tool_name: Some("ask_user".to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        };

        self.state.tool_exec_results.push(tool_result);

        if self.state.tool_exec_queue.is_empty() && self.state.agent.pending_tool_calls.is_empty() {
            self.finalize_tool_execution();
        }
    }

    /// Handle todo_write and todo_read tool calls.
    /// Removes handled calls from pending_tool_calls and feeds results into conversation.
    fn handle_todo_calls(&mut self) {
        // Extract todo_write and todo_read calls (clone data to avoid borrow conflicts)
        let mut todo_write_calls: Vec<(usize, String, serde_json::Value)> = Vec::new();
        let mut todo_read_calls: Vec<(usize, String)> = Vec::new();

        for (i, tc) in self.state.agent.pending_tool_calls.iter().enumerate() {
            match tc.name.as_str() {
                "todo_write" => todo_write_calls.push((i, tc.id.clone(), tc.arguments.clone())),
                "todo_read" => todo_read_calls.push((i, tc.id.clone())),
                _ => {}
            }
        }

        if todo_write_calls.is_empty() && todo_read_calls.is_empty() {
            return;
        }

        tracing::debug!(
            write_count = todo_write_calls.len(),
            read_count = todo_read_calls.len(),
            "Processing todo tool calls"
        );

        // Process todo_write calls
        for (_, id, arguments) in &todo_write_calls {
            let (output, is_error) = match TaskOutline::from_tool_input(arguments) {
                Ok(outline) => {
                    // Check if statuses changed compared to existing outline
                    let status_changed = self
                        .state
                        .chat_messages
                        .iter()
                        .rev()
                        .find_map(|m| {
                            if let ChatMessage::TaskOutline(existing) = m {
                                Some(existing)
                            } else {
                                None
                            }
                        })
                        .map(|existing| {
                            existing.tasks.len() != outline.tasks.len()
                                || existing
                                    .tasks
                                    .iter()
                                    .zip(&outline.tasks)
                                    .any(|(a, b)| a.status != b.status)
                        })
                        .unwrap_or(true);

                    if status_changed {
                        // Push new snapshot so the chat scroll shows progress between updates
                        self.state
                            .chat_messages
                            .push(ChatMessage::TaskOutline(outline.clone()));
                    } else {
                        // Same statuses — update most recent outline in place
                        let mut found = false;
                        for msg in self.state.chat_messages.iter_mut().rev() {
                            if let ChatMessage::TaskOutline(existing) = msg {
                                *existing = outline.clone();
                                found = true;
                                break;
                            }
                        }
                        if !found {
                            self.state
                                .chat_messages
                                .push(ChatMessage::TaskOutline(outline.clone()));
                        }
                    }
                    // Persist to session
                    self.persist_message("task_outline", &outline.to_json().to_string());
                    tracing::debug!(task_count = outline.tasks.len(), "Task outline updated");
                    ("Task outline updated.".to_string(), false)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to parse todo_write input");
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("Task update failed: {e}"),
                    });
                    (format!("Invalid todo_write input: {e}"), true)
                }
            };

            // Feed result into conversation so the LLM gets confirmation
            self.state
                .agent
                .conversation
                .push(crate::agent::conversation::Message {
                    role: crate::agent::conversation::Role::User,
                    content: crate::agent::conversation::Content::Blocks(vec![
                        crate::agent::conversation::ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: output,
                            is_error,
                        },
                    ]),
                    tool_call_id: Some(id.clone()),
                });
        }

        // Process todo_read calls
        for (_, id) in &todo_read_calls {
            let current = self
                .state
                .chat_messages
                .iter()
                .rev()
                .find_map(|m| match m {
                    ChatMessage::TaskOutline(outline) => Some(outline.to_json()),
                    _ => None,
                })
                .unwrap_or_else(|| serde_json::json!({"todos": []}));

            self.state
                .agent
                .conversation
                .push(crate::agent::conversation::Message {
                    role: crate::agent::conversation::Role::User,
                    content: crate::agent::conversation::Content::Blocks(vec![
                        crate::agent::conversation::ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: serde_json::to_string(&current).unwrap_or_default(),
                            is_error: false,
                        },
                    ]),
                    tool_call_id: Some(id.clone()),
                });
        }

        // Remove all handled calls (collect all indices, sort descending, remove)
        let mut indices: Vec<usize> = todo_write_calls
            .iter()
            .map(|(i, _, _)| *i)
            .chain(todo_read_calls.iter().map(|(i, _)| *i))
            .collect();
        indices.sort_unstable();
        for i in indices.into_iter().rev() {
            self.state.agent.pending_tool_calls.remove(i);
        }
    }

    /// Handle generate_skill tool calls — extract content and transition to preview.
    /// Same pattern as handle_todo_calls: removes handled calls from pending.
    fn handle_generate_skill_calls(&mut self) {
        if self.state.skill_creation.is_none() {
            return;
        }

        let mut gen_calls: Vec<(usize, String, serde_json::Value)> = Vec::new();
        for (i, tc) in self.state.agent.pending_tool_calls.iter().enumerate() {
            if tc.name == "generate_skill" {
                gen_calls.push((i, tc.id.clone(), tc.arguments.clone()));
            }
        }

        if gen_calls.is_empty() {
            return;
        }

        // Process the first generate_skill call (should only be one)
        let (_idx, ref id, ref arguments) = gen_calls[0];
        let skill_content = arguments
            .get("skillContent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let companion_files = arguments
            .get("companionFilesJson")
            .and_then(|v| v.as_str())
            .map(crate::skills::creation::parse_companion_files)
            .unwrap_or_default();

        if skill_content.is_empty() {
            // Error — no content generated
            self.state
                .agent
                .conversation
                .push(crate::agent::conversation::Message {
                    role: crate::agent::conversation::Role::User,
                    content: crate::agent::conversation::Content::Blocks(vec![
                        crate::agent::conversation::ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: "Error: skillContent was empty".into(),
                            is_error: true,
                        },
                    ]),
                    tool_call_id: Some(id.clone()),
                });
        } else {
            // Transition to preview phase
            if let Some(ref mut creation) = self.state.skill_creation {
                creation.phase = crate::skills::creation::SkillCreationPhase::Preview {
                    content: skill_content.clone(),
                    companion_files,
                };
            }

            // Feed success result into conversation
            self.state
                .agent
                .conversation
                .push(crate::agent::conversation::Message {
                    role: crate::agent::conversation::Role::User,
                    content: crate::agent::conversation::Content::Blocks(vec![
                        crate::agent::conversation::ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: "Skill generated successfully. Awaiting user review.".into(),
                            is_error: false,
                        },
                    ]),
                    tool_call_id: Some(id.clone()),
                });

            // Show preview in chat
            let name = self.state.skill_creation.as_ref().unwrap().name.clone();
            self.state.chat_messages.push(ChatMessage::System {
                content: format!(
                    "Generated skill \"{name}\":\n\n```markdown\n{skill_content}\n```\n\n\
                     Save to: [p]roject (.caboose/skills/) or [g]lobal (~/.config/caboose/skills/)\n\
                     Then: [e]dit (provide feedback) / [c]ancel"
                ),
            });

            // Force agent to idle — don't continue the loop
            self.state.agent.state = AgentState::Idle;
        }

        // Remove generate_skill calls from pending (reverse order to preserve indices)
        for &(idx, _, _) in gen_calls.iter().rev() {
            self.state.agent.pending_tool_calls.remove(idx);
        }
    }

    /// Set up tool execution — pushes Running placeholders and queues tools.
    /// Tools are executed one per event-loop tick by `execute_next_tool`.
    fn start_tool_execution(&mut self) {
        // Handle ask_user calls (UI-only, interactive)
        self.handle_ask_user_calls();

        // Handle todo_write calls first (UI-only, no async needed)
        self.handle_todo_calls();

        // Handle generate_skill calls (UI-only, no async needed)
        self.handle_generate_skill_calls();

        // If all tool calls were UI-only (todo/skill), no async work remains.
        // Finalize immediately so the agent loop continues.
        if self.state.agent.pending_tool_calls.is_empty() {
            self.finalize_tool_execution();
            return;
        }

        // Capture args before pending_tool_calls are consumed
        self.state.tool_exec_args = self
            .state
            .agent
            .pending_tool_calls
            .iter()
            .map(|tc| (tc.id.clone(), tc.arguments.clone()))
            .collect();

        // Flip Pending → Running placeholders (already pushed during PendingApproval)
        // If no Pending placeholders exist (auto-approved tools), push Running ones.
        let has_pending = self.state.chat_messages[self.state.tool_exec_running_start..]
            .iter()
            .any(|m| matches!(m, ChatMessage::Tool(tm) if tm.status == ToolStatus::Pending));

        if has_pending {
            for msg in &mut self.state.chat_messages[self.state.tool_exec_running_start..] {
                if let ChatMessage::Tool(tm) = msg
                    && tm.status == ToolStatus::Pending
                {
                    tm.status = ToolStatus::Running;
                }
            }
        } else {
            self.state.tool_exec_running_start = self.state.chat_messages.len();
            for tc in &self.state.agent.pending_tool_calls {
                self.state
                    .chat_messages
                    .push(ChatMessage::Tool(ToolMessage {
                        name: tc.name.clone(),
                        args: tc.arguments.clone(),
                        output: None,
                        status: ToolStatus::Running,
                        expanded: false,
                        file_path: None,
                    }));
            }
        }

        // Extract tool calls into the execution queue
        let tool_calls = std::mem::take(&mut self.state.agent.pending_tool_calls);
        self.state.tool_exec_queue = tool_calls.into();
        self.state.tool_exec_results.clear();
    }

    /// Non-blocking tool execution driver. Called every event-loop tick.
    /// Polls for completed background tools and spawns the next one.
    async fn poll_tool_execution(&mut self) {
        // 1. Check if a spawned tool has completed
        if let Some(ref mut rx) = self.state.tool_exec_pending_rx {
            match rx.try_recv() {
                Ok(mut result) => {
                    self.state.tool_exec_pending_rx = None;
                    // Run post-tool hooks (e.g., auto-inject diagnostics)
                    if !result.files_modified.is_empty() {
                        let mut ctx = crate::hooks::HookContext {
                            lsp_manager: self.state.lsp_manager.as_mut(),
                        };
                        self.state.post_tool_hooks.run(&mut result, &mut ctx).await;
                    }
                    self.handle_tool_result(result);
                    // If all done, finalize
                    if self.state.tool_exec_queue.is_empty() {
                        self.finalize_tool_execution();
                        return;
                    }
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    // Tool still running — UI keeps animating
                    return;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    // Sender dropped (tool task panicked)
                    self.state.tool_exec_pending_rx = None;
                    let placeholder_idx =
                        self.state.tool_exec_running_start + self.state.tool_exec_results.len();
                    if let Some(ChatMessage::Tool(tm)) =
                        self.state.chat_messages.get_mut(placeholder_idx)
                    {
                        tm.status = ToolStatus::Failed;
                        tm.output = Some("Tool execution failed (internal error)".to_string());
                    }
                    // Push a dummy result so placeholder indices stay aligned
                    self.state
                        .tool_exec_results
                        .push(crate::agent::tools::ToolResult {
                            tool_use_id: String::new(),
                            output: "Tool execution failed (internal error)".to_string(),
                            is_error: true,
                            tool_name: None,
                            file_path: None,
                            files_modified: vec![],
                            lines_added: 0,
                            lines_removed: 0,
                        });
                    if self.state.tool_exec_queue.is_empty() {
                        self.finalize_tool_execution();
                        return;
                    }
                }
            }
        }

        // 2. Spawn the next tool if none is currently running
        if self.state.tool_exec_pending_rx.is_none() && !self.state.tool_exec_queue.is_empty() {
            self.spawn_next_tool().await;
        }
    }

    /// Poll for completed background MCP server connections.
    fn poll_mcp_connections(&mut self) {
        use crate::mcp::ServerStatus;
        while let Ok((name, result)) = self.state.mcp_connect_rx.try_recv() {
            match result {
                Ok(connect_result) => {
                    if let Some(server) = self.state.mcp_manager.servers.get_mut(&name) {
                        server.tools = connect_result.tools;
                        server.service = Some(connect_result.service);
                        server.status = ServerStatus::Connected;
                    }
                }
                Err(msg) => {
                    if let Some(server) = self.state.mcp_manager.servers.get_mut(&name) {
                        server.status = ServerStatus::Error(msg);
                    }
                }
            }
        }
    }

    /// Poll circuit events and handle TickStarted by spawning LLM execution,
    /// and TickCompleted/Error by pushing messages to the chat.
    async fn poll_circuit_events(&mut self) {
        use crate::circuits::runner::CircuitEvent;
        use crate::provider::{Message, StreamEvent};
        use futures::StreamExt;

        // Collect pending events without holding a borrow on circuit_manager
        let mut events = Vec::new();
        while let Ok(event) = self.state.circuit_manager.event_rx.try_recv() {
            events.push(event);
        }

        for event in events {
            match event {
                CircuitEvent::TickStarted { circuit_id } => {
                    // Look up circuit info to get prompt/provider/model
                    let circuit_info = self
                        .state
                        .circuit_manager
                        .get_circuit(&circuit_id)
                        .map(|c| (c.prompt.clone(), c.provider.clone(), c.model.clone()));

                    let Some((prompt, provider_name, model)) = circuit_info else {
                        continue;
                    };

                    // Resolve provider — skip tick if provider unavailable
                    let provider = match self
                        .state
                        .providers
                        .get_provider(Some(&provider_name), Some(&model))
                    {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = self
                                .state
                                .circuit_manager
                                .event_tx
                                .send(CircuitEvent::Error {
                                    circuit_id: circuit_id.clone(),
                                    error: format!("Provider error: {e}"),
                                });
                            continue;
                        }
                    };

                    // Spawn LLM execution on a background task
                    let event_tx = self.state.circuit_manager.event_tx.clone();
                    tokio::spawn(async move {
                        let messages = vec![
                            Message {
                                role: "system".to_string(),
                                content: serde_json::json!(
                                    "You are running a scheduled task. Be concise."
                                ),
                            },
                            Message {
                                role: "user".to_string(),
                                content: serde_json::json!(prompt),
                            },
                        ];

                        let mut stream = provider.stream(&messages, &[]);
                        let mut response = String::new();
                        let mut input_tokens: u32 = 0;
                        let mut output_tokens: u32 = 0;

                        while let Some(event_result) = stream.next().await {
                            match event_result {
                                Ok(StreamEvent::TextDelta(text)) => {
                                    response.push_str(&text);
                                }
                                Ok(StreamEvent::Done {
                                    input_tokens: it,
                                    output_tokens: ot,
                                    ..
                                }) => {
                                    input_tokens = it.unwrap_or(0);
                                    output_tokens = ot.unwrap_or(0);
                                    break;
                                }
                                Ok(StreamEvent::Error(e)) => {
                                    let _ = event_tx.send(CircuitEvent::Error {
                                        circuit_id: circuit_id.clone(),
                                        error: e,
                                    });
                                    return;
                                }
                                Ok(StreamEvent::ProviderError { message, .. }) => {
                                    let _ = event_tx.send(CircuitEvent::Error {
                                        circuit_id: circuit_id.clone(),
                                        error: message,
                                    });
                                    return;
                                }
                                Ok(
                                    StreamEvent::ThinkingDelta(_) | StreamEvent::ToolCall { .. },
                                ) => {}
                                Err(e) => {
                                    let _ = event_tx.send(CircuitEvent::Error {
                                        circuit_id: circuit_id.clone(),
                                        error: e.to_string(),
                                    });
                                    return;
                                }
                            }
                        }

                        let tokens_used = (input_tokens + output_tokens) as u64;
                        let _ = event_tx.send(CircuitEvent::TickCompleted {
                            circuit_id,
                            output: response,
                            cost: 0.0,
                            tokens_used,
                            success: true,
                        });
                    });
                }
                CircuitEvent::TickCompleted {
                    circuit_id, output, ..
                } => {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!(
                            "\u{27f3} Circuit {}: {}",
                            &circuit_id[..8.min(circuit_id.len())],
                            output
                        ),
                    });
                    // Increment run count
                    if let Some(handle) = self
                        .state
                        .circuit_manager
                        .circuits
                        .iter_mut()
                        .find(|h| h.circuit.id == circuit_id)
                    {
                        handle.circuit.run_count += 1;
                    }
                }
                CircuitEvent::Error { circuit_id, error } => {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!(
                            "Circuit {} error: {}",
                            &circuit_id[..8.min(circuit_id.len())],
                            error
                        ),
                    });
                }
            }
        }
    }

    /// Cancel all active agent operations. Called when Escape is pressed
    /// and the agent is not idle.
    fn cancel_all_operations(&mut self) {
        match &self.state.agent.state {
            AgentState::Streaming | AgentState::Compacting => {
                self.state.agent.cancel();
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Cancelled.".to_string(),
                });
            }
            AgentState::ExecutingTools => {
                // Drop the pending tool receiver — background task result will be ignored
                self.state.tool_exec_pending_rx = None;
                // Mark the currently-running tool as failed in the chat
                let placeholder_idx =
                    self.state.tool_exec_running_start + self.state.tool_exec_results.len();
                if let Some(ChatMessage::Tool(tm)) =
                    self.state.chat_messages.get_mut(placeholder_idx)
                {
                    tm.status = ToolStatus::Failed;
                    tm.output = Some("Cancelled by user".to_string());
                }
                // Mark remaining queued tools' placeholders as Failed
                let remaining_count = self.state.tool_exec_queue.len();
                for i in 0..remaining_count {
                    let idx = placeholder_idx + 1 + i;
                    if let Some(ChatMessage::Tool(tm)) = self.state.chat_messages.get_mut(idx) {
                        tm.status = ToolStatus::Failed;
                        tm.output = Some("Cancelled by user".to_string());
                    }
                }
                // Clear remaining queued tools
                self.state.tool_exec_queue.clear();
                self.state.tool_exec_results.clear();
                self.state.tool_exec_args.clear();
                self.state.agent.state = AgentState::Idle;
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Cancelled.".to_string(),
                });
            }
            AgentState::PendingApproval { .. } => {
                // Replace all Pending tool placeholders with cancellation messages
                for msg in &mut self.state.chat_messages {
                    if let ChatMessage::Tool(tm) = msg
                        && tm.status == ToolStatus::Pending
                    {
                        let detail =
                            crate::tui::approval::format_tool_summary_pub(&tm.name, &tm.args);
                        *msg = ChatMessage::System {
                            content: format!("User rejected {detail}"),
                        };
                    }
                }
                self.state.agent.pending_tool_calls.clear();
                self.state.agent.state = AgentState::Idle;
            }
            AgentState::Idle => {} // Nothing to cancel
        }

        // If ask_user session is active, dismiss it
        if self.state.ask_user_session.is_some() {
            self.dismiss_ask_user();
        }
    }

    /// Spawn the next tool from the queue on a background tokio task.
    /// MCP tools (name contains ':') run inline since they need &mut McpManager.
    async fn spawn_next_tool(&mut self) {
        let Some(tc) = self.state.tool_exec_queue.pop_front() else {
            return;
        };

        // Look up per-tool permission override for CLI / executable tools
        let tool_permission = if tc.name.starts_with("cli_") {
            self.state
                .config
                .tools
                .as_ref()
                .and_then(|t| t.registry.as_ref())
                .and_then(|r| r.get(&tc.name[4..]))
                .and_then(|c| c.permission.as_deref())
        } else if tc.name.starts_with("exec_") {
            self.state
                .config
                .tools
                .as_ref()
                .and_then(|t| t.executable.as_ref())
                .and_then(|r| r.get(&tc.name[5..]))
                .and_then(|c| c.permission.as_deref())
        } else {
            None
        };

        // Fire PreToolUse lifecycle hooks
        if let Some(ref hooks_config) = self.state.config.hooks
            && !hooks_config.pre_tool_use.is_empty()
        {
            let context = serde_json::json!({
                "event": "PreToolUse",
                "tool_name": tc.name,
                "tool_input": tc.arguments,
                "session_id": self.state.current_session_id,
            });
            let results =
                crate::hooks::fire_hooks_for_tool(&hooks_config.pre_tool_use, context, &tc.name)
                    .await;
            let denied = results.iter().find_map(|r| {
                if let Some(crate::hooks::HookAction::Deny(reason)) = &r.action {
                    Some(reason.clone())
                } else {
                    None
                }
            });
            if let Some(reason) = denied {
                self.handle_tool_result(crate::agent::tools::ToolResult {
                    tool_use_id: tc.id.clone(),
                    output: format!("Blocked by PreToolUse hook: {reason}"),
                    is_error: true,
                    tool_name: Some(tc.name.clone()),
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
                return;
            }
        }

        // Permission check (sync — runs before spawning)
        let decision = crate::agent::permission::check_permission(
            &self.state.agent.permission_mode,
            &tc.name,
            &tc.arguments,
            &self.state.agent.allow_list,
            &self.state.agent.deny_list,
            &self.state.agent.session_allows,
            tool_permission,
        );

        if let crate::agent::permission::ToolDecision::Blocked(reason) = decision {
            self.handle_tool_result(crate::agent::tools::ToolResult {
                tool_use_id: tc.id.clone(),
                output: format!("Tool blocked: {reason}"),
                is_error: true,
                tool_name: Some(tc.name.clone()),
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
            return;
        }

        // Snapshot files before modification for checkpoint/rewind + baseline tracking
        if matches!(tc.name.as_str(), "write_file" | "edit_file" | "apply_patch") {
            // Extract file paths from tool arguments and snapshot them
            if let Some(path) = tc
                .arguments
                .get("path")
                .or_else(|| tc.arguments.get("file_path"))
                .or_else(|| tc.arguments.get("filename"))
                .and_then(|v| v.as_str())
            {
                self.state
                    .checkpoints
                    .ensure_snapshotted(std::path::Path::new(path));
                // Capture baseline for net diff tracking (first touch only)
                self.state
                    .file_baselines
                    .entry(path.to_string())
                    .or_insert_with(|| std::fs::read_to_string(path).ok());
            }
            // apply_patch can touch multiple files — extract from diff headers
            if tc.name == "apply_patch"
                && let Some(diff) = tc.arguments.get("diff").and_then(|v| v.as_str())
            {
                for line in diff.lines() {
                    if let Some(rest) = line.strip_prefix("+++ ") {
                        let path = rest.trim().trim_start_matches("b/");
                        if path != "/dev/null" {
                            self.state
                                .checkpoints
                                .ensure_snapshotted(std::path::Path::new(path));
                            self.state
                                .file_baselines
                                .entry(path.to_string())
                                .or_insert_with(|| std::fs::read_to_string(path).ok());
                        }
                    }
                }
            }
        }

        if tc.name.contains(':') {
            // MCP tools — prepare synchronously, execute on background task
            match self
                .state
                .mcp_manager
                .prepare_tool_call(&tc.name, &tc.arguments)
            {
                Ok(prepared) => {
                    let id = tc.id.clone();
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    self.state.tool_exec_pending_rx = Some(rx);
                    tokio::spawn(async move {
                        let mut result = prepared.execute().await;
                        result.tool_use_id = id;
                        let _ = tx.send(result);
                    });
                }
                Err(mut err_result) => {
                    err_result.tool_use_id = tc.id.clone();
                    self.handle_tool_result(err_result);
                }
            }
        } else if tc.name == "diagnostics" || tc.name == "lsp" {
            // LSP tools — execute inline (need &mut lsp_manager)
            let mut result = {
                let State {
                    ref mut agent,
                    ref mut lsp_manager,
                    ref config,
                    ..
                } = self.state;
                match crate::agent::tools::execute_tool(
                    &tc.name,
                    &tc.arguments,
                    &agent.additional_secrets,
                    None,
                    lsp_manager.as_mut(),
                    config.services.as_ref(),
                    config.tools.as_ref().and_then(|t| t.registry.as_ref()),
                    config
                        .tools
                        .as_ref()
                        .and_then(|t| t.deny_commands.as_deref())
                        .unwrap_or(&[]),
                    config.tools.as_ref().and_then(|t| t.executable.as_ref()),
                )
                .await
                {
                    Ok(mut r) => {
                        r.tool_use_id = tc.id.clone();
                        r
                    }
                    Err(e) => crate::agent::tools::ToolResult {
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
            };
            // Run post-tool hooks
            if !result.files_modified.is_empty() {
                let mut ctx = crate::hooks::HookContext {
                    lsp_manager: self.state.lsp_manager.as_mut(),
                };
                self.state.post_tool_hooks.run(&mut result, &mut ctx).await;
            }
            self.handle_tool_result(result);
        } else {
            // Built-in tool — spawn on background tokio task
            let name = tc.name.clone();
            let args = tc.arguments;
            let secrets = self.state.agent.additional_secrets.clone();
            let id = tc.id;
            let services = self.state.config.services.clone();
            let cli_tools = self
                .state
                .config
                .tools
                .as_ref()
                .and_then(|t| t.registry.clone());
            let deny_commands: Vec<String> = self
                .state
                .config
                .tools
                .as_ref()
                .and_then(|t| t.deny_commands.clone())
                .unwrap_or_default();
            let exec_tools = self
                .state
                .config
                .tools
                .as_ref()
                .and_then(|t| t.executable.clone());

            let (tx, rx) = tokio::sync::oneshot::channel();
            self.state.tool_exec_pending_rx = Some(rx);

            tokio::spawn(async move {
                let result = match crate::agent::tools::execute_tool(
                    &name,
                    &args,
                    &secrets,
                    None,
                    None,
                    services.as_ref(),
                    cli_tools.as_ref(),
                    &deny_commands,
                    exec_tools.as_ref(),
                )
                .await
                {
                    Ok(mut r) => {
                        r.tool_use_id = id;
                        r
                    }
                    Err(e) => crate::agent::tools::ToolResult {
                        tool_use_id: id,
                        output: format!("Tool error: {e}"),
                        is_error: true,
                        tool_name: Some(name),
                        file_path: None,
                        files_modified: vec![],
                        lines_added: 0,
                        lines_removed: 0,
                    },
                };
                let _ = tx.send(result);
            });
        }
    }

    /// Process a completed tool result — updates UI placeholder and agent conversation.
    fn handle_tool_result(&mut self, result: crate::agent::tools::ToolResult) {
        // Update the Running placeholder in-place
        let placeholder_idx =
            self.state.tool_exec_running_start + self.state.tool_exec_results.len();
        if let Some(ChatMessage::Tool(tm)) = self.state.chat_messages.get_mut(placeholder_idx) {
            tm.status = if result.is_error {
                ToolStatus::Failed
            } else {
                ToolStatus::Success
            };
            tm.output = Some(result.output.clone());
            tm.file_path = result.file_path.clone();
        }

        // Push result into agent conversation
        self.state
            .agent
            .conversation
            .push(crate::agent::conversation::Message {
                role: crate::agent::conversation::Role::User,
                content: crate::agent::conversation::Content::Blocks(vec![
                    crate::agent::conversation::ContentBlock::ToolResult {
                        tool_use_id: result.tool_use_id.clone(),
                        content: result.output.clone(),
                        is_error: result.is_error,
                    },
                ]),
                tool_call_id: Some(result.tool_use_id.clone()),
            });

        // Fire PostToolUse / PostToolUseFailure lifecycle hooks (fire-and-forget)
        if let Some(ref hooks_config) = self.state.config.hooks {
            let tool_name = result.tool_name.clone().unwrap_or_default();
            if result.is_error && !hooks_config.post_tool_use_failure.is_empty() {
                let hooks = hooks_config.post_tool_use_failure.clone();
                let context = serde_json::json!({
                    "event": "PostToolUseFailure",
                    "tool_name": tool_name,
                    "error": result.output,
                    "session_id": self.state.current_session_id,
                });
                let tool_name_clone = tool_name.clone();
                tokio::spawn(async move {
                    crate::hooks::fire_hooks_for_tool(&hooks, context, &tool_name_clone).await;
                });
            } else if !result.is_error && !hooks_config.post_tool_use.is_empty() {
                let hooks = hooks_config.post_tool_use.clone();
                let context = serde_json::json!({
                    "event": "PostToolUse",
                    "tool_name": tool_name,
                    "tool_output": result.output,
                    "session_id": self.state.current_session_id,
                });
                let tool_name_clone = tool_name.clone();
                tokio::spawn(async move {
                    crate::hooks::fire_hooks_for_tool(&hooks, context, &tool_name_clone).await;
                });
            }
        }

        self.state.tool_exec_results.push(result);
    }

    /// Called after all queued tools have executed — tracks files,
    /// records observations, and continues the agent loop.
    fn finalize_tool_execution(&mut self) {
        let results = std::mem::take(&mut self.state.tool_exec_results);
        self.state.tool_exec_args.clear();

        // Track tool invocation counts + recompute net file diffs from baselines
        for result in &results {
            if let Some(ref tool_name) = result.tool_name {
                *self
                    .state
                    .tool_counts
                    .entry(tool_name.to_string())
                    .or_insert(0) += 1;
                match tool_name.as_str() {
                    "read_file" | "list_directory" => {
                        if let Some(ref path) = result.file_path {
                            let entry = self.state.modified_files.entry(path.clone()).or_default();
                            entry.reads += 1;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Recompute net diffs for all baselined files (baseline vs current on disk)
        for (path, baseline) in &self.state.file_baselines {
            let current = std::fs::read_to_string(path).ok();
            let (additions, deletions) = match (baseline, &current) {
                (Some(old), Some(new)) => crate::tools::write::line_diff(old, new),
                (Some(old), None) => (0, old.lines().count()), // file deleted
                (None, Some(new)) => (new.lines().count(), 0), // file created
                (None, None) => (0, 0),                        // didn't exist, still doesn't
            };
            let entry = self.state.modified_files.entry(path.clone()).or_default();
            entry.additions = additions;
            entry.deletions = deletions;
        }

        // Record observations for memory extraction
        if let Some(ref session_id) = self.state.current_session_id
            && self.state.memory.enabled()
        {
            for result in &results {
                let obs_kind = match result.tool_name.as_deref() {
                    Some("read_file")
                    | Some("glob")
                    | Some("grep")
                    | Some("list_directory")
                    | Some("fetch") => "read",
                    Some("write_file") => "write",
                    Some("edit_file") => "edit",
                    Some("run_command") => "command",
                    Some(name) if name.contains(':') => "mcp",
                    _ => "other",
                };
                let target = result
                    .file_path
                    .as_deref()
                    .unwrap_or_else(|| result.tool_name.as_deref().unwrap_or("unknown"));
                let summary = format!(
                    "{} {}",
                    result.tool_name.as_deref().unwrap_or("unknown"),
                    target,
                );
                let _ = crate::memory::observations::record(
                    self.state.sessions.storage().conn(),
                    session_id,
                    obs_kind,
                    target,
                    &summary,
                );
            }
        }

        // Check budget before auto-continuing
        if self.check_budget_exceeded() {
            return;
        }

        // Continue the agent loop — start next stream
        if let Some(ref provider) = self.provider {
            let tool_defs = self.build_tool_defs();
            self.state
                .agent
                .continue_after_tools(provider.as_ref(), &tool_defs);
        }
    }

    /// Check if session cost has exceeded the configured budget.
    /// If so, pause the agent and show a system message. Returns true if paused.
    fn check_budget_exceeded(&mut self) -> bool {
        let max_cost = self
            .state
            .config
            .behavior
            .as_ref()
            .and_then(|b| b.max_session_cost);
        if let Some(max) = max_cost
            && self.state.session_cost >= max
            && !self.state.budget_paused
        {
            self.state.budget_paused = true;
            self.state.agent.state = AgentState::Idle;
            self.state.chat_messages.push(ChatMessage::System {
                content: format!(
                    "Session budget of ${:.2} reached (spent ${:.2}). Press [c] to continue, [r] to raise limit, [s] to stop.",
                    max, self.state.session_cost
                ),
            });
            return true;
        }
        false
    }

    /// Move the streaming text buffer into the chat display.
    fn flush_assistant_text(&mut self) {
        // Get text from the last assistant message in conversation
        if let Some(msg) = self.state.agent.conversation.messages.last()
            && msg.role == crate::agent::conversation::Role::Assistant
        {
            let text = match &msg.content {
                crate::agent::conversation::Content::Text(t) => t.clone(),
                crate::agent::conversation::Content::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| {
                        if let crate::agent::conversation::ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };
            let text = text.trim().to_string();
            if !text.is_empty() {
                self.state.chat_messages.push(ChatMessage::Assistant {
                    content: text.clone(),
                });
                let t0 = Instant::now();
                self.persist_message("assistant", &text);
                let persist_ms = t0.elapsed().as_millis();
                let t1 = Instant::now();
                self.update_session_meta();
                let meta_ms = t1.elapsed().as_millis();
                if persist_ms > 5 || meta_ms > 5 {
                    tracing::debug!(
                        "flush_assistant_text: persist={persist_ms}ms meta={meta_ms}ms"
                    );
                }
            }
        }
    }

    /// Handle `/memories` — display current memory contents as a system message.
    fn handle_memories_command(&mut self) {
        let ctx = self.state.memory.load_context();
        let mut content = String::new();
        if let Some(ref project) = ctx.project {
            content.push_str("**Project memories** (`.caboose/memory/MEMORY.md`):\n\n");
            content.push_str(project);
            content.push_str("\n\n");
        } else {
            content.push_str("No project memories found.\n\n");
        }
        if let Some(ref global) = ctx.global {
            content.push_str("**Global memories** (`~/.config/caboose/memory/MEMORY.md`):\n\n");
            content.push_str(global);
        } else {
            content.push_str("No global memories found.\n");
        }
        self.state
            .chat_messages
            .push(ChatMessage::System { content });
    }

    /// Handle key presses during skill creation preview phase.
    /// Returns true if the key was consumed.
    fn handle_skill_creation_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        let creation = match &self.state.skill_creation {
            Some(c) => c.clone(),
            None => return false,
        };

        let (content, companions) = match &creation.phase {
            crate::skills::creation::SkillCreationPhase::Preview {
                content,
                companion_files,
            } => (content.clone(), companion_files.clone()),
            _ => return false, // Only handle keys in preview phase
        };

        match key {
            crossterm::event::KeyCode::Char('p') => {
                self.save_created_skill(
                    &creation.name,
                    &content,
                    &companions,
                    crate::skills::creation::SkillScope::Project,
                );
                true
            }
            crossterm::event::KeyCode::Char('g') => {
                self.save_created_skill(
                    &creation.name,
                    &content,
                    &companions,
                    crate::skills::creation::SkillScope::Global,
                );
                true
            }
            crossterm::event::KeyCode::Char('e') => {
                // Edit — return to gathering with feedback prompt
                self.state.skill_creation.as_mut().unwrap().phase =
                    crate::skills::creation::SkillCreationPhase::Gathering;
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Type your feedback to refine the skill:".into(),
                });
                true
            }
            crossterm::event::KeyCode::Char('c') => {
                // Cancel
                self.state.skill_creation = None;
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Skill creation cancelled.".into(),
                });
                true
            }
            _ => false,
        }
    }

    /// Save a created skill to disk and reload.
    fn save_created_skill(
        &mut self,
        name: &str,
        content: &str,
        companions: &[crate::skills::creation::CompanionFile],
        scope: crate::skills::creation::SkillScope,
    ) {
        // Check for existing skill at target
        if let Some(existing_path) = crate::skills::creation::skill_exists(name, scope) {
            self.state.chat_messages.push(ChatMessage::System {
                content: format!("Overwriting existing skill at {}", existing_path.display()),
            });
        }

        match crate::skills::creation::write_skill(name, content, companions, scope) {
            Ok(path) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Skill \"{name}\" saved to {}", path.display()),
                });

                // Reload skills
                let disabled = self
                    .state
                    .config
                    .skills
                    .as_ref()
                    .map(|s| s.disabled.clone())
                    .unwrap_or_default();
                self.state.skills =
                    crate::skills::loader::load_all_skills(std::path::Path::new("."), &disabled);

                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Skill list reloaded ({} skills available). Use /{name} to invoke it.",
                        self.state.skills.len()
                    ),
                });
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to save skill: {e}"),
                });
            }
        }

        // Clear creation state
        self.state.skill_creation = None;
    }

    /// Toggle the currently selected skill's disabled state in the Skills picker.
    fn toggle_skill_disabled(&mut self) {
        let Some(auto) = &self.state.slash_auto else {
            return;
        };
        let filtered = crate::tui::slash_auto::filter_skills(&self.state.skills, &auto.filter);
        let Some(&idx) = filtered.get(auto.selected) else {
            return;
        };
        let skill_name = self.state.skills[idx].name.clone();

        let skills_config = self
            .state
            .config
            .skills
            .get_or_insert_with(Default::default);
        let lower = skill_name.to_lowercase();
        if let Some(pos) = skills_config
            .disabled
            .iter()
            .position(|s| s.to_lowercase() == lower)
        {
            skills_config.disabled.remove(pos);
        } else {
            skills_config.disabled.push(skill_name);
        }

        // Persist to config file
        let project_config_exists = std::path::Path::new(".caboose/config.toml").exists();
        crate::config::save_skills_disabled(&skills_config.disabled, project_config_exists);

        // Reload skills to reflect change
        let disabled = self
            .state
            .config
            .skills
            .as_ref()
            .map(|s| s.disabled.clone())
            .unwrap_or_default();
        self.state.skills =
            crate::skills::loader::load_all_skills(std::path::Path::new("."), &disabled);
    }

    /// Delete the currently selected user skill (not built-in) from disk.
    fn delete_user_skill(&mut self) {
        let Some(auto) = &self.state.slash_auto else {
            return;
        };
        let filtered = crate::tui::slash_auto::filter_skills(&self.state.skills, &auto.filter);
        let Some(&idx) = filtered.get(auto.selected) else {
            return;
        };
        let skill = &self.state.skills[idx];

        // Only user skills (File source) can be deleted
        let path = match &skill.source {
            crate::skills::SkillSource::File(p) => p.clone(),
            crate::skills::SkillSource::Builtin => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Cannot delete built-in skills.".into(),
                });
                return;
            }
        };

        let name = skill.name.clone();

        // Delete the file (or folder for folder-skills)
        if path.is_dir() {
            if std::fs::remove_dir_all(&path).is_err() {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to delete skill folder: {}", path.display()),
                });
                return;
            }
        } else if std::fs::remove_file(&path).is_err() {
            self.state.chat_messages.push(ChatMessage::Error {
                content: format!("Failed to delete skill file: {}", path.display()),
            });
            return;
        }

        self.state.chat_messages.push(ChatMessage::System {
            content: format!("Deleted skill \"{name}\""),
        });

        // Reload skills
        let disabled = self
            .state
            .config
            .skills
            .as_ref()
            .map(|s| s.disabled.clone())
            .unwrap_or_default();
        self.state.skills =
            crate::skills::loader::load_all_skills(std::path::Path::new("."), &disabled);

        // Clamp selection in picker
        if let Some(auto) = self.state.slash_auto.as_mut() {
            let count =
                crate::tui::slash_auto::filtered_skill_count(&self.state.skills, &auto.filter);
            if auto.selected >= count && count > 0 {
                auto.selected = count - 1;
            }
        }
    }

    /// Handle `/create-skill [name] [goal]` — start the skill creation flow.
    ///
    /// Supports both direct (`/create-skill deploy automate deploys`) and
    /// conversational (`/create-skill` → prompts for name → prompts for goal).
    fn handle_create_skill_command(&mut self, args_str: &str) {
        // Always transition to chat screen
        if matches!(self.state.dialog_stack.base, Screen::Home) {
            self.state.dialog_stack.base = Screen::Chat;
            self.state.dialog_stack.clear();
        }

        self.state.chat_messages.push(ChatMessage::User {
            content: if args_str.is_empty() {
                "/create-skill".to_string()
            } else {
                format!("/create-skill {args_str}")
            },
            images: vec![],
        });
        self.state.user_scrolled_up = false;

        let parts: Vec<&str> = args_str.splitn(2, char::is_whitespace).collect();
        let name = parts
            .first()
            .filter(|n| !n.is_empty())
            .map(|n| n.trim().to_lowercase());
        let goal = parts
            .get(1)
            .map(|g| g.trim())
            .filter(|g| !g.is_empty())
            .map(String::from);

        match (name, goal) {
            // Both provided — validate and start immediately
            (Some(name), Some(goal)) => {
                if crate::skills::creation::is_reserved_name(&name) {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!(
                            "'{name}' is a reserved command name. Choose a different name."
                        ),
                    });
                    return;
                }
                self.start_skill_creation(name, goal);
            }
            // Name only — ask for goal
            (Some(name), None) => {
                if crate::skills::creation::is_reserved_name(&name) {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!(
                            "'{name}' is a reserved command name. Choose a different name."
                        ),
                    });
                    return;
                }
                self.state.skill_creation = Some(crate::skills::creation::SkillCreationState {
                    name,
                    goal: String::new(),
                    phase: crate::skills::creation::SkillCreationPhase::AwaitingGoal,
                    question_count: 0,
                });
                self.state.chat_messages.push(ChatMessage::System {
                    content: "What should this skill do? Describe the goal in a sentence or two."
                        .into(),
                });
            }
            // Nothing — ask for name
            _ => {
                self.state.skill_creation = Some(crate::skills::creation::SkillCreationState {
                    name: String::new(),
                    goal: String::new(),
                    phase: crate::skills::creation::SkillCreationPhase::AwaitingName,
                    question_count: 0,
                });
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Let's create a skill! What do you want to name it?".into(),
                });
            }
        }
    }

    /// Spawn parallel planner tasks for Roundhouse mode.
    fn start_roundhouse_planning(&mut self) {
        let session = match self.state.roundhouse_session.as_ref() {
            Some(s) => s,
            None => return,
        };
        let prompt = match session.prompt.clone() {
            Some(p) => p,
            None => return,
        };
        let timeout = session.config.planning_timeout_secs;

        // Get read-only tool subset
        let tools =
            crate::roundhouse::planner::planning_tool_subset(self.state.tools.definitions());

        let (update_tx, update_rx) = tokio::sync::mpsc::unbounded_channel();
        self.state.roundhouse_update_rx = Some(update_rx);

        // Spawn primary planner (index 0)
        if let Ok(primary_provider) = self.state.providers.get_provider(
            Some(&session.primary_provider),
            Some(&session.primary_model),
        ) {
            let tx = update_tx.clone();
            let p = prompt.clone();
            let t = tools.clone();
            tokio::spawn(async move {
                let result = crate::roundhouse::planner::run_planner(
                    primary_provider,
                    p,
                    t,
                    timeout,
                    tx.clone(),
                    0,
                )
                .await;
                let _ = tx.send(crate::roundhouse::PlannerUpdate::PlanComplete {
                    planner_index: 0,
                    result,
                });
            });
        }

        // Spawn secondary planners (index 1, 2, ...)
        let secondaries: Vec<(usize, String, String)> = session
            .secondaries
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.provider_name.clone(), s.model_name.clone()))
            .collect();

        for (i, provider_name, model_name) in secondaries {
            if let Ok(provider) = self
                .state
                .providers
                .get_provider(Some(&provider_name), Some(&model_name))
            {
                let tx = update_tx.clone();
                let p = prompt.clone();
                let t = tools.clone();
                let idx = i + 1;
                tokio::spawn(async move {
                    let result = crate::roundhouse::planner::run_planner(
                        provider,
                        p,
                        t,
                        timeout,
                        tx.clone(),
                        idx,
                    )
                    .await;
                    let _ = tx.send(crate::roundhouse::PlannerUpdate::PlanComplete {
                        planner_index: idx,
                        result,
                    });
                });
            } else {
                // Mark as failed if we can't create the provider
                if let Some(ref mut session) = self.state.roundhouse_session
                    && let Some(s) = session.secondaries.get_mut(i)
                {
                    s.status = crate::roundhouse::PlannerStatus::Failed(format!(
                        "Could not create provider '{provider_name}'"
                    ));
                }
            }
        }
    }

    /// Handle `/roundhouse execute` and `/roundhouse cancel` subcommands.
    fn handle_roundhouse_subcommand(&mut self, sub: &str) {
        match sub {
            "execute" => {
                if let Some(ref session) = self.state.roundhouse_session {
                    if session.phase == crate::roundhouse::RoundhousePhase::Reviewing {
                        let plan = session.synthesized_plan.clone().unwrap_or_default();
                        let msg = format!(
                            "Execute the following implementation plan now. Start implementing immediately — read the relevant files, make the code changes, and run any commands specified. Do not just describe what you would do; actually do it step by step using your tools.\n\n{plan}"
                        );
                        self.state.roundhouse_session.as_mut().unwrap().phase =
                            crate::roundhouse::RoundhousePhase::Executing;
                        // Queue the plan for the agent to execute
                        self.state.message_queue.push_back(msg);
                    } else {
                        self.state.chat_messages.push(ChatMessage::System {
                            content: format!(
                                "Cannot execute: roundhouse is in {:?} phase (expected Reviewing).",
                                session.phase
                            ),
                        });
                    }
                } else {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "No active roundhouse session.".to_string(),
                    });
                }
            }
            "cancel" => {
                if self.state.roundhouse_session.is_some() {
                    self.state.roundhouse_session = None;
                    self.state.roundhouse_update_rx = None;
                    self.state.roundhouse_synthesis_rx = None;
                    self.state.roundhouse_model_add = false;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "Roundhouse cancelled.".to_string(),
                    });
                } else {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "No active roundhouse session.".to_string(),
                    });
                }
            }
            "clear" => {
                if self.state.roundhouse_session.is_some() {
                    self.state.roundhouse_session = None;
                    self.state.roundhouse_update_rx = None;
                    self.state.roundhouse_synthesis_rx = None;
                    self.state.roundhouse_model_add = false;
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "Roundhouse session cleared.".to_string(),
                    });
                } else {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: "No active roundhouse session.".to_string(),
                    });
                }
            }
            other => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Unknown roundhouse subcommand: `{other}`. Use `execute`, `cancel`, or `clear`."
                    ),
                });
            }
        }
    }

    /// Send all collected plans to the primary provider for synthesis.
    fn start_roundhouse_synthesis(&mut self) {
        let session = match self.state.roundhouse_session.as_ref() {
            Some(s) => s,
            None => return,
        };
        let plans = session.successful_plans();
        if plans.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "No successful plans to synthesize.".to_string(),
            });
            if let Some(ref mut s) = self.state.roundhouse_session {
                s.phase = crate::roundhouse::RoundhousePhase::Cancelled;
            }
            return;
        }

        let prompt = session.prompt.clone().unwrap_or_default();
        let system = crate::roundhouse::planner::synthesis_system_prompt(&prompt, &plans);

        let provider = match self.state.providers.get_provider(
            Some(&session.primary_provider),
            Some(&session.primary_model),
        ) {
            Ok(p) => p,
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to create provider for synthesis: {e}"),
                });
                return;
            }
        };

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Build messages: system prompt as system message, then user asks to synthesize
        let messages = vec![
            crate::provider::Message {
                role: "system".to_string(),
                content: serde_json::json!(system),
            },
            crate::provider::Message {
                role: "user".to_string(),
                content: serde_json::json!(
                    "Synthesize the plans above into a single unified implementation plan."
                ),
            },
        ];

        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = provider.stream(&messages, &[]);

            while let Some(event_result) = stream.next().await {
                match event_result {
                    Ok(crate::provider::StreamEvent::TextDelta(delta)) => {
                        let _ = tx.send(delta);
                    }
                    Ok(crate::provider::StreamEvent::Error(_))
                    | Ok(crate::provider::StreamEvent::ProviderError { .. })
                    | Ok(crate::provider::StreamEvent::Done { .. }) => {
                        break;
                    }
                    _ => {}
                }
            }
            // tx drops here, signalling completion
        });

        self.state.roundhouse_synthesis_rx = Some(rx);
    }

    /// Start the LLM-guided skill creation after name and goal are known.
    fn start_skill_creation(&mut self, name: String, goal: String) {
        if !self.require_provider() {
            return;
        }

        self.state.skill_creation = Some(crate::skills::creation::SkillCreationState {
            name: name.clone(),
            goal: goal.clone(),
            phase: crate::skills::creation::SkillCreationPhase::Gathering,
            question_count: 0,
        });

        self.state.chat_messages.push(ChatMessage::System {
            content: format!(
                "Creating skill \"{name}\" — the assistant will ask a few questions to refine it."
            ),
        });

        // Inject creation system prompt and send initial message
        let creation_prompt = crate::skills::creation::system_prompt(&name, &goal);
        let initial_msg = format!(
            "{creation_prompt}\n\nI want to create a skill called \"{name}\". Goal: {goal}"
        );

        let tool_defs = self.build_tool_defs();

        self.state.agent.send_message(
            initial_msg,
            self.provider.as_ref().unwrap().as_ref(),
            &tool_defs,
        );
    }

    /// Handle `/init` — scan repo and generate CABOOSE.md via LLM.
    ///
    /// Non-blocking: spawns the streaming task in the background.
    /// The main loop polls `state.init_rx` for events.
    fn handle_init_command(&mut self) {
        // Transition to chat screen first so any errors are visible
        if matches!(self.state.dialog_stack.base, Screen::Home) {
            self.state.dialog_stack.base = Screen::Chat;
            self.state.dialog_stack.clear();
        }

        // Show the user's command in the chat
        self.state.chat_messages.push(ChatMessage::User {
            content: "/init".to_string(),
            images: vec![],
        });
        self.state.user_scrolled_up = false;

        if !self.require_provider() {
            return;
        }

        // 1. Scan
        self.state.chat_messages.push(ChatMessage::System {
            content: "Scanning repository...".to_string(),
        });

        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let ctx = crate::init::scanner::scan(&cwd);

        // Store init metadata for when generation completes
        self.state.init_had_existing = ctx.existing_caboose.is_some();
        self.state.init_old_lines = ctx.existing_caboose.as_ref().map(|c| c.lines().count());
        self.state.init_write_root = ctx.root.clone();
        self.state.init_text.clear();

        // 2. Build prompt and spawn background streaming task
        let user_prompt = crate::init::handler::build_prompt(&ctx);
        let provider = self.provider.as_ref().unwrap();

        self.state.chat_messages.push(ChatMessage::System {
            content: "Generating CABOOSE.md...".to_string(),
        });

        let messages = vec![crate::provider::Message {
            role: "user".to_string(),
            content: serde_json::json!(user_prompt),
        }];
        let stream = provider.stream(&messages, &[]); // no tools

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.state.init_rx = Some(rx);

        // Spawn non-blocking — events polled in main loop
        tokio::spawn(async move {
            use futures::StreamExt;
            let mut stream = stream;
            while let Some(event) = stream.next().await {
                let init_event = match event {
                    Ok(crate::provider::StreamEvent::TextDelta(text)) => {
                        crate::init::handler::InitEvent::TextDelta(text)
                    }
                    Ok(crate::provider::StreamEvent::Done {
                        input_tokens,
                        output_tokens,
                        ..
                    }) => crate::init::handler::InitEvent::Done {
                        input_tokens: input_tokens.unwrap_or(0),
                        output_tokens: output_tokens.unwrap_or(0),
                    },
                    Ok(crate::provider::StreamEvent::Error(e)) => {
                        crate::init::handler::InitEvent::Error(format!(
                            "Failed to generate CABOOSE.md: {e}"
                        ))
                    }
                    Err(e) => crate::init::handler::InitEvent::Error(format!("Stream error: {e}")),
                    _ => continue,
                };
                if tx.send(init_event).is_err() {
                    break; // receiver dropped
                }
            }
        });
    }

    /// Finalize /init generation: write file and show result.
    fn finalize_init(&mut self) {
        let generated = std::mem::take(&mut self.state.init_text);
        let had_existing = self.state.init_had_existing;
        let old_lines = self.state.init_old_lines;
        let write_root = std::mem::take(&mut self.state.init_write_root);

        if generated.trim().is_empty() {
            self.state.chat_messages.push(ChatMessage::Error {
                content: "LLM returned empty response".to_string(),
            });
            return;
        }

        // Persist the generated content as a collapsible Assistant message
        self.state.chat_messages.push(ChatMessage::Assistant {
            content: generated.trim().to_string(),
        });

        match crate::init::handler::write_caboose_md(&write_root, generated.trim()) {
            Ok((path, line_count)) => {
                let msg = if had_existing {
                    format!(
                        "Wrote {} ({} lines, was {})",
                        path.display(),
                        line_count,
                        old_lines.unwrap_or(0),
                    )
                } else {
                    format!("Wrote {} ({line_count} lines)", path.display())
                };
                self.state
                    .chat_messages
                    .push(ChatMessage::System { content: msg });
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to write CABOOSE.md: {e}"),
                });
            }
        }
    }

    /// Handle `/forget` — list memory entries for removal.
    fn handle_forget_command(&mut self) {
        let ctx = self.state.memory.load_context();
        let mut lines = Vec::new();
        if let Some(ref project) = ctx.project {
            for line in project.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                    lines.push(("project", trimmed.to_string()));
                }
            }
        }
        if let Some(ref global) = ctx.global {
            for line in global.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
                    lines.push(("global", trimmed.to_string()));
                }
            }
        }
        if lines.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "No memories to forget.".to_string(),
            });
        } else {
            let mut content = String::from("Current memories:\n\n");
            for (i, (scope, line)) in lines.iter().enumerate() {
                content.push_str(&format!("{}. [{}] {}\n", i + 1, scope, line));
            }
            content.push_str("\nTell me which memory to remove (by number or description).");
            self.state
                .chat_messages
                .push(ChatMessage::System { content });
        }
    }

    /// Open the settings picker with current config values.
    fn open_settings_picker(&mut self) {
        let memory_config = self.state.config.memory.clone().unwrap_or_default();
        let items = vec![
            crate::tui::slash_auto::SettingsItem {
                key: "memory.enabled".to_string(),
                label: "Memory".to_string(),
                value: if memory_config.enabled {
                    "on".to_string()
                } else {
                    "off".to_string()
                },
                kind: crate::tui::slash_auto::SettingsKind::Toggle,
            },
            crate::tui::slash_auto::SettingsItem {
                key: "memory.auto_extract".to_string(),
                label: "Auto-extract memories".to_string(),
                value: if memory_config.auto_extract {
                    "on".to_string()
                } else {
                    "off".to_string()
                },
                kind: crate::tui::slash_auto::SettingsKind::Toggle,
            },
            {
                let presets = ["off", "$1", "$2", "$5", "$10", "$25", "$50", "$100"];
                let current_value = self
                    .state
                    .config
                    .behavior
                    .as_ref()
                    .and_then(|b| b.max_session_cost)
                    .map(|v| {
                        // Use integer format for whole numbers, decimal otherwise
                        if v == v.floor() {
                            format!("${:.0}", v)
                        } else {
                            format!("${:.2}", v)
                        }
                    })
                    .unwrap_or_else(|| "off".to_string());

                let mut choices: Vec<String> = presets.iter().map(|s| s.to_string()).collect();

                // If current value is custom (not in presets), prepend it
                let is_custom = !presets.contains(&current_value.as_str());
                let display_value = if is_custom {
                    let custom_label = format!("{} (custom)", current_value);
                    choices.insert(0, custom_label.clone());
                    custom_label
                } else {
                    current_value
                };

                crate::tui::slash_auto::SettingsItem {
                    key: "behavior.max_session_cost".to_string(),
                    label: "Session budget".to_string(),
                    value: display_value,
                    kind: crate::tui::slash_auto::SettingsKind::Choice(choices),
                }
            },
            crate::tui::slash_auto::SettingsItem {
                key: "theme".to_string(),
                label: "Theme".to_string(),
                value: crate::tui::theme::active_variant().label().to_string(),
                kind: crate::tui::slash_auto::SettingsKind::Choice(
                    crate::tui::theme::ThemeVariant::ALL
                        .iter()
                        .map(|v| v.label().to_string())
                        .collect(),
                ),
            },
            {
                let mut migrate_choices = vec!["(none)".to_string()];
                for platform in crate::migrate::SourcePlatform::all() {
                    migrate_choices.push(platform.label().to_string());
                }
                crate::tui::slash_auto::SettingsItem {
                    key: "migrate".to_string(),
                    label: "Migrate from...".to_string(),
                    value: "(none)".to_string(),
                    kind: crate::tui::slash_auto::SettingsKind::Choice(migrate_choices),
                }
            },
        ];
        self.state.slash_auto = Some(crate::tui::slash_auto::SlashAutoState::with_settings(items));
        self.state.input.clear();
    }

    /// Recompute `modified_files` diffs from `file_baselines` vs current files on disk.
    /// Called after rewind restores files so the sidebar shows accurate counts.
    fn recompute_modified_files(&mut self) {
        // Clear old diff counts (keep read counts intact)
        for entry in self.state.modified_files.values_mut() {
            entry.additions = 0;
            entry.deletions = 0;
        }
        // Recompute from baselines
        for (path, baseline) in &self.state.file_baselines {
            let current = std::fs::read_to_string(path).ok();
            let (additions, deletions) = match (baseline, &current) {
                (Some(old), Some(new)) => crate::tools::write::line_diff(old, new),
                (Some(old), None) => (0, old.lines().count()),
                (None, Some(new)) => (new.lines().count(), 0),
                (None, None) => (0, 0),
            };
            let entry = self.state.modified_files.entry(path.clone()).or_default();
            entry.additions = additions;
            entry.deletions = deletions;
        }
        // Remove entries with zero activity
        self.state
            .modified_files
            .retain(|_, v| v.additions > 0 || v.deletions > 0 || v.reads > 0);
    }

    /// Open the rewind picker with current checkpoints.
    fn open_rewind_picker(&mut self) {
        let now = std::time::Instant::now();
        // Filter to checkpoints that actually modified files
        let items: Vec<(u32, String, String, usize)> = self
            .state
            .checkpoints
            .list()
            .iter()
            .filter(|cp| !cp.files.is_empty())
            .map(|cp| {
                let elapsed = now.duration_since(cp.timestamp);
                let age = if elapsed.as_secs() < 60 {
                    format!("{}s ago", elapsed.as_secs())
                } else {
                    format!("{}m ago", elapsed.as_secs() / 60)
                };
                (cp.id, cp.prompt_preview.clone(), age, cp.files.len())
            })
            .collect();
        if items.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "No checkpoints with file changes to rewind to.".into(),
            });
            self.state.input.clear();
            return;
        }
        self.state.slash_auto = Some(crate::tui::slash_auto::SlashAutoState::with_checkpoints(
            items,
        ));
        self.state.input.clear();
    }

    /// Handle the /handoff command — build summary and prompt for new session.
    async fn handle_handoff_command(&mut self, args: &str) {
        // Collect user messages
        let user_msgs: Vec<&str> = self
            .state
            .chat_messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();

        // Convert modified_files to handoff format
        let modified: std::collections::HashMap<String, crate::skills::handoff::HandoffFileStats> =
            self.state
                .modified_files
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        crate::skills::handoff::HandoffFileStats {
                            additions: v.additions,
                            deletions: v.deletions,
                        },
                    )
                })
                .collect();

        // Collect open tasks from the last TaskOutline
        let open_tasks: Vec<&str> = self
            .state
            .chat_messages
            .iter()
            .rev()
            .find_map(|m| match m {
                ChatMessage::TaskOutline(outline) => Some(outline),
                _ => None,
            })
            .map(|outline| {
                outline
                    .tasks
                    .iter()
                    .filter(|t| !matches!(t.status, TaskStatus::Completed | TaskStatus::Cancelled))
                    .map(|t| t.content.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let ctx = crate::skills::handoff::HandoffContext {
            session_id: self.state.current_session_id.as_deref(),
            session_title: self.state.session_title.as_deref(),
            provider_name: Some(self.state.active_provider_name.as_str()),
            model_name: Some(self.state.active_model_name.as_str()),
            turn_count: self.state.agent.turn_count,
            user_messages: user_msgs,
            modified_files: &modified,
            tool_counts: &self.state.tool_counts,
            open_tasks,
            focus: if args.is_empty() { None } else { Some(args) },
        };

        let summary = crate::skills::handoff::build_handoff_summary(&ctx);

        // Display summary as system message
        self.state.chat_messages.push(ChatMessage::System {
            content: summary.clone(),
        });
        self.persist_message("system", &summary);

        // Store pending handoff for confirmation
        self.state.pending_handoff = Some(summary);

        // Show confirmation prompt
        self.state.chat_messages.push(ChatMessage::System {
            content: "Handoff ready. Start new session with this context? [y]es / [n]o".into(),
        });
    }

    /// Run end-of-session memory extraction if enabled and there are enough observations.
    async fn extract_session_memories(&mut self) {
        let memory_config = self.state.config.memory.clone().unwrap_or_default();
        if !memory_config.enabled || !memory_config.auto_extract {
            return;
        }
        let session_id = match &self.state.current_session_id {
            Some(id) => id.clone(),
            None => return,
        };

        // Check observation count
        let count = crate::memory::observations::count_for_session(
            self.state.sessions.storage().conn(),
            &session_id,
        )
        .unwrap_or(0);

        if count < crate::memory::extraction::MIN_OBSERVATIONS {
            return;
        }

        // Load observations
        let observations = match crate::memory::observations::for_session(
            self.state.sessions.storage().conn(),
            &session_id,
        ) {
            Ok(obs) => obs,
            Err(_) => return,
        };

        // Load current memory
        let memory_ctx = self.state.memory.load_context();

        // Build extraction prompt
        let prompt = crate::memory::extraction::build_extraction_prompt(
            &observations,
            memory_ctx.project.as_deref(),
        );

        // Send to provider (non-streaming, one-shot)
        if let Some(ref provider) = self.provider {
            let messages = vec![crate::provider::Message {
                role: "user".to_string(),
                content: serde_json::json!(prompt),
            }];

            // Collect stream into response
            use tokio_stream::StreamExt;
            let mut response = String::new();
            let mut stream = provider.stream(&messages, &[]);
            while let Some(event) = stream.next().await {
                if let Ok(crate::provider::StreamEvent::TextDelta(text)) = event {
                    response.push_str(&text);
                }
            }

            // Parse and append
            if let Some(new_lines) = crate::memory::extraction::parse_extraction_response(&response)
            {
                let memory_path = self.state.memory.project_dir().join("MEMORY.md");
                if let Err(e) =
                    crate::memory::extraction::append_to_memory_file(&memory_path, &new_lines)
                {
                    tracing::warn!("Failed to append memories: {e}");
                }
            }
        }

        // Prune old observations
        let _ = crate::memory::observations::prune(
            self.state.sessions.storage().conn(),
            memory_config.observation_retention_days,
        );
    }

    async fn handle_circuit_command(&mut self, args: &str) {
        // /circuit stop <id>
        if let Some(id) = args.strip_prefix("stop ") {
            let id = id.trim();
            if id == "all" || id == "-all" {
                let count = self.state.circuit_manager.active_count();
                self.state.circuit_manager.stop_all();
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Stopped {} circuit(s).", count),
                });
            } else if self.state.circuit_manager.stop_circuit(id) {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Circuit {} stopped.", id),
                });
            } else {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Circuit {} not found.", id),
                });
            }
            return;
        }

        // /circuit stop-all
        if args == "stop-all" {
            let count = self.state.circuit_manager.active_count();
            self.state.circuit_manager.stop_all();
            self.state.chat_messages.push(ChatMessage::System {
                content: format!("Stopped {} circuit(s).", count),
            });
            return;
        }

        // /circuit [--persist] <interval> "prompt"
        match parse_circuit_args(args) {
            Some((persist, interval_secs, prompt)) => {
                let kind = if persist {
                    crate::circuits::CircuitKind::Persistent
                } else {
                    crate::circuits::CircuitKind::InSession
                };
                let _ = self.create_circuit(&prompt, interval_secs, kind).await;
            }
            None => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: "Usage: /circuit [--persist] <interval> \"<prompt>\"\nExamples: /circuit 5m \"check build\" | /circuit --persist 10m \"watch CI\"".to_string(),
                });
            }
        }
    }

    /// Create a circuit and return its ID on success, or None on failure.
    async fn create_circuit(
        &mut self,
        prompt: &str,
        interval_secs: u64,
        kind: crate::circuits::CircuitKind,
    ) -> Option<String> {
        let ts = chrono::Utc::now().timestamp_millis() as u64;
        let mut id = format!("c-{:x}", ts % 0x1000000);
        // Ensure uniqueness against existing circuits
        let mut counter = 1u64;
        while self.state.circuit_manager.circuits.iter().any(|h| h.circuit.id == id) {
            id = format!("c-{:x}", (ts + counter) % 0x1000000);
            counter += 1;
        }
        let circuit = crate::circuits::Circuit {
            id: id.clone(),
            prompt: prompt.to_string(),
            interval_secs,
            provider: self.state.active_provider_name.clone(),
            model: self.state.active_model_name.clone(),
            permission_mode: "plan".to_string(),
            kind,
            status: crate::circuits::CircuitStatus::Active,
            last_run: None,
            next_run: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            total_cost: 0.0,
            run_count: 0,
        };

        if let Err(e) = self.state.circuit_manager.start_circuit(circuit) {
            self.state.chat_messages.push(ChatMessage::Error {
                content: format!("Failed to start circuit: {}", e),
            });
            return None;
        }

        self.state.chat_messages.push(ChatMessage::System {
            content: format!(
                "Circuit started: \"{}\" every {}",
                prompt,
                format_duration(interval_secs)
            ),
        });
        Some(id)
    }

    async fn handle_watch_command(&mut self, args: &str) {
        // /watch pr <number> [--persist]
        // /watch mr <number> [--persist]
        let rest = if let Some(r) = args
            .strip_prefix("pr ")
            .or_else(|| args.strip_prefix("mr "))
        {
            r
        } else {
            self.state.chat_messages.push(ChatMessage::Error {
                content: "Usage: /watch pr <number> [--persist]".to_string(),
            });
            return;
        };

        let parts: Vec<&str> = rest.split_whitespace().collect();
        let pr_number = match parts.first().and_then(|s| s.parse::<u32>().ok()) {
            Some(n) => n,
            None => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: "Usage: /watch pr <number> [--persist]".to_string(),
                });
                return;
            }
        };
        let persist = parts.contains(&"--persist");

        self.create_watcher(pr_number, persist).await;
    }

    async fn create_watcher(&mut self, pr_number: u32, persist: bool) {
        let interval_secs = 180; // 3 minutes
        let prompt = format!(
            "Check the status of PR/MR #{pr_number}. Use the check_ci tool and report: is CI passing, failing, or pending? Is the PR merged or closed?"
        );

        let kind = if persist {
            crate::circuits::CircuitKind::Persistent
        } else {
            crate::circuits::CircuitKind::InSession
        };

        if let Some(circuit_id) = self.create_circuit(&prompt, interval_secs, kind).await {
            let watcher = crate::scm::watcher::Watcher {
                circuit_id,
                pr_number,
                title: None,
                last_status: crate::scm::watcher::WatcherStatus::Unknown,
            };
            self.state.active_watchers.push(watcher);
            self.state.chat_messages.push(ChatMessage::System {
                content: format!("Watching PR/MR #{pr_number} — updates every 3 minutes."),
            });
        }
    }
}

/// Parse "5m" → 300, "30s" → 30, "1h" → 3600
fn parse_interval(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('s') {
        n.parse().ok()
    } else if let Some(n) = s.strip_suffix('m') {
        n.parse::<u64>().ok().map(|n| n * 60)
    } else if let Some(n) = s.strip_suffix('h') {
        n.parse::<u64>().ok().map(|n| n * 3600)
    } else {
        None
    }
}

/// Format seconds back to human-readable: 300 → "5m", 3600 → "1h", 90 → "1m 30s"
fn format_duration(secs: u64) -> String {
    if secs >= 3600 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs >= 60 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

/// Parse circuit command args: "[--persist] <interval> <prompt>"
fn parse_circuit_args(args: &str) -> Option<(bool, u64, String)> {
    let args = args.trim();
    let persist = args.starts_with("--persist");
    let rest = if persist {
        args.strip_prefix("--persist").unwrap().trim()
    } else {
        args
    };

    // First token is interval
    let space = rest.find(' ')?;
    let interval_str = &rest[..space];
    let interval = parse_interval(interval_str)?;

    // Rest is prompt (strip quotes if present)
    let prompt = rest[space..].trim();
    let prompt = prompt.trim_matches('"').trim_matches('\'').trim();
    if prompt.is_empty() {
        return None;
    }

    Some((persist, interval, prompt.to_string()))
}

/// Parse task-like patterns from assistant text output.
/// Recognizes markdown checklists and numbered lists with status markers.
fn parse_tasks_from_text(text: &str) -> Option<TaskOutline> {
    let mut tasks = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Markdown checklist: - [x] Done, - [ ] Pending
        if let Some(rest) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
        {
            tasks.push(Task {
                content: rest.trim().to_string(),
                active_form: rest.trim().to_string(),
                status: TaskStatus::Completed,
            });
        } else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            tasks.push(Task {
                content: rest.trim().to_string(),
                active_form: rest.trim().to_string(),
                status: TaskStatus::Pending,
            });
        }
        // Numbered list: 1. [DONE] Task, 2. [IN PROGRESS] Task, 3. Task
        else if let Some(after_dot) = trimmed.split_once(". ").and_then(|(num, rest)| {
            if num.chars().all(|c| c.is_ascii_digit()) && !num.is_empty() {
                Some(rest)
            } else {
                None
            }
        }) {
            if let Some(rest) = after_dot
                .strip_prefix("[DONE] ")
                .or_else(|| after_dot.strip_prefix("[done] "))
            {
                tasks.push(Task {
                    content: rest.trim().to_string(),
                    active_form: rest.trim().to_string(),
                    status: TaskStatus::Completed,
                });
            } else if let Some(rest) = after_dot
                .strip_prefix("[IN PROGRESS] ")
                .or_else(|| after_dot.strip_prefix("[in progress] "))
            {
                tasks.push(Task {
                    content: rest.trim().to_string(),
                    active_form: rest.trim().to_string(),
                    status: TaskStatus::InProgress,
                });
            } else if let Some(rest) = after_dot
                .strip_prefix("[CANCELLED] ")
                .or_else(|| after_dot.strip_prefix("[cancelled] "))
            {
                tasks.push(Task {
                    content: rest.trim().to_string(),
                    active_form: rest.trim().to_string(),
                    status: TaskStatus::Cancelled,
                });
            } else {
                tasks.push(Task {
                    content: after_dot.trim().to_string(),
                    active_form: after_dot.trim().to_string(),
                    status: TaskStatus::Pending,
                });
            }
        }
    }

    if tasks.len() >= 2 {
        Some(TaskOutline { tasks })
    } else {
        None
    }
}

#[cfg(test)]
mod task_text_parse_tests {
    use super::*;

    #[test]
    fn parse_markdown_checklist() {
        let text =
            "Here's what I'll do:\n- [x] Read the file\n- [ ] Edit the code\n- [ ] Run tests";
        let outline = parse_tasks_from_text(text).unwrap();
        assert_eq!(outline.tasks.len(), 3);
        assert_eq!(outline.tasks[0].status, TaskStatus::Completed);
        assert_eq!(outline.tasks[1].status, TaskStatus::Pending);
    }

    #[test]
    fn parse_numbered_list_with_status() {
        let text = "Tasks:\n1. [DONE] Setup project\n2. [IN PROGRESS] Write code\n3. Run tests";
        let outline = parse_tasks_from_text(text).unwrap();
        assert_eq!(outline.tasks.len(), 3);
        assert_eq!(outline.tasks[0].status, TaskStatus::Completed);
        assert_eq!(outline.tasks[1].status, TaskStatus::InProgress);
        assert_eq!(outline.tasks[2].status, TaskStatus::Pending);
    }

    #[test]
    fn single_item_returns_none() {
        let text = "- [ ] Only one task";
        assert!(parse_tasks_from_text(text).is_none());
    }

    #[test]
    fn no_tasks_returns_none() {
        let text = "Just some regular text with no task patterns.";
        assert!(parse_tasks_from_text(text).is_none());
    }
}

#[cfg(test)]
mod task_outline_tests {
    use super::*;

    #[test]
    fn task_outline_from_json() {
        let json = serde_json::json!({
            "todos": [
                {"content": "Read config", "active_form": "Reading config", "status": "completed"},
                {"content": "Write handler", "active_form": "Writing handler", "status": "in_progress"},
                {"content": "Run tests", "active_form": "Running tests", "status": "pending"},
                {"content": "Old task", "active_form": "Old task", "status": "cancelled"}
            ]
        });
        let outline = TaskOutline::from_tool_input(&json).unwrap();
        assert_eq!(outline.tasks.len(), 4);
        assert_eq!(outline.tasks[0].status, TaskStatus::Completed);
        assert_eq!(outline.tasks[1].status, TaskStatus::InProgress);
        assert_eq!(outline.tasks[1].active_form, "Writing handler");
        assert_eq!(outline.tasks[2].status, TaskStatus::Pending);
        assert_eq!(outline.tasks[3].status, TaskStatus::Cancelled);
    }

    #[test]
    fn task_outline_to_json_roundtrip() {
        let outline = TaskOutline {
            tasks: vec![Task {
                content: "Do thing".into(),
                active_form: "Doing thing".into(),
                status: TaskStatus::Pending,
            }],
        };
        let json = outline.to_json();
        let restored = TaskOutline::from_tool_input(&json).unwrap();
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.tasks[0].content, "Do thing");
    }

    #[test]
    fn task_outline_cancelled_roundtrip() {
        let outline = TaskOutline {
            tasks: vec![
                Task {
                    content: "Done".into(),
                    active_form: "Done".into(),
                    status: TaskStatus::Completed,
                },
                Task {
                    content: "Skipped".into(),
                    active_form: "Skipped".into(),
                    status: TaskStatus::Cancelled,
                },
            ],
        };
        let json = outline.to_json();
        let restored = TaskOutline::from_tool_input(&json).unwrap();
        assert_eq!(restored.tasks[1].status, TaskStatus::Cancelled);
    }

    #[test]
    fn task_outline_empty_returns_error() {
        let json = serde_json::json!({"todos": []});
        assert!(TaskOutline::from_tool_input(&json).is_err());
    }

    #[test]
    fn task_outline_serializes_for_storage() {
        let outline = TaskOutline {
            tasks: vec![Task {
                content: "Do X".into(),
                active_form: "Doing X".into(),
                status: TaskStatus::InProgress,
            }],
        };
        let json = outline.to_json().to_string();
        let restored: serde_json::Value = serde_json::from_str(&json).unwrap();
        let outline2 = TaskOutline::from_tool_input(&restored).unwrap();
        assert_eq!(outline2.tasks[0].content, "Do X");
        assert_eq!(outline2.tasks[0].status, TaskStatus::InProgress);
    }
}

#[cfg(test)]
mod circuit_parse_tests {
    use super::*;

    #[test]
    fn parse_interval_seconds() {
        assert_eq!(parse_interval("30s"), Some(30));
    }

    #[test]
    fn parse_interval_minutes() {
        assert_eq!(parse_interval("5m"), Some(300));
    }

    #[test]
    fn parse_interval_hours() {
        assert_eq!(parse_interval("1h"), Some(3600));
    }

    #[test]
    fn parse_interval_invalid() {
        assert_eq!(parse_interval("abc"), None);
        assert_eq!(parse_interval(""), None);
    }

    #[test]
    fn parse_circuit_args_basic() {
        let (persist, interval, prompt) = parse_circuit_args("5m \"check build\"").unwrap();
        assert!(!persist);
        assert_eq!(interval, 300);
        assert_eq!(prompt, "check build");
    }

    #[test]
    fn parse_circuit_args_persist() {
        let (persist, interval, prompt) = parse_circuit_args("--persist 10m \"watch CI\"").unwrap();
        assert!(persist);
        assert_eq!(interval, 600);
        assert_eq!(prompt, "watch CI");
    }

    #[test]
    fn parse_circuit_args_no_quotes() {
        let (_, _, prompt) = parse_circuit_args("5m check build status").unwrap();
        assert_eq!(prompt, "check build status");
    }

    #[test]
    fn parse_circuit_args_missing_prompt() {
        assert!(parse_circuit_args("5m").is_none());
        assert!(parse_circuit_args("5m \"\"").is_none());
    }

    #[test]
    fn parse_circuit_args_bad_interval() {
        assert!(parse_circuit_args("abc \"prompt\"").is_none());
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(300), "5m");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3600), "1h");
    }

    #[test]
    fn format_duration_mixed() {
        assert_eq!(format_duration(90), "1m 30s");
    }
}
