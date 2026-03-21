use super::*;

impl App {
    fn handle_workspace_command(&mut self, slash: &str) {
        use crate::tui::dialog::DialogKind;

        let args: Vec<&str> = slash.split_whitespace().collect();

        let _ = args.get(1); // subcommand reserved for future use
        let state = build_workspace_list_state(&self.state.config);
        self.state
            .dialog_stack
            .push(DialogKind::WorkspaceList(state));
    }

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

    /// Handle `/suggest` — run codebase scans and inject digest into conversation.
    async fn handle_suggest_command(&mut self) {
        // Switch to chat screen if on home
        self.state.dialog_stack.base = Screen::Chat;

        // Show scanning message
        self.state.chat_messages.push(ChatMessage::System {
            content: "Scanning codebase...".to_string(),
        });

        // Run the suggest pipeline
        let suggest_config = self.state.config.suggest.as_ref();
        let digest = crate::suggest::run_suggest(suggest_config).await;

        // Update scanning message
        if let Some(ChatMessage::System { content }) = self.state.chat_messages.last_mut()
            && content == "Scanning codebase..."
        {
            *content = "Scan complete — analyzing findings...".to_string();
        }

        // Inject digest as user message and trigger agent stream
        if let Some(ref provider) = self.provider {
            let tool_defs = self.build_tool_defs();
            self.state
                .agent
                .send_message(digest, provider.as_ref(), &tool_defs);
        }
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

        let messages = vec![caboose_core::provider::Message {
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
                    Ok(caboose_core::provider::StreamEvent::TextDelta(text)) => {
                        crate::init::handler::InitEvent::TextDelta(text)
                    }
                    Ok(caboose_core::provider::StreamEvent::Done {
                        input_tokens,
                        output_tokens,
                        ..
                    }) => crate::init::handler::InitEvent::Done {
                        input_tokens: input_tokens.unwrap_or(0),
                        output_tokens: output_tokens.unwrap_or(0),
                    },
                    Ok(caboose_core::provider::StreamEvent::Error(e)) => {
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
    pub(super) fn finalize_init(&mut self) {
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
            thinking: None,
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

    /// Handle /pin — add a pinned rule, auto-creates session from home screen.
    fn handle_pin_command(&mut self, slash: &str) {
        let args = slash.strip_prefix("pin").unwrap_or("").trim();

        // /pin --save <text> — append rule to CABOOSE.md (persistent across sessions)
        if let Some(save_text) = args.strip_prefix("--save") {
            let save_text = save_text.trim();
            if save_text.is_empty() {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Usage: /pin --save <text>".to_string(),
                });
                self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
                self.state.dialog_stack.clear();
                return;
            }
            let caboose_path = self.state.primary_root.join("CABOOSE.md");
            let mut content = std::fs::read_to_string(&caboose_path).unwrap_or_default();
            if content.is_empty() {
                content = "# CABOOSE.md\n\n## Rules\n".to_string();
            }
            // Append under a Rules section if it exists, otherwise at the end
            if !content.contains("## Rules") {
                content.push_str("\n## Rules\n");
            }
            content.push_str(&format!("\n- {save_text}\n"));
            match std::fs::write(&caboose_path, &content) {
                Ok(()) => {
                    // Also add as a session pin for immediate effect
                    self.state.pins.push(save_text.to_string());
                    self.sync_pins_to_system_prompt();
                    if let Some(ref sid) = self.state.current_session_id {
                        let _ = self.state.sessions.update_pins(sid, &self.state.pins);
                    }
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!("Saved to CABOOSE.md and pinned: {save_text}"),
                    });
                }
                Err(e) => {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("Failed to write CABOOSE.md: {e}"),
                    });
                }
            }
            self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
            self.state.dialog_stack.clear();
            return;
        }

