use super::*;

impl App {
    /// Sync session pins into the agent's system prompt.
    ///
    /// Strips any existing `## Session Pins` section and, if pins are non-empty,
    /// appends a fresh one right before the `## Subagents` section.
    pub(super) fn sync_pins_to_system_prompt(&mut self) {
        let prompt = &mut self.state.agent.conversation.system_prompt;

        // Remove any existing pins section
        if let Some(start) = prompt.find("\n\n## Session Pins") {
            // Find where the next section begins (or end of string)
            let after = &prompt[start + 1..];
            let end = after
                .find("\n\n## ")
                .map(|pos| start + 1 + pos)
                .unwrap_or(prompt.len());
            prompt.replace_range(start..end, "");
        }

        // Insert pins before the Subagents section (if present)
        if !self.state.pins.is_empty() {
            let mut pins_block =
                String::from("\n\n## Session Pins (user-set rules for this session)\n");
            for (i, pin) in self.state.pins.iter().enumerate() {
                pins_block.push_str(&format!("{}. {pin}\n", i + 1));
            }
            if let Some(pos) = prompt.find("\n\n## Subagents") {
                prompt.insert_str(pos, &pins_block);
            } else {
                prompt.push_str(&pins_block);
            }
        }
    }

    /// Restore a session from the database, loading messages into the chat.
    pub(super) fn restore_session(&mut self, session_id: &str) {
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
        self.state.pins = session.pins.clone();
        self.state.pins_expanded = false;
        self.sync_pins_to_system_prompt();
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
        let mut i = 0;
        while i < messages.len() {
            let msg = &messages[i];
            let chat_msg = match msg.role.as_str() {
                "user" => {
                    i += 1;
                    ChatMessage::User {
                        content: msg.content.clone(),
                        images: vec![],
                    }
                }
                "thinking" => {
                    // Look ahead: if next message is "assistant", attach thinking to it
                    if i + 1 < messages.len() && messages[i + 1].role == "assistant" {
                        let thinking_content = msg.content.clone();
                        i += 1; // advance to assistant
                        let assistant_content = messages[i].content.clone();
                        i += 1; // advance past assistant
                        ChatMessage::Assistant {
                            content: assistant_content,
                            thinking: Some(thinking_content),
                        }
                    } else {
                        // Orphaned thinking — skip it
                        i += 1;
                        continue;
                    }
                }
                "assistant" => {
                    i += 1;
                    ChatMessage::Assistant {
                        content: msg.content.clone(),
                        thinking: None,
                    }
                }
                "system" => {
                    i += 1;
                    ChatMessage::System {
                        content: msg.content.clone(),
                    }
                }
                "provider_error" => {
                    i += 1;
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                        ChatMessage::ProviderError {
                            category: serde_json::from_value(
                                json.get("category").cloned().unwrap_or_default(),
                            )
                            .unwrap_or(caboose_core::provider::error::ErrorCategory::Unknown),
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
                "error" => {
                    i += 1;
                    ChatMessage::Error {
                        content: msg.content.clone(),
                    }
                }
                "task_outline" => {
                    i += 1;
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
                "fork_context" | "model_switch_context" => {
                    self.state.agent.conversation.messages.push(
                        crate::agent::conversation::Message {
                            role: crate::agent::conversation::Role::User,
                            content: crate::agent::conversation::Content::Text(msg.content.clone()),
                            tool_call_id: None,
                        },
                    );
                    i += 1;
                    continue;
                }
                _ => {
                    i += 1;
                    continue;
                }
            };
            self.state.chat_messages.push(chat_msg);
        }

        // If there were messages, go directly to chat screen
        if !self.state.chat_messages.is_empty() {
            self.state.dialog_stack.base = Screen::Chat;
        }
    }

    /// Ensure a session exists (create if needed) and persist a chat message.
    pub(super) fn persist_message(&mut self, role: &str, content: &str) {
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
            match self.state.sessions.create(model, provider, None, None) {
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
    pub(super) fn update_session_meta(&mut self) {
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
                parent_session_id: None,
                fork_message_count: None,
                pins: vec![],
                total_input_tokens: self.state.session_input_tokens,
                total_output_tokens: self.state.session_output_tokens,
                total_cost_usd: self.state.session_cost,
            };
            if let Err(e) = self.state.sessions.update(&session) {
                tracing::warn!("Failed to update session: {e}");
            }
        }
    }

    /// Recompute `modified_files` diffs from `file_baselines` vs current files on disk.
    /// Called after rewind restores files so the sidebar shows accurate counts.
    pub(super) fn recompute_modified_files(&mut self) {
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

    /// Run end-of-session memory extraction if enabled and there are enough observations.
    pub(super) async fn extract_session_memories(&mut self) {
        let memory_config = self.state.config.memory.clone().unwrap_or_default();
        if !memory_config.enabled || !memory_config.auto_extract {
            return;
        }
        let session_id = match &self.state.current_session_id {
            Some(id) => id.clone(),
            None => return,
        };

        // Check observation count
        let count = caboose_core::memory::observations::count_for_session(
            self.state.sessions.storage().conn(),
            &session_id,
        )
        .unwrap_or(0);

        if count < caboose_core::memory::extraction::MIN_OBSERVATIONS {
            return;
        }

        // Load observations
        let observations = match caboose_core::memory::observations::for_session(
            self.state.sessions.storage().conn(),
            &session_id,
        ) {
            Ok(obs) => obs,
            Err(_) => return,
        };

        // Load current memory
        let memory_ctx = self.state.memory.load_context();

        // Build extraction prompt
        let prompt = caboose_core::memory::extraction::build_extraction_prompt(
            &observations,
            memory_ctx.project.as_deref(),
        );

        // Send to provider (non-streaming, one-shot)
        if let Some(ref provider) = self.provider {
            let messages = vec![caboose_core::provider::Message {
                role: "user".to_string(),
                content: serde_json::json!(prompt),
            }];

            // Collect stream into response
            use tokio_stream::StreamExt;
            let mut response = String::new();
            let mut stream = provider.stream(&messages, &[]);
            while let Some(event) = stream.next().await {
                if let Ok(caboose_core::provider::StreamEvent::TextDelta(text)) = event {
                    response.push_str(&text);
                }
            }

            // Parse and append
            if let Some(new_lines) =
                caboose_core::memory::extraction::parse_extraction_response(&response)
            {
                let memory_path = self.state.memory.project_dir().join("MEMORY.md");
                if let Err(e) = caboose_core::memory::extraction::append_to_memory_file(
                    &memory_path,
                    &new_lines,
                ) {
                    tracing::warn!("Failed to append memories: {e}");
                }
            }
        }

        // Prune old observations
        let _ = caboose_core::memory::observations::prune(
            self.state.sessions.storage().conn(),
            memory_config.observation_retention_days,
        );
    }

    /// Extract memories, clean up cold storage, and clear all session state.
    /// Called when the user confirms a new session via the Confirm dialog,
    /// or directly from handle_shared_slash when no confirmation is needed.
    pub(super) async fn execute_new_session(&mut self) {
        self.extract_session_memories().await;
        if let Some(ref store) = self.state.agent.cold_store {
            let _ = store.cleanup();
        }
        self.reset_text_input_activity();
        // Clear all session state (must match session.new execute closure exactly)
        self.state.chat_messages.clear();
        self.state.input.clear();
        self.state.scroll_offset = 0;
        self.state.user_scrolled_up = false;
        self.state.session_title = None;
        self.state.session_title_source = None;
        self.state.title_rx = None;
        self.state.title_manually_set = false;
        self.state.current_session_id = None;
        self.state.modified_files.clear();
        self.state.file_baselines.clear();
        self.state.tool_counts.clear();
        self.state.focused_tool = None;
        self.state.pending_handoff = None;
        self.state.roundhouse_session = None;
        self.state.roundhouse_update_rx = None;
        self.state.roundhouse_synthesis_rx = None;
        self.state.roundhouse_critique_rx = None;
        self.state.roundhouse_model_add = false;
        self.state.pins.clear();
        self.state.pins_expanded = false;
        self.state.agent.cancel();
        self.state.agent.conversation.messages.clear();
        self.state.agent.turn_count = 0;
        self.state.session_cost = 0.0;
        self.state.session_input_tokens = 0;
        self.state.session_output_tokens = 0;
        self.state.agent.session_allows.clear();
        self.state.agent.handoff_prompted = false;
        self.state.expanded_messages.clear();
        self.state.expanded_thinking.clear();
        self.state.message_queue.clear();
        self.state.tool_exec_queue.clear();
        self.state.tool_exec_args.clear();
        self.state.tool_exec_results.clear();
        self.state.tool_exec_running_start = 0;
        self.state.tool_exec_pending_rx = None;
        self.state.skill_creation = None;
        self.state.handoff_agent_pending = false;
        self.state.attachments.clear();
        self.state.dialog_stack.base = crate::tui::dialog::Screen::Home;
        self.state.dialog_stack.clear();
    }
}
