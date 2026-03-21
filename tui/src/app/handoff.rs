use super::*;

impl App {
    /// Handle the /fork command — clone current session into a new one with context.
    pub(super) fn handle_fork_command(&mut self) {
        // Guard: need an active session
        let parent_id = match self.state.current_session_id.clone() {
            Some(id) => id,
            None => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "No active session to fork.".into(),
                });
                return;
            }
        };

        // Guard: need messages
        if self.state.chat_messages.is_empty() {
            self.state.chat_messages.push(ChatMessage::System {
                content: "Cannot fork an empty session.".into(),
            });
            return;
        }

        // Build handoff summary BEFORE switching sessions (needs current state)
        let user_msgs: Vec<&str> = self
            .state
            .chat_messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();

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
            session_id: Some(parent_id.as_str()),
            session_title: self.state.session_title.as_deref(),
            provider_name: Some(self.state.active_provider_name.as_str()),
            model_name: Some(self.state.active_model_name.as_str()),
            turn_count: self.state.agent.turn_count,
            user_messages: user_msgs,
            modified_files: &modified,
            tool_counts: &self.state.tool_counts,
            open_tasks,
            focus: None,
        };

        let handoff_summary = crate::skills::handoff::build_handoff_summary(&ctx);

        // Inherit parent title with " (fork)" suffix
        let fork_title = match &self.state.session_title {
            Some(t) => Some(format!("{t} (fork)")),
            None => Some("Untitled (fork)".to_string()),
        };

        // Count messages for fork metadata
        let message_count = match self.state.sessions.load_messages(&parent_id) {
            Ok(msgs) => msgs.len() as u32,
            Err(_) => 0,
        };

        // Create new session
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
        let new_session_id =
            match self
                .state
                .sessions
                .create(model, provider, Some(&parent_id), Some(message_count))
            {
                Ok(session) => session.id,
                Err(e) => {
                    self.state.chat_messages.push(ChatMessage::Error {
                        content: format!("Failed to create fork session: {e}"),
                    });
                    return;
                }
            };

        // Copy messages from parent to new session
        if let Err(e) = self
            .state
            .sessions
            .copy_messages(&parent_id, &new_session_id)
        {
            self.state.chat_messages.push(ChatMessage::Error {
                content: format!("Failed to copy messages to fork: {e}"),
            });
            return;
        }

        // Set fork title on the new session
        let title_session = crate::session::Session {
            id: new_session_id.clone(),
            title: fork_title,
            model: model.map(|s| s.to_string()),
            provider: provider.map(|s| s.to_string()),
            turn_count: 0,
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            parent_session_id: Some(parent_id.clone()),
            fork_message_count: Some(message_count),
            pins: vec![],
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
        };
        if let Err(e) = self.state.sessions.update(&title_session) {
            tracing::warn!("Failed to set fork title: {e}");
        }

        // Reset all session-scoped state (mirrors /new command)
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
        self.state.agent.cancel();
        self.state.agent.conversation.messages.clear();
        self.state.agent.turn_count = 0;
        self.state.agent.session_allows.clear();
        self.state.agent.handoff_prompted = false;
        self.state.dialog_stack.clear();

        // Restore the forked session (loads copied messages)
        self.restore_session(&new_session_id);

        // Guard: if restore failed
        if self.state.current_session_id.is_none() {
            return;
        }

        // Inject fork context
        let short_parent_id = if parent_id.len() > 8 {
            &parent_id[..8]
        } else {
            &parent_id
        };
        let fork_context = format!("[Forked from session {short_parent_id}]\n\n{handoff_summary}");

        // Persist as fork_context role (not displayed to user)
        self.persist_message("fork_context", &fork_context);

        // Inject into agent conversation as a User message
        self.state
            .agent
            .conversation
            .messages
            .push(crate::agent::conversation::Message {
                role: crate::agent::conversation::Role::User,
                content: crate::agent::conversation::Content::Text(fork_context),
                tool_call_id: None,
            });

        // Push system notification
        self.state.chat_messages.push(ChatMessage::System {
            content: format!(
                "Session forked from {short_parent_id}. You're now in a new branch with full conversation history."
            ),
        });
    }

    /// Handle the /handoff command — build summary and prompt for new session.
    pub(super) async fn handle_handoff_command(&mut self, args: &str) {
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

    pub(super) fn build_model_switch_handoff_context(
        &self,
        old_provider: &str,
        old_model: &str,
        new_provider: &str,
        new_model: &str,
    ) -> Option<String> {
        if self.state.current_session_id.is_none()
            || !has_meaningful_model_switch_context(
                &self.state.chat_messages,
                !self.state.modified_files.is_empty(),
                !self.state.tool_counts.is_empty(),
            )
        {
            return None;
        }

        let user_msgs: Vec<&str> = self
            .state
            .chat_messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::User { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();

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

        let focus = format!(
            "Continuing this existing session after switching models from {old_provider}/{old_model} to {new_provider}/{new_model}. Preserve the current goals, constraints, unfinished tasks, and file-change context."
        );
        let ctx = crate::skills::handoff::HandoffContext {
            session_id: self.state.current_session_id.as_deref(),
            session_title: self.state.session_title.as_deref(),
            provider_name: Some(new_provider),
            model_name: Some(new_model),
            turn_count: self.state.agent.turn_count,
            user_messages: user_msgs,
            modified_files: &modified,
            tool_counts: &self.state.tool_counts,
            open_tasks,
            focus: Some(focus.as_str()),
        };

        let summary = crate::skills::handoff::build_handoff_summary(&ctx);
        Some(format!(
            "[Model switch: {old_provider}/{old_model} -> {new_provider}/{new_model}]\n\n{summary}"
        ))
    }

    /// Spawn a handoff subagent with the selected model to consult another model
    /// with the current session context.
    pub(super) async fn spawn_handoff_agent(&mut self, provider_name: &str, model_id: &str) {
        // Build handoff summary
        let handoff = self.build_model_switch_handoff_context(
            &self.state.active_provider_name,
            &self.state.active_model_name,
            provider_name,
            model_id,
        );
        let summary = handoff.unwrap_or_else(|| {
            format!(
                "[Handoff to {provider_name}/{model_id}]\n\n\
                 No meaningful context to summarize — session is new or empty."
            )
        });

        // Build the task: handoff context + instruction to continue
        let task = format!(
            "{summary}\n\n\
             Review this handoff and continue where the previous model left off. \
             Focus on any open tasks or unfinished work described above."
        );

        let spawn_args = serde_json::json!({
            "task": task,
        });

        self.state.chat_messages.push(ChatMessage::System {
            content: format!("Handoff: spawning subagent with {provider_name}/{model_id}..."),
        });

        // Temporarily override active provider/model for spawn_agent_setup
        let orig_provider = self.state.active_provider_name.clone();
        let orig_model = self.state.active_model_name.clone();
        self.state.active_provider_name = provider_name.to_string();
        self.state.active_model_name = model_id.to_string();

        match self.spawn_agent_setup(&spawn_args).await {
            Ok((agent_id, input, provider, config, tx, task, branch, worktree_path, base_sha)) => {
                let placeholder_idx = self.state.chat_messages.len();
                self.state
                    .chat_messages
                    .push(ChatMessage::Tool(ToolMessage {
                        name: "spawn_agent".to_string(),
                        args: spawn_args.clone(),
                        output: None,
                        status: ToolStatus::Running,
                        expanded: false,
                        file_path: None,
                        diff_preview: None,
                        diff_expanded: false,
                    }));
                let tool_use_id = format!("handoff-{}", uuid::Uuid::new_v4());
                let handle = tokio::spawn(run_spawn_agent_task(
                    agent_id,
                    tool_use_id.clone(),
                    task,
                    branch,
                    worktree_path,
                    base_sha,
                    input,
                    provider,
                    config,
                    tx,
                ));
                self.state.spawn_agent_handles.push(SpawnAgentHandle {
                    tool_use_id,
                    arguments: spawn_args,
                    chat_placeholder_idx: placeholder_idx,
                    handle,
                });
            }
            Err(err_msg) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("Handoff agent failed: {err_msg}"),
                });
            }
        }

        // Restore original provider/model
        self.state.active_provider_name = orig_provider;
        self.state.active_model_name = orig_model;
    }
}
