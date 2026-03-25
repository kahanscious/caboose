use super::*;

/// UI-visible state, separated so it can be borrowed independently from Terminal.
pub struct State {
    pub config: Config,
    pub dialog_stack: DialogStack,
    pub input: crate::tui::input_buffer::InputBuffer,
    pub should_quit: bool,
    /// Set on first ctrl+c; second ctrl+c within 2s actually quits.
    pub quit_first_press: Option<Instant>,
    /// Set on first ctrl+c in Roundhouse; second ctrl+c within 2s cancels session.
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
    /// Current sidebar width in columns (user-resizable via drag).
    pub sidebar_width: u16,
    /// Whether keyboard focus is in the sidebar (agents section navigation).
    pub sidebar_focused: bool,
    /// Active sidebar resize drag (column where drag started).
    pub sidebar_drag: Option<u16>,
    /// Index of the focused tool message in chat_messages (for expand/collapse navigation).
    pub focused_tool: Option<usize>,
    /// Inline slash autocomplete state — active when input starts with `/`.
    pub slash_auto: Option<crate::tui::slash_auto::SlashAutoState>,
    /// @file autocomplete state — active when input contains `@` prefix.
    pub file_auto: Option<crate::tui::file_auto::FileAutoState>,
    /// All loaded skills (built-in + user).
    pub skills: Vec<crate::skills::Skill>,
    /// All loaded custom agent definitions.
    pub agent_definitions: Vec<crate::agents::AgentDefinition>,
    /// Current active session ID (for persistence).
    pub current_session_id: Option<String>,
    /// Memory store for cross-session persistence.
    pub memory: caboose_core::memory::MemoryStore,
    /// Input history for Up/Down browsing across sessions.
    pub history: crate::tui::input_history::InputHistory,
    /// Timestamp of the most recent text insertion into the composer.
    /// Used to detect multiline paste streams that arrive as raw key events.
    pub last_text_input_at: Option<Instant>,
    /// Approximate number of characters inserted in the current rapid-input burst.
    pub rapid_input_streak: usize,
    /// Short grace window after paste-like bursts so chunked Enter events don't submit.
    pub paste_like_mode_until: Option<Instant>,
    /// Messages expanded past truncation threshold.
    pub expanded_messages: std::collections::HashSet<usize>,
    /// Indices of assistant messages whose thinking blocks are expanded.
    pub expanded_thinking: std::collections::HashSet<usize>,
    pub pricing: caboose_core::provider::pricing::PricingRegistry,
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
    /// Whether the active model supports extended thinking/reasoning.
    pub model_supports_thinking: bool,
    /// Current thinking mode toggle state.
    pub thinking_mode: caboose_core::provider::ThinkingMode,
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
    /// Absolute canonicalized path of the primary repository root.
    /// Captured at startup via `canonicalize(current_dir())`.
    pub primary_root: std::path::PathBuf,
    /// Receiver for async directory scan results used by WorkspaceAdd dialog.
    /// `None` when no scan is in progress.
    pub workspace_scan_rx: Option<tokio::sync::mpsc::Receiver<Vec<String>>>,
    /// Debounce tracking: last path_input value when scan was triggered.
    pub workspace_scan_last_query: String,
    /// Screen y → message index for clickable truncation indicators.
    pub truncation_click_zones: RefCell<Vec<(u16, usize)>>,
    /// Click zones for thinking block arrows: (screen_row, message_index).
    /// usize::MAX represents the currently-streaming thinking block.
    /// Populated by the post-render wrapping pass (same pattern as truncation_click_zones).
    pub thinking_click_zones: RefCell<Vec<(u16, usize)>>,
    /// Index into chat_messages of the assistant message currently hovered by mouse.
    pub hovered_message: Option<usize>,
    /// Per-frame hover zones for assistant messages: (start_screen_y, end_screen_y, msg_index).
    /// Computed post-render in layout.rs; read by Moved mouse handler.
    pub copy_hover_zones: RefCell<Vec<(u16, u16, usize)>>,
    /// Screen rect of the copy badge for the currently hovered message.
    /// Set during badge render; used by Down mouse handler for click detection.
    pub copy_badge_rect: Cell<Option<ratatui::prelude::Rect>>,
    /// Per-frame hover zones for code blocks: (start_y, end_y, msg_index, block_index).
    /// Computed during chat render in layout.rs; read by Moved mouse handler.
    pub code_block_hover_zones: RefCell<Vec<(u16, u16, usize, usize)>>,
    /// Screen rect of the code block copy badge (when hovering a code block).
    pub code_block_badge_rect: Cell<Option<ratatui::prelude::Rect>>,
    /// Currently hovered code block: (msg_index, block_index).
    pub hovered_code_block: Option<(usize, usize)>,
    /// Screen rect of the scroll-to-bottom badge, shown when the user has scrolled up.
    /// Set during badge render; cleared each frame; used by mouse Down handler.
    pub scroll_to_bottom_badge_rect: Cell<Option<ratatui::prelude::Rect>>,
    /// Screen y → message index for clickable diff toggle indicators (▶/▼ expand/collapse).
    pub tool_toggle_rects: RefCell<Vec<(u16, usize)>>,
    /// Active mouse text selection in the chat area.
    pub text_selection: Option<TextSelection>,
    /// The Rect of the chat area, set each frame for mouse hit-testing.
    pub chat_area: Cell<Option<ratatui::prelude::Rect>>,
    /// Plain-text content of wrapped chat rows (rebuilt each frame for text extraction).
    pub rendered_chat_text: RefCell<Vec<String>>,
    /// Active skill creation session (set by `/create-skill`).
    pub skill_creation: Option<crate::skills::creation::SkillCreationState>,
    /// Pending handoff summary awaiting user confirmation (y/n).
    pub pending_handoff: Option<String>,
    /// When true, the model picker spawns a handoff subagent instead of switching models.
    pub handoff_agent_pending: bool,
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
    /// Accumulated session cost in USD (reset on /new).
    pub session_cost: f64,
    /// Accumulated input tokens across all turns in this session.
    pub session_input_tokens: u64,
    /// Accumulated output tokens across all turns in this session.
    pub session_output_tokens: u64,
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
    /// Per-invocation override for roundhouse critique: `--no-critique` → Some(false),
    /// `--critique` → Some(true), neither → None (falls back to config).
    pub roundhouse_critique_override: Option<bool>,
    /// When true, the model picker adds to roundhouse secondaries instead of switching.
    pub roundhouse_model_add: bool,
    /// When true, LocalProviderConnect was opened from the model picker (vs /connect command).
    /// On completion: update discovered_locals + handle roundhouse vs active-provider switch.
    pub model_picker_connect: bool,
    /// Receiver for roundhouse planner status updates (parallel planning engine).
    pub roundhouse_update_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::roundhouse::PlannerUpdate>>,
    /// Receiver for roundhouse synthesis streaming deltas.
    pub roundhouse_synthesis_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    /// Receiver for roundhouse critique phase planner updates.
    pub roundhouse_critique_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::roundhouse::PlannerUpdate>>,
    /// Active subagents.
    pub sub_agents: Vec<crate::sub_agent::SubAgent>,
    /// Sender cloned into each subagent executor so they can emit events.
    pub sub_agent_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::sub_agent::SubAgentEvent>>,
    /// Receiver for subagent events, polled each frame.
    pub sub_agent_rx: Option<tokio::sync::mpsc::UnboundedReceiver<crate::sub_agent::SubAgentEvent>>,
    /// Queued subagent approval requests (agent_id, tool_name, arguments).
    pub sub_agent_pending_approvals: std::collections::VecDeque<(uuid::Uuid, String, String)>,
    /// The subagent approval currently being shown to the user, if any.
    pub sub_agent_approval_showing: Option<(uuid::Uuid, String, String)>,
    /// Pending spawn_agent background tasks. Polled each event-loop tick.
    pub spawn_agent_handles: Vec<SpawnAgentHandle>,
    /// Collected AgentChanges from completed agents, used for cross-agent conflict detection.
    pub agent_changes: Vec<crate::sub_agent::conflict::AgentChanges>,
    /// Conflict report from the latest cross-agent sweep, if any blocking overlaps were found.
    pub conflict_report: Option<crate::sub_agent::conflict::ConflictReport>,
    /// Index into sub_agents for the stream overlay. None = closed.
    // TODO: consumed by overlay renderer
    pub agent_stream_overlay: Option<usize>,
    /// Selected agent row in sidebar agents section.
    pub sidebar_agent_selected: usize,
    /// Absolute screen row of the clickable agents dismiss button (set each frame).
    pub agents_dismiss_row: Cell<Option<u16>>,
    /// Whether the "Files Modified" sidebar section is collapsed.
    pub files_modified_collapsed: bool,
    /// Screen row of the "Files Modified" header for click-to-toggle (set each frame).
    pub files_modified_header_row: Cell<Option<u16>>,
    /// In-session circuit manager.
    pub circuit_manager: crate::circuits::runner::CircuitManager,
    /// Local LLM servers discovered at startup (background probe).
    pub discovered_locals: Vec<caboose_core::provider::local::LocalServer>,
    /// Receiver for background local server discovery result.
    pub local_discovery_rx:
        Option<tokio::sync::oneshot::Receiver<Vec<caboose_core::provider::local::LocalServer>>>,
    /// Detected SCM provider for the current working directory.
    pub scm_provider: crate::scm::detection::ScmProvider,
    /// Active SCM watchers (each backed by a circuit).
    pub active_watchers: Vec<crate::scm::watcher::Watcher>,
    /// Whether the pending diff preview is expanded (true) or collapsed (false).
    pub diff_expanded: bool,
    /// Scroll offset for the expanded pending diff.
    pub diff_scroll: usize,
    /// Session-scoped pinned rules injected into system prompt.
    pub pins: Vec<String>,
    /// Whether the pins sidebar section is expanded.
    pub pins_expanded: bool,
    /// Oneshot receiver for LLM-generated session title.
    pub title_rx: Option<tokio::sync::oneshot::Receiver<String>>,
    /// Set to true when user manually sets title via /title — prevents LLM overwrite.
    pub title_manually_set: bool,
    /// Embedded WebSocket server handle (when server is enabled).
    pub server_handle: Option<caboose_server::ServerHandle>,
    /// Background agent manager for /bg commands (wired in follow-up).
    #[allow(dead_code)]
    pub background_manager:
        Option<std::sync::Arc<caboose_core::background::BackgroundAgentManager>>,
    /// Cached background agent list for sidebar rendering (updated on CoreEvent).
    pub background_agents_cache: Vec<caboose_core::background::BackgroundAgentInfo>,
    /// Sequential counter for simple background agent IDs (0, 1, 2...).
    pub bg_agent_counter: u32,
    /// Receiver for background search setup result.
    pub search_setup_rx: Option<tokio::sync::oneshot::Receiver<String>>,
    /// Receiver for core events (background agent lifecycle, etc.).
    #[allow(dead_code)]
    pub core_event_rx: Option<tokio::sync::broadcast::Receiver<caboose_core::events::CoreEvent>>,
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