        let text = args.to_string();
        if text.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Usage: /pin <text>  or  /pin --save <text>".to_string(),
            });
            // Still switch to chat so the error is visible
            self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
            self.state.dialog_stack.clear();
            return;
        }
        // Auto-create session if needed
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
            match self.state.sessions.create(model, provider, None, None) {
                Ok(session) => {
                    self.state.agent.init_cold_store(&session.id);
                    self.state.current_session_id = Some(session.id);
                }
                Err(e) => {
                    tracing::warn!("Failed to create session for pin: {e}");
                    return;
                }
            }
        }
        self.state.pins.push(text.clone());
        self.sync_pins_to_system_prompt();
        if let Some(ref sid) = self.state.current_session_id {
            let _ = self.state.sessions.update_pins(sid, &self.state.pins);
        }
        self.state.chat_messages.push(ChatMessage::System {
            content: format!("Pinned: {text}"),
        });
        self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
        self.state.dialog_stack.clear();
    }

    /// Handle /pins — list all pinned rules.
    fn handle_pins_command(&mut self) {
        if self.state.pins.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "No pins set.".to_string(),
            });
        } else {
            let list = self
                .state
                .pins
                .iter()
                .enumerate()
                .map(|(i, p)| format!("  {}. {p}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            self.state.chat_messages.push(ChatMessage::System {
                content: format!("Pins:\n{list}"),
            });
        }
    }

    /// Handle /unpin — remove pin(s) by index or clear all.
    fn handle_unpin_command(&mut self, slash: &str) {
        let arg = slash.strip_prefix("unpin").unwrap_or("").trim();
        if arg.is_empty() {
            if self.state.pins.is_empty() {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "No pins to remove.".to_string(),
                });
            } else {
                let count = self.state.pins.len();
                self.state.pins.clear();
                self.sync_pins_to_system_prompt();
                if let Some(ref sid) = self.state.current_session_id {
                    let _ = self.state.sessions.update_pins(sid, &self.state.pins);
                }
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Removed all {count} pins."),
                });
            }
        } else if let Ok(n) = arg.parse::<usize>() {
            if n == 0 || n > self.state.pins.len() {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Pin {n} does not exist. You have {} pins.",
                        self.state.pins.len()
                    ),
                });
            } else {
                let removed = self.state.pins.remove(n - 1);
                self.sync_pins_to_system_prompt();
                if let Some(ref sid) = self.state.current_session_id {
                    let _ = self.state.sessions.update_pins(sid, &self.state.pins);
                }
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Removed pin: {removed}"),
                });
            }
        } else {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Usage: /unpin or /unpin <number>".to_string(),
            });
        }
    }

    pub(super) async fn handle_shared_slash(&mut self, slash: &str) -> bool {
        if slash == "init" {
            self.handle_init_command();
            return true;
        }
        if slash == "status" || slash == "usage" || slash == "cost" {
            self.state.dialog_stack.push(DialogKind::Status);
            return true;
        }
        if slash == "mcp" {
            self.open_mcp_picker();
            return true;
        }
        if slash.starts_with("mcp ") {
            self.handle_mcp_command(slash).await;
            return true;
        }
        if slash == "workspace" || slash.starts_with("workspace ") {
            self.handle_workspace_command(slash);
            return true;
        }
        if slash == "model" {
            self.state.handoff_agent_pending = false;
            self.open_model_dropdown().await;
            return true;
        }
        if slash == "memories" {
            self.handle_memories_command();
            return true;
        }
        if slash == "forget" {
            self.handle_forget_command();
            return true;
        }
        if slash.starts_with("create-skill") {
            let args = slash.strip_prefix("create-skill").unwrap_or("").trim();
            self.handle_create_skill_command(args);
            return true;
        }
        if slash == "reasoning" {
            if !self.state.model_supports_thinking {
                self.state.dialog_stack.base = Screen::Chat;
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!(
                        "{} does not support reasoning",
                        self.state.active_model_name
                    ),
                });
                return true;
            }
            let current = self.state.thinking_mode.label().to_string();
            let items = vec![crate::tui::slash_auto::SettingsItem {
                key: "reasoning.level".to_string(),
                label: "Reasoning Level".to_string(),
                value: current,
                kind: crate::tui::slash_auto::SettingsKind::Choice(
                    caboose_core::provider::ThinkingMode::ALL
                        .iter()
                        .map(|l| l.label().to_string())
                        .collect(),
                ),
            }];
            self.state.slash_auto = Some(crate::tui::slash_auto::SlashAutoState {
                selected: 0,
                mode: crate::tui::slash_auto::DropdownMode::Settings { items },
                filter: String::new(),
            });
            return true;
        }
        if slash == "settings" {
            self.open_settings_picker();
            return true;
        }
        if let Some(name) = slash.strip_prefix("checkpoint ") {
            let name = name.trim();
            if name.is_empty() {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: "Usage: /checkpoint <name>".into(),
                });
            } else {
                let id = self.state.checkpoints.create_named(name);
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Checkpoint \"{name}\" saved (id {id})."),
                });
            }
            return true;
        }
        if slash == "checkpoint" {
            self.state.chat_messages.push(ChatMessage::Error {
                content: "Usage: /checkpoint <name>".into(),
            });
            return true;
        }
        if slash == "rewind" {
            self.open_rewind_picker();
            return true;
        }
        if slash == "undo" {
            // Find the last checkpoint that has file changes
            let last_with_files = self
                .state
                .checkpoints
                .list()
                .iter()
                .rev()
                .find(|c| !c.files.is_empty())
                .map(|c| c.id);

            match last_with_files {
                Some(checkpoint_id) => match self.state.checkpoints.rewind(checkpoint_id) {
                    Ok(summary) => {
                        self.recompute_modified_files();
                        self.state.chat_messages.push(ChatMessage::System {
                            content: format!("Undo: {summary}"),
                        });
                    }
                    Err(e) => {
                        self.state.chat_messages.push(ChatMessage::Error {
                            content: format!("Undo failed: {e}"),
                        });
                    }
                },
                None => {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: "Nothing to undo — no file changes to revert.".into(),
                    });
                }
            }
            return true;
        }
        if slash == "suggest" {
            self.handle_suggest_command().await;
            return true;
        }
        if slash == "export" {
            let title = self.state.session_title.clone().unwrap_or_else(|| {
                self.state
                    .current_session_id
                    .as_ref()
                    .map(|id| id.chars().take(6).collect())
                    .unwrap_or_else(|| "untitled".to_string())
            });

            let slug = crate::session::export::slugify(&title);
            let date = chrono::Local::now().format("%Y-%m-%d");
            let filename = format!("{slug}-{date}.md");
            let dir = std::path::PathBuf::from(".caboose/exports");

            if let Err(e) = std::fs::create_dir_all(&dir) {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to create exports directory: {e}"),
                });
                return true;
            }

            let path = dir.join(&filename);
            let markdown =
                crate::session::export::format_markdown(&title, &self.state.chat_messages);

            match std::fs::write(&path, &markdown) {
                Ok(()) => {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!("Session exported to {}", path.display()),
                    });
                }
                Err(e) => {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("Failed to export session: {e}"),
                    });
                }
            }
            return true;
        }
        // /roundhouse — parse --critique / --no-critique flags
        if slash == "roundhouse" || slash.starts_with("roundhouse ") {
            if slash.contains("--no-critique") {
                self.state.roundhouse_critique_override = Some(false);
            } else if slash.contains("--critique") {
                self.state.roundhouse_critique_override = Some(true);
            } else {
                self.state.roundhouse_critique_override = None;
            }
        }
        if let Some(sub) = slash.strip_prefix("roundhouse ") {
            self.handle_roundhouse_subcommand(sub.trim());
            return true;
        }
        if let Some(args) = slash.strip_prefix("circuit ") {
            self.handle_circuit_command(args.trim()).await;
            return true;
        }
        if let Some(args) = slash.strip_prefix("watch ") {
            self.handle_watch_command(args.trim()).await;
            return true;
        }
        if slash == "pin" || slash.starts_with("pin ") {
            self.handle_pin_command(slash);
            return true;
        }
        if slash == "pins" {
            self.handle_pins_command();
            return true;
        }
        if slash == "unpin" || slash.starts_with("unpin ") {
            self.handle_unpin_command(slash);
            return true;
        }
        // /bg — background agent commands
        if slash == "bg" || slash.starts_with("bg ") {
            self.handle_bg_command(slash);
            return true;
        }
        // /pair — generate device pairing code
        if slash == "pair" {
            self.handle_pair_command();
            return true;
        }
        // /devices — list paired devices
        if slash == "devices" {
            self.handle_devices_command();
            return true;
        }
        // /unpair — revoke a paired device
        if slash == "unpair" || slash.starts_with("unpair ") {
            self.handle_unpair_command(slash);
            return true;
        }
        // /new — confirm if session has real content, then extract memories and clear
        if slash == "new" {
            if needs_new_session_confirm(&self.state.chat_messages) {
                self.state
                    .dialog_stack
                    .push(crate::tui::dialog::DialogKind::Confirm {
                        message: "start a new session?".into(),
                        on_confirm: crate::tui::dialog::ConfirmAction::NewSession,
                    });
                return true;
            }
            // No real conversation — proceed directly
            self.execute_new_session().await;
            return true;
        }
        // Command registry fallback
        if let Some(cmd) = self.state.commands.find_slash(slash)
            && (cmd.available)(&self.state)
        {
            let action = (cmd.execute)(&mut self.state);
            self.process_action(action).await;
            return true;
        }
        false
    }

    async fn handle_circuit_command(&mut self, args: &str) {
        if matches!(self.state.dialog_stack.base, Screen::Home) {
            self.state.dialog_stack.base = Screen::Chat;
            self.state.dialog_stack.clear();
        }

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

        // /circuit <interval> "prompt"
        match parse_circuit_args(args) {
            Some((interval_secs, prompt)) => {
                let _ = self.create_circuit(&prompt, interval_secs).await;
            }
            None => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: "Usage: /circuit <interval> \"<prompt>\"\nExamples: /circuit 5m \"check build\" | /circuit 10m \"watch CI\"".to_string(),
                });
            }
        }
    }

    /// Create a circuit and return its ID on success, or None on failure.
    async fn create_circuit(&mut self, prompt: &str, interval_secs: u64) -> Option<String> {
        let ts = chrono::Utc::now().timestamp_millis() as u64;
        let mut id = format!("c-{:x}", ts % 0x1000000);
        // Ensure uniqueness against existing circuits
        let mut counter = 1u64;
        while self
            .state
            .circuit_manager
            .circuits
            .iter()
            .any(|h| h.circuit.id == id)
        {
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
        if matches!(self.state.dialog_stack.base, Screen::Home) {
            self.state.dialog_stack.base = Screen::Chat;
            self.state.dialog_stack.clear();
        }

        // /watch pr <number>
        // /watch mr <number>
        let rest = if let Some(r) = args
            .strip_prefix("pr ")
            .or_else(|| args.strip_prefix("mr "))
        {
            r
        } else {
            self.state.chat_messages.push(ChatMessage::Error {
                content: "Usage: /watch pr <number>".to_string(),
            });
            return;
        };

        let pr_number = match rest
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u32>().ok())
        {
            Some(n) => n,
            None => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: "Usage: /watch pr <number>".to_string(),
                });
                return;
            }
        };

        self.create_watcher(pr_number).await;
    }

    async fn create_watcher(&mut self, pr_number: u32) {
        let interval_secs = 180; // 3 minutes
        let prompt = format!(
            "Check the status of PR/MR #{pr_number}. Use the check_ci tool and report: is CI passing, failing, or pending? Is the PR merged or closed?"
        );

        if let Some(circuit_id) = self.create_circuit(&prompt, interval_secs).await {
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

    // ---- Background agent commands ----

    fn handle_bg_command(&mut self, slash: &str) {
        let args = slash.strip_prefix("bg").unwrap_or("").trim();
        if args.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Usage: /bg <prompt> — spawn a background agent\n\
                         /bg list — show running agents\n\
                         /bg kill <id> — stop an agent\n\
                         (background agent spawning not yet wired)"
                    .to_string(),
            });
            return;
        }
        match args
            .split_once(' ')
            .map(|(cmd, rest)| (cmd, rest.trim()))
            .unwrap_or((args, ""))
        {
            ("list", _) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "No background agents running. (spawning not yet wired)".to_string(),
                });
            }
            ("kill", id) if !id.is_empty() => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Background agent kill not yet implemented (id: {id})."),
                });
            }
            ("attach", id) if !id.is_empty() => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Background agent attach not yet implemented (id: {id})."),
                });
            }
            ("detach", _) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Background agent detach not yet implemented.".to_string(),
                });
            }
            _ => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Would spawn background agent: \"{args}\" (not yet wired)."),
                });
            }
        }
    }

    fn handle_pair_command(&mut self) {
        if self.state.server_handle.is_none() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Server not running. Enable [server] in config to use pairing."
                    .to_string(),
            });
            return;
        }
        self.state.chat_messages.push(ChatMessage::System {
            content: "Device pairing not yet wired to TUI.".to_string(),
        });
    }

    fn handle_devices_command(&mut self) {
        if self.state.server_handle.is_none() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Server not running. Enable [server] in config to manage devices."
                    .to_string(),
            });
            return;
        }
        self.state.chat_messages.push(ChatMessage::System {
            content: "Device listing not yet wired to TUI.".to_string(),
        });
    }

    fn handle_unpair_command(&mut self, slash: &str) {
        let device_id = slash.strip_prefix("unpair").unwrap_or("").trim();
        if device_id.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Usage: /unpair <device_id>".to_string(),
            });
            return;
        }
        if self.state.server_handle.is_none() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Server not running. Enable [server] in config to manage devices."
                    .to_string(),
            });
            return;
        }
        self.state.chat_messages.push(ChatMessage::System {
            content: format!("Device unpair not yet wired (id: {device_id})."),
        });
    }
}