        // Disable slash commands while roundhouse is active (awaiting prompt or running)
        if self.roundhouse_session.is_some() {
            self.slash_auto = None;
            return;
        }

        let input_text = self.input.content();
        let prefix = crate::tui::slash_auto::slash_prefix(&input_text);
        match prefix {
            Some(p) => {
                let count = crate::tui::slash_auto::total_filtered(
                    p,
                    &self.commands,
                    &self.agent_definitions,
                    &self.skills,
                );
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

        let prefs = crate::prefs::TuiPrefs::load();

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
        let memory_store = caboose_core::memory::MemoryStore::new(
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

        // Load custom agent definitions (ensure directories exist)
        let project_agents_dir = std::path::PathBuf::from(".caboose/agents");
        let global_agents_dir = dirs::config_dir()
            .map(|d| d.join("caboose/agents"))
            .unwrap_or_else(|| std::path::PathBuf::from(".caboose/agents"));
        let _ = std::fs::create_dir_all(&project_agents_dir);
        let _ = std::fs::create_dir_all(&global_agents_dir);
        let agent_definitions =
            crate::agents::load_agents(Some(&project_agents_dir), Some(&global_agents_dir));

        // Build system prompt with memory context
        let memory_ctx = memory_store.load_context();
        let base_prompt = config
            .system_prompt
            .clone()
            .unwrap_or_else(|| {
                "You are Caboose, a terminal-native AI coding agent. You help users build, debug, \
                 and understand software from the command line.\n\n\
                 ## Tone\n\n\
                 Be conversational and direct — like a sharp coworker pairing with the user. \
                 Briefly say what you're about to do before doing it (e.g. \"let me check that file\", \
                 \"I'll search for where that's defined\"). Don't explain what you just did after — the \
                 user can see the tool output. Keep text responses to a few lines unless the user asks \
                 for detail. No preamble (\"Based on my analysis...\"), no postamble (\"Let me know if \
                 you need anything else!\"), no filler.\n\n\
                 Your output renders as markdown in a monospace terminal. Use formatting when it helps \
                 (code blocks, bold, lists) but don't over-format simple answers.\n\n\
                 ## Tools\n\n\
                 Use `glob` and `grep` to locate files before reading — don't guess paths. \
                 Use `read_file` with `offset`/`limit` for targeted reads. \
                 Batch independent tool calls in a single response. \
                 Don't re-read files already in context unless they've been modified.\n\n\
                 ## Images\n\n\
                 The user may attach images to their messages (screenshots, diagrams, photos). \
                 When images are present, you can see and analyze them. Describe what you see \
                 and use the visual context to inform your response.\n\n\
                 ## Tasks\n\n\
                 Use `todo_write` for multi-step work (3+ steps) to show progress in the sidebar. \
                 Each call replaces the entire list. Keep task names short. \
                 Mark tasks completed as you finish each one. \
                 When the user changes topic, don't carry over old tasks — they are cleared automatically.\n\n\
                 ## Conventions\n\n\
                 Before editing code, read it first. Match the existing style — naming, patterns, \
                 libraries. Don't add comments unless the code is genuinely tricky. Don't refactor \
                 code you weren't asked to touch. Don't commit unless the user asks you to. \
                 Follow security best practices — never log secrets, never commit credentials.\n\n\
                 ## Error recovery\n\n\
                 When a shell command fails (non-zero exit code, test failures, lint errors, build errors), \
                 don't just report the error — read the output, fix the underlying issue, and re-run the \
                 command to verify. Keep going until the command succeeds or you've determined the problem \
                 is beyond an automatic fix (e.g. requires user input, missing credentials, ambiguous \
                 requirements). If you've retried and the same error persists, stop and explain what's \
                 wrong instead of looping.\n\n\
                 ## Thinking\n\n\
                 When reasoning internally, think naturally about the problem itself. Don't narrate what \
                 you are, describe your own instructions, or explain your reasoning process — just reason."
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

        // Inject agent awareness block into system prompt
        let system_prompt = if !agent_definitions.is_empty() {
            let mut prompt = system_prompt;
            prompt.push_str(&crate::agents::build_agent_awareness_block(
                &agent_definitions,
            ));
            prompt
        } else {
            system_prompt
        };

        // Inject workspace context
        let system_prompt = {
            let ws_block = workspace_system_prompt_block(&config.workspaces);
            if !ws_block.is_empty() {
                let mut prompt = system_prompt;
                prompt.push_str(&ws_block);
                prompt
            } else {
                system_prompt
            }
        };

        // Session pins
        // (At construction time pins are empty; they are injected dynamically
        // when loaded from a resumed session or added via /pin.)

        // Inject spawn_agent guidance
        let system_prompt = {
            let mut prompt = system_prompt;
            prompt.push_str(
                "\n\n## Subagents\n\n\
                 You have access to a `spawn_agent` tool. Use it when you identify work that \
                 can proceed in parallel — for example, when you have a list of independent \
                 tasks that don't share state or outputs. Call `spawn_agent` once per \
                 independent task; multiple calls in the same response run concurrently. \
                 The subagents work in isolated git worktrees and merge their changes back \
                 when done. For sequential or dependent work, do it yourself.\n",
            );
            prompt.push_str(
                "\nYou also have a `spawn_background` tool. Use it for tasks that are \
                 independent of the current conversation and don't need interactive feedback — \
                 long test runs, large refactors, code generation tasks where the user doesn't \
                 need to watch progress. Background agents run with auto-approve and report \
                 results when done. Don't use it for tasks where the user is waiting for the answer.\n",
            );
            prompt
        };

        let mut agent = AgentLoop::new(system_prompt, permission_mode);

        // Wire primary_root into agent for cross-workspace write detection
        let primary_root =
            std::fs::canonicalize(std::env::current_dir().unwrap_or_default()).unwrap_or_default();
        agent.primary_root = primary_root.clone();
        agent.workspace_paths = config.workspaces.values().map(|c| c.path.clone()).collect();

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
            if caboose_core::provider::models_dev::context_window(model_id).is_none()
                && let Ok(model_list) = p.list_models().await
            {
                let cw_entries: Vec<(String, Option<u32>)> = model_list
                    .iter()
                    .map(|m| (m.id.clone(), m.context_window))
                    .collect();
                caboose_core::provider::models_dev::cache_from_model_list(&cw_entries);
            }

            agent.context_window =
                caboose_core::provider::models_dev::context_window_or_default(model_id);
        }

        let active_provider_name = provider
            .as_ref()
            .map(|p| p.name().to_string())
            .unwrap_or_else(|| "none".to_string());
        let active_model_name = provider
            .as_ref()
            .map(|p| p.model().to_string())
            .unwrap_or_else(|| "no key configured".to_string());

        // Infer model capabilities from provider/model name at startup.
        // These will be updated with accurate values when the model picker fetches the model list.
        // Capabilities default to false; updated from provider model list at startup below

        let (sub_agent_tx, sub_agent_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::sub_agent::SubAgentEvent>();

        // Create core event bus
        let (core_handle, _cmd_rx) = caboose_core::events::CoreHandle::new();
        let core_event_rx = core_handle.subscribe();

        // Initialize background agent manager from config
        let bg_config = {
            let schema = config.background_agents.as_ref();
            caboose_core::background::BackgroundAgentConfig {
                per_agent_budget: schema.and_then(|s| s.per_agent_budget).unwrap_or(100_000),
                global_budget: schema.and_then(|s| s.global_budget).unwrap_or(500_000),
                max_agents: schema.and_then(|s| s.max_agents).unwrap_or(5),
            }
        };
        let background_manager = std::sync::Arc::new(
            caboose_core::background::BackgroundAgentManager::new(bg_config, core_handle),
        );

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
                sidebar_width: 35,
                sidebar_focused: false,
                sidebar_drag: None,
                focused_tool: None,
                slash_auto: None,
                file_auto: None,
                skills,
                agent_definitions,
                current_session_id: None,
                memory: memory_store,
                history: crate::tui::input_history::InputHistory::load(),
                last_text_input_at: None,
                rapid_input_streak: 0,
                paste_like_mode_until: None,
                expanded_messages: std::collections::HashSet::new(),
                expanded_thinking: std::collections::HashSet::new(),
                pricing: caboose_core::provider::pricing::PricingRegistry::new(),
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
                model_supports_vision: false,
                model_supports_thinking: false,
                thinking_mode: caboose_core::provider::ThinkingMode::Off,
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
                primary_root: primary_root.clone(),
                workspace_scan_rx: None,
                workspace_scan_last_query: String::new(),
                truncation_click_zones: RefCell::new(Vec::new()),
                thinking_click_zones: RefCell::new(Vec::new()),
                hovered_message: None,
                copy_hover_zones: RefCell::new(Vec::new()),
                copy_badge_rect: Cell::new(None),
                code_block_hover_zones: RefCell::new(Vec::new()),
                code_block_badge_rect: Cell::new(None),
                hovered_code_block: None,
                scroll_to_bottom_badge_rect: Cell::new(None),
                tool_toggle_rects: RefCell::new(Vec::new()),
                text_selection: None,
                chat_area: Cell::new(None),
                rendered_chat_text: RefCell::new(Vec::new()),
                skill_creation: None,
                pending_handoff: None,
                handoff_agent_pending: false,
                terminal_panel: None,
                terminal_focused: false,
                terminal_area: Cell::new(None),
                terminal_last_size: None,
                ask_user_session: None,
                mcp_connect_rx,
                mcp_connect_tx,
                session_cost: 0.0,
                session_input_tokens: 0,
                session_output_tokens: 0,
                budget_paused: false,
                checkpoints: crate::checkpoint::CheckpointManager::new(),
                attachments: Vec::new(),
                update_available: None,
                update_check_rx: None,
                roundhouse_session: None,
                roundhouse_critique_override: None,
                roundhouse_model_add: false,
                model_picker_connect: false,
                roundhouse_update_rx: None,
                roundhouse_synthesis_rx: None,
                roundhouse_critique_rx: None,
                sub_agents: Vec::new(),
                sub_agent_tx: Some(sub_agent_tx),
                sub_agent_rx: Some(sub_agent_rx),
                sub_agent_pending_approvals: std::collections::VecDeque::new(),
                sub_agent_approval_showing: None,
                spawn_agent_handles: Vec::new(),
                agent_changes: Vec::new(),
                conflict_report: None,
                agent_stream_overlay: None,
                sidebar_agent_selected: 0,
                agents_dismiss_row: Cell::new(None),
                files_modified_collapsed: false,
                files_modified_header_row: Cell::new(None),
                circuit_manager: crate::circuits::runner::CircuitManager::new(5),
                discovered_locals: vec![],
                local_discovery_rx: None,
                scm_provider,
                active_watchers: Vec::new(),
                diff_expanded: false,
                diff_scroll: 0,
                pins: vec![],
                pins_expanded: false,
                title_rx: None,
                title_manually_set: false,
                server_handle: None,
                background_manager: Some(background_manager),
                background_agents_cache: Vec::new(),
                bg_agent_counter: 0,
                search_setup_rx: None,
                core_event_rx: Some(core_event_rx),
            },
            terminal,
            provider,
        };

        // Resolve dedicated compaction provider (if configured)
        app.resolve_compaction_provider();

        // Fetch OpenRouter pricing at startup so sidebar shows costs immediately
        if app.state.active_provider_name == "openrouter"
            && let Some(api_key) = app.state.config.keys.get("openrouter")
        {
            let or_provider = caboose_core::provider::openrouter::OpenRouterProvider::new(
                api_key.to_string(),
                app.state.active_model_name.clone(),
            );
            if let Ok((models, pricing_entries)) = or_provider.list_models_with_pricing().await {
                for (model_id, model_pricing) in &pricing_entries {
                    app.state
                        .pricing
                        .insert_with_cross_map(model_id.clone(), *model_pricing);
                }
                // Set capabilities for the active model from the fetched list
                if let Some(info) = models.iter().find(|m| m.id == app.state.active_model_name) {
                    app.state.model_supports_tools = info.supports_tools;
                    app.state.model_supports_vision = info.supports_vision;
                    app.state.model_supports_thinking = info.supports_thinking;
                }
            }
        }

        // Load user pricing overrides from config (highest priority — applied last)
        if !app.state.config.pricing.is_empty() {
            app.state
                .pricing
                .load_from_config(&app.state.config.pricing);
        }

        // For non-OpenRouter providers, look up capabilities from the provider's model list
        if app.state.active_provider_name != "openrouter"
            && let Some(ref provider) = app.provider
            && let Ok(models) = provider.list_models().await
            && let Some(info) = models.iter().find(|m| m.id == app.state.active_model_name)
        {
            app.state.model_supports_tools = info.supports_tools;
            app.state.model_supports_vision = info.supports_vision;
            app.state.model_supports_thinking = info.supports_thinking;
        }

        // If --session was provided, restore that session
        if let Some(ref sid) = session_id {
            app.restore_session(sid);
        }

        Ok(app)
    }
}
