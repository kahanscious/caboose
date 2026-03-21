use super::*;

impl App {
    /// Compute diff preview lines for a pending tool call. Returns None if not
    /// a write/edit/patch tool, or if preview is unavailable.
    pub(super) async fn compute_pending_diff(
        name: &str,
        args: &serde_json::Value,
    ) -> Option<Vec<String>> {
        use crate::tools::write::compute_diff_lines;
        match name {
            "edit_file" => {
                let old = args.get("old_string")?.as_str()?;
                let new = args.get("new_string")?.as_str()?;
                Some(compute_diff_lines(old, new))
            }
            "write_file" => {
                let path = args
                    .get("path")
                    .or_else(|| args.get("file_path"))
                    .and_then(|v| v.as_str())?;
                let new = args.get("content")?.as_str()?;
                let old = tokio::fs::read_to_string(path).await.ok();
                match old {
                    None => {
                        // New file or binary/unreadable — mark as new file, all lines added
                        let mut lines: Vec<String> =
                            new.lines().map(|l| format!("+ {l}")).collect();
                        lines.insert(0, "(new file)".to_string());
                        Some(lines)
                    }
                    Some(ref old_content) => {
                        let lines = compute_diff_lines(old_content, new);
                        if lines.is_empty() {
                            None // identical content — no diff to show
                        } else {
                            Some(lines)
                        }
                    }
                }
            }
            "apply_patch" => {
                // The diff input IS the diff — collect its content lines
                let diff_text = args.get("diff")?.as_str()?;
                let lines: Vec<String> = diff_text
                    .lines()
                    .filter(|l| !l.starts_with("---") && !l.starts_with("+++"))
                    .map(|l| l.to_string())
                    .collect();
                if lines.is_empty() { None } else { Some(lines) }
            }
            _ => None,
        }
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
    pub(super) fn handle_ask_user_key(&mut self, key: KeyCode) {
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
                    self.state.input.content()
                } else {
                    return; // nothing to submit
                };

                // Record answer
                let question_text = current_q.question;
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
    pub(super) fn start_tool_execution(&mut self) {
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
                        diff_preview: None,
                        diff_expanded: true,
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
    pub(super) async fn poll_tool_execution(&mut self) {
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
                    // If all done, finalize (also wait for spawn_agent handles)
                    if self.state.tool_exec_queue.is_empty()
                        && self.state.spawn_agent_handles.is_empty()
                    {
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
                    if self.state.tool_exec_queue.is_empty()
                        && self.state.spawn_agent_handles.is_empty()
                    {
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

    /// Fast setup for a spawn_agent call. Creates worktree, registers SubAgent,
    /// returns all owned data needed by the background task.
    pub(super) async fn spawn_agent_setup(
        &mut self,
        arguments: &serde_json::Value,
    ) -> Result<
        (
            uuid::Uuid,
            crate::sub_agent::executor::SubAgentInput,
            std::sync::Arc<dyn caboose_core::provider::Provider + Send + Sync>,
            caboose_core::config::Config,
            tokio::sync::mpsc::UnboundedSender<crate::sub_agent::SubAgentEvent>,
            String,
            String,
            std::path::PathBuf,
            String, // base_sha
        ),
        String,
    > {
        let task = match arguments.get("task").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return Err("spawn_agent: missing required parameter 'task'".to_string()),
        };

        // Look up custom agent definition if specified
        let agent_def = if let Some(name) = arguments.get("agent").and_then(|v| v.as_str()) {
            match self.state.agent_definitions.iter().find(|a| a.name == name) {
                Some(def) => Some(def.clone()),
                None => return Err(format!("spawn_agent: unknown agent '{name}'")),
            }
        } else {
            None
        };

        let use_worktree = agent_def
            .as_ref()
            .map(|d| d.worktree.unwrap_or(true))
            .unwrap_or(true);

        // Auto-clear terminal-state agents
        self.state.sub_agents.retain(|a| !a.state.is_terminal());
        self.state
            .agent_changes
            .retain(|c| self.state.sub_agents.iter().any(|a| a.id == c.agent_id));
        self.state.conflict_report = None;

        let (branch, worktree_path, base_sha) = if use_worktree {
            // Check gitignore
            if let Err(e) = crate::sub_agent::worktree::check_worktrees_ignored() {
                return Err(format!(
                    "Cannot spawn agent: .worktrees/ is not git-ignored ({e}). \
                     Add .worktrees/ to .gitignore first."
                ));
            }

            // Compute unique slug
            let used_slugs: Vec<String> = self
                .state
                .sub_agents
                .iter()
                .filter_map(|a| {
                    a.worktree_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix("agent-"))
                        .map(|s| s.to_string())
                })
                .collect();
            let slug = crate::sub_agent::worktree::unique_slug(&task, &used_slugs);
            let branch = crate::sub_agent::worktree::branch_name(&slug);
            let worktree_path = crate::sub_agent::worktree::worktree_path(&slug);

            // Clean up any stale branch/worktree from a previous run
            let branch_cleanup = branch.clone();
            let path_cleanup = worktree_path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _ = std::process::Command::new("git")
                    .args([
                        "worktree",
                        "remove",
                        "--force",
                        &path_cleanup.to_string_lossy(),
                    ])
                    .output();
                let _ = std::process::Command::new("git")
                    .args(["branch", "-D", &branch_cleanup])
                    .output();
            })
            .await;

            // Create worktree
            let path_clone = worktree_path.clone();
            let branch_clone = branch.clone();
            let worktree_result = tokio::task::spawn_blocking(move || {
                crate::sub_agent::worktree::create_worktree(&path_clone, &branch_clone)
            })
            .await;

            match worktree_result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(format!("spawn_agent: failed to create worktree: {e}")),
                Err(e) => return Err(format!("spawn_agent: worktree task panicked: {e}")),
            }

            // Capture HEAD SHA before any agent work begins
            let base_sha = tokio::task::spawn_blocking(|| {
                crate::sub_agent::worktree::current_head_sha()
                    .unwrap_or_else(|_| "unknown".to_string())
            })
            .await
            .unwrap_or_else(|_| "unknown".to_string());

            (branch, worktree_path, base_sha)
        } else {
            // No worktree — run in current working directory
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            (String::new(), cwd, String::new())
        };

        // Register SubAgent
        let agent_id = uuid::Uuid::new_v4();
        let (approval_tx, approval_rx) = tokio::sync::mpsc::unbounded_channel::<bool>();

        let agent_auto_approve = agent_def
            .as_ref()
            .and_then(|d| d.auto_approve)
            .unwrap_or(false);

        let base_sha_ret = base_sha.clone();
        self.state.sub_agents.push(crate::sub_agent::SubAgent {
            id: agent_id,
            task: task.clone(),
            branch: branch.clone(),
            worktree_path: worktree_path.clone(),
            base_sha: base_sha.clone(),
            state: crate::sub_agent::SubAgentState::Running,
            started_at: Some(std::time::Instant::now()),
            cost_usd: 0.0,
            stream: Vec::new(),
            approval_tx: Some(approval_tx),
            auto_approve: agent_auto_approve,
        });

        // Clamp permission mode
        use crate::agent::permission::Mode;
        let subagent_mode = match self.state.mode {
            Mode::Plan => PermissionMode::Default,
            Mode::Create => PermissionMode::Default,
            Mode::Chug => PermissionMode::Chug,
        };

        // Build system prompt: custom agent body or inherited
        let system_prompt = if let Some(ref def) = agent_def {
            let mut prompt = def.system_prompt.clone();
            let ws_block = workspace_system_prompt_block(&self.state.config.workspaces);
            if !ws_block.is_empty() {
                prompt.push_str(&ws_block);
            }
            prompt.push_str(
                "\n\nYou are a specialized sub-agent. Focus only on your assigned task. \
                 Do not modify files outside your task scope.",
            );
            prompt
        } else {
            self.state.agent.conversation.system_prompt.clone()
        };

        // Resolve model for custom agents
        let mut provider_name = self.state.active_provider_name.clone();
        let model_name = if let Some(ref def) = agent_def {
            if let Some(ref model_str) = def.model {
                if let Some((provider_override, model_override)) = model_str.split_once('/') {
                    provider_name = provider_override.to_string();
                    model_override.to_string()
                } else {
                    match crate::agents::resolve_model_shorthand(model_str) {
                        Some(resolved) => resolved.to_string(),
                        None => model_str.clone(),
                    }
                }
            } else {
                self.state.active_model_name.clone()
            }
        } else {
            self.state.active_model_name.clone()
        };

        let pricing = self.state.pricing.get(&model_name);
        let input = crate::sub_agent::executor::SubAgentInput {
            id: agent_id,
            task: task.clone(),
            worktree_path: worktree_path.clone(),
            system_prompt,
            permission_mode: subagent_mode,
            approval_rx,
            input_per_m: pricing.map(|p| p.input_per_m).unwrap_or(0.0),
            output_per_m: pricing.map(|p| p.output_per_m).unwrap_or(0.0),
            allowed_tools: agent_def.as_ref().and_then(|d| d.tools.clone()),
            denied_tools: agent_def.as_ref().and_then(|d| d.denied_tools.clone()),
        };

        // Get provider
        let provider_arc = match self
            .state
            .providers
            .get_provider_arc(Some(&provider_name), Some(&model_name))
        {
            Ok(p) => p,
            Err(e) => {
                if let Some(a) = self.state.sub_agents.iter_mut().find(|a| a.id == agent_id) {
                    a.state = crate::sub_agent::SubAgentState::Failed;
                    a.approval_tx = None;
                }
                let wt = worktree_path.clone();
                let br = branch.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    crate::sub_agent::worktree::remove_worktree(&wt, &br)
                })
                .await;
                return Err(format!(
                    "spawn_agent: no provider/model available for {}/{}: {e}",
                    provider_name, model_name
                ));
            }
        };

        let config = self.state.config.clone();
        let tx = match self.state.sub_agent_tx.clone() {
            Some(tx) => tx,
            None => {
                return Err(
                    "spawn_agent: internal error — subagent channel not initialized".to_string(),
                );
            }
        };

        Ok((
            agent_id,
            input,
            provider_arc,
            config,
            tx,
            task,
            branch,
            worktree_path,
            base_sha_ret,
        ))
    }

    /// Poll pending spawn_agent background tasks. Called each event-loop tick.
    /// When a task completes, injects its ToolResult into the agent conversation,
    /// updates the SubAgent state and chat placeholder, then cleans up.
    pub(super) async fn poll_spawn_agent_handles(&mut self) {
        let mut completed: Vec<usize> = Vec::new();
        for (i, sh) in self.state.spawn_agent_handles.iter().enumerate() {
            if sh.handle.is_finished() {
                completed.push(i);
            }
        }

        for i in completed.into_iter().rev() {
            let sh = self.state.spawn_agent_handles.remove(i);
            // is_finished() was true, so .await returns immediately
            let result = match sh.handle.await {
                Ok(r) => r,
                Err(e) => crate::sub_agent::SpawnAgentResult {
                    agent_id: uuid::Uuid::nil(),
                    tool_use_id: sh.tool_use_id.clone(),
                    task: String::new(),
                    result_text: format!("spawn_agent: task panicked: {e}"),
                    is_error: true,
                    produced_changes: false,
                    final_state: crate::sub_agent::SubAgentState::Failed,
                    cost_usd: 0.0,
                    changes: None,
                },
            };

            // Update SubAgent state
            if let Some(a) = self
                .state
                .sub_agents
                .iter_mut()
                .find(|a| a.id == result.agent_id)
            {
                a.state = result.final_state.clone();
                a.cost_usd = result.cost_usd;
                a.approval_tx = None;
            }

            // Stash AgentChanges for Review-state agents
            if matches!(result.final_state, crate::sub_agent::SubAgentState::Review)
                && let Some(changes) = result.changes
            {
                self.state.agent_changes.push(changes);
            }

            // Update chat placeholder
            if let Some(ChatMessage::Tool(tm)) =
                self.state.chat_messages.get_mut(sh.chat_placeholder_idx)
            {
                tm.status = if result.is_error {
                    ToolStatus::Failed
                } else {
                    ToolStatus::Success
                };
                tm.output = Some(result.result_text.clone());
            }

            // Inject ToolResult into agent conversation (skip for slash-invoked agents)
            if !result.tool_use_id.starts_with("slash-") {
                self.state
                    .agent
                    .conversation
                    .push(crate::agent::conversation::Message {
                        role: crate::agent::conversation::Role::User,
                        content: crate::agent::conversation::Content::Blocks(vec![
                            crate::agent::conversation::ContentBlock::ToolResult {
                                tool_use_id: result.tool_use_id.clone(),
                                content: result.result_text.clone(),
                                is_error: result.is_error,
                            },
                        ]),
                        tool_call_id: Some(result.tool_use_id),
                    });
            }

            // Emit system chat message
            if result.is_error {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("agent failed: {}", result.task),
                });
            } else if !result.produced_changes {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("agent produced no changes: {}", result.task),
                });
            } else if matches!(result.final_state, crate::sub_agent::SubAgentState::Review) {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("agent ready for review: {}", result.task),
                });
            } else {
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("agent done: {}", result.task),
                });
            }
        }

        let review_count = self
            .state
            .sub_agents
            .iter()
            .filter(|a| matches!(a.state, crate::sub_agent::SubAgentState::Review))
            .count();
        let has_live_subagents = self.state.sub_agents.iter().any(|a| {
            matches!(
                a.state,
                crate::sub_agent::SubAgentState::Running
                    | crate::sub_agent::SubAgentState::WaitingApproval { .. }
            )
        });
        if !has_live_subagents && review_count > 0 && self.state.conflict_report.is_none() {
            self.merge_reviewed_agents().await;
            let unresolved_review_ids: Vec<uuid::Uuid> = self
                .state
                .sub_agents
                .iter()
                .filter(|a| matches!(a.state, crate::sub_agent::SubAgentState::Review))
                .map(|a| a.id)
                .collect();
            if !unresolved_review_ids.is_empty() && self.state.conflict_report.is_none() {
                let tasks = self
                    .state
                    .sub_agents
                    .iter()
                    .filter(|a| unresolved_review_ids.contains(&a.id))
                    .map(|a| a.task.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ");
                for agent in &mut self.state.sub_agents {
                    if unresolved_review_ids.contains(&agent.id) {
                        agent.state = crate::sub_agent::SubAgentState::Conflict;
                    }
                }
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Conflict Analysis\n\n  ✗ reviewed agents remained unresolved after the merge sweep ({tasks})\n\nAgents were left in Conflict instead of allowing a silent continuation."
                    ),
                });
            }
        }

        // When all spawn handles are done and no other tools are pending, finalize
        if self.state.spawn_agent_handles.is_empty()
            && self.state.tool_exec_queue.is_empty()
            && self.state.tool_exec_pending_rx.is_none()
            && matches!(self.state.agent.state, AgentState::ExecutingTools)
        {
            self.finalize_tool_execution();
        }
    }

    /// Run the conflict detection sweep and merge agents that are ready.
    /// Called when all non-terminal agents have reached Review state.
    async fn merge_reviewed_agents(&mut self) {
        self.state.agent_changes.clear();

        // No-worktree agents won't have changes entries — move them to Done directly
        let no_worktree_ids: Vec<uuid::Uuid> = self
            .state
            .sub_agents
            .iter()
            .filter(|a| {
                matches!(a.state, crate::sub_agent::SubAgentState::Review) && a.branch.is_empty()
            })
            .map(|a| a.id)
            .collect();
        for id in no_worktree_ids {
            self.merge_single_agent(id).await;
        }

        let review_worktree_ids: std::collections::HashSet<uuid::Uuid> = self
            .state
            .sub_agents
            .iter()
            .filter(|a| {
                matches!(a.state, crate::sub_agent::SubAgentState::Review) && !a.branch.is_empty()
            })
            .map(|a| a.id)
            .collect();
        let changes = self.collect_review_agent_changes().await;
        let analyzed_ids: std::collections::HashSet<uuid::Uuid> =
            changes.iter().map(|c| c.agent_id).collect();
        let missing_ids: std::collections::HashSet<uuid::Uuid> = review_worktree_ids
            .difference(&analyzed_ids)
            .copied()
            .collect();

        if !missing_ids.is_empty() {
            for agent in &mut self.state.sub_agents {
                if missing_ids.contains(&agent.id) {
                    agent.state = crate::sub_agent::SubAgentState::Conflict;
                }
            }
            let tasks = self
                .state
                .sub_agents
                .iter()
                .filter(|a| missing_ids.contains(&a.id))
                .map(|a| a.task.as_str())
                .collect::<Vec<_>>()
                .join(" | ");
            self.state.chat_messages.push(ChatMessage::System {
                content: format!(
                    "Conflict Analysis\n\n  ✗ unable to analyze all reviewed agent diffs ({tasks})\n\nAgents were left in Conflict instead of being auto-merged."
                ),
            });
            return;
        }

        if changes.len() <= 1 {
            // Single agent or no agents — skip cross-agent check, merge directly
            for agent_change in &changes {
                self.merge_single_agent(agent_change.agent_id).await;
            }
            return;
        }

        // Run cross-agent check
        let report = crate::sub_agent::conflict::cross_agent_check(&changes);
        let overlap_ids: std::collections::HashSet<uuid::Uuid> = report
            .overlaps
            .iter()
            .flat_map(|o| o.participants.iter().map(|p| p.agent_id))
            .collect();

        // Merge policy varies by permission mode:
        // - Chug: auto-merge AutoMerge + AutoReconcile, hold only RequiresReview
        // - Create/Plan: auto-merge only AutoMerge, hold AutoReconcile + RequiresReview
        let is_chug = matches!(self.state.mode, crate::agent::permission::Mode::Chug);
        let needs_hold = |o: &crate::sub_agent::conflict::Overlap| -> bool {
            match o.resolution {
                crate::sub_agent::conflict::OverlapResolution::RequiresReview => true,
                crate::sub_agent::conflict::OverlapResolution::AutoReconcile => !is_chug,
                crate::sub_agent::conflict::OverlapResolution::AutoMerge => false,
            }
        };
        let review_ids: std::collections::HashSet<uuid::Uuid> = report
            .overlaps
            .iter()
            .filter(|o| needs_hold(o))
            .flat_map(|o| o.participants.iter().map(|p| p.agent_id))
            .collect();
        let has_holds = !review_ids.is_empty();

        if report.overlaps.is_empty() {
            for agent_change in &changes {
                self.merge_single_agent(agent_change.agent_id).await;
            }
        } else if !has_holds {
            // Only safe overlaps were detected — auto-merge them.
            let warn_text = crate::sub_agent::conflict::format_conflict_report_text(&report);
            self.state
                .chat_messages
                .push(ChatMessage::System { content: warn_text });
            for agent_change in &changes {
                self.merge_single_agent(agent_change.agent_id).await;
            }
            self.state.chat_messages.push(ChatMessage::System {
                content: format!(
                    "Agent coordination\n\nAutomatically reconciled and merged {} reviewed agent result(s).",
                    changes.len()
                ),
            });
        } else {
            // Overlaps need user review — auto-merge safe participants and hold the rest.
            let report_text = crate::sub_agent::conflict::format_conflict_report_text(&report);
            self.state.chat_messages.push(ChatMessage::System {
                content: report_text,
            });

            for agent_change in &changes {
                if !review_ids.contains(&agent_change.agent_id) {
                    self.merge_single_agent(agent_change.agent_id).await;
                }
            }

            if !overlap_ids.is_empty() {
                self.state.conflict_report = Some(report);
            }
        }
    }

    async fn collect_review_agent_changes(&self) -> Vec<crate::sub_agent::conflict::AgentChanges> {
        let review_agents: Vec<(uuid::Uuid, String, String, std::path::PathBuf, String)> = self
            .state
            .sub_agents
            .iter()
            .filter(|a| {
                matches!(a.state, crate::sub_agent::SubAgentState::Review) && !a.branch.is_empty()
            })
            .map(|a| {
                (
                    a.id,
                    a.task.clone(),
                    a.branch.clone(),
                    a.worktree_path.clone(),
                    a.base_sha.clone(),
                )
            })
            .collect();

        let mut changes = Vec::new();
        for (agent_id, task, branch, worktree_path, base_sha) in review_agents {
            let diff_output = tokio::task::spawn_blocking({
                let base = base_sha.clone();
                let br = branch.clone();
                move || crate::sub_agent::worktree::run_diff(&base, &br)
            })
            .await;

            let Ok(Ok(output)) = diff_output else {
                continue;
            };

            let mut files = crate::sub_agent::conflict::parse_diff_hunks(&output);
            if files.is_empty() {
                continue;
            }
            for file in &mut files {
                let base_content =
                    crate::sub_agent::worktree::read_file_at_commit(&base_sha, &file.path)
                        .ok()
                        .flatten();
                let new_content =
                    crate::sub_agent::worktree::read_worktree_file(&worktree_path, &file.path)
                        .ok()
                        .flatten();
                crate::sub_agent::conflict::enrich_file_change_semantics(
                    file,
                    base_content.as_deref(),
                    new_content.as_deref(),
                );
            }

            changes.push(crate::sub_agent::conflict::AgentChanges {
                agent_id,
                task,
                files,
            });
        }

        changes
    }

    /// Merge a single agent's branch and clean up its worktree.
    pub(super) async fn merge_single_agent(&mut self, agent_id: uuid::Uuid) {
        // Extract data upfront to avoid borrow issues with async
        let (branch, worktree_path) = match self.state.sub_agents.iter().find(|a| a.id == agent_id)
        {
            Some(a) => (a.branch.clone(), a.worktree_path.clone()),
            None => return,
        };

        // No-worktree agents: skip merge, go straight to Done
        if branch.is_empty() {
            if let Some(a) = self.state.sub_agents.iter_mut().find(|a| a.id == agent_id) {
                a.state = crate::sub_agent::SubAgentState::Done;
            }
            return;
        }

        let branch_for_merge = branch.clone();
        let merge_result = tokio::task::spawn_blocking(move || {
            crate::sub_agent::worktree::merge_branch(&branch_for_merge)
        })
        .await;

        match merge_result {
            Ok(Ok(())) => {
                // Clean up worktree
                let wp = worktree_path;
                let br = branch;
                let _ = tokio::task::spawn_blocking(move || {
                    crate::sub_agent::worktree::remove_worktree(&wp, &br)
                })
                .await;

                if let Some(a) = self.state.sub_agents.iter_mut().find(|a| a.id == agent_id) {
                    a.state = crate::sub_agent::SubAgentState::Done;
                }
            }
            Ok(Err(_)) => {
                if let Some(a) = self.state.sub_agents.iter_mut().find(|a| a.id == agent_id) {
                    a.state = crate::sub_agent::SubAgentState::Conflict;
                }
            }
            Err(_) => {
                if let Some(a) = self.state.sub_agents.iter_mut().find(|a| a.id == agent_id) {
                    a.state = crate::sub_agent::SubAgentState::Failed;
                }
            }
        }
    }

    /// Poll for completed background MCP server connections.
    pub(super) fn poll_mcp_connections(&mut self) {
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
    pub(super) async fn poll_circuit_events(&mut self) {
        use crate::circuits::runner::CircuitEvent;
        use caboose_core::provider::{Message, StreamEvent};
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

                        while let Some(event_result) = stream.next().await {
                            match event_result {
                                Ok(StreamEvent::TextDelta(text)) => {
                                    response.push_str(&text);
                                }
                                Ok(StreamEvent::Done { .. }) => {
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

                        let _ = event_tx.send(CircuitEvent::TickCompleted {
                            circuit_id,
                            output: response,
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
    pub(super) fn cancel_all_operations(&mut self) {
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
                // Inject cancelled tool_results into the conversation for all
                // tool_use blocks that haven't received results yet. Without
                // these the API rejects the next request (orphaned tool_use).
                let completed_count = self.state.tool_exec_results.len();
                for tc in self
                    .state
                    .agent
                    .pending_tool_calls
                    .iter()
                    .skip(completed_count)
                {
                    self.state
                        .agent
                        .conversation
                        .push(crate::agent::conversation::Message {
                            role: crate::agent::conversation::Role::User,
                            content: crate::agent::conversation::Content::Blocks(vec![
                                crate::agent::conversation::ContentBlock::ToolResult {
                                    tool_use_id: tc.id.clone(),
                                    content: "Cancelled by user".to_string(),
                                    is_error: true,
                                },
                            ]),
                            tool_call_id: Some(tc.id.clone()),
                        });
                }
                // Clear remaining queued tools
                self.state.tool_exec_queue.clear();
                self.state.tool_exec_results.clear();
                self.state.tool_exec_args.clear();
                self.state.agent.pending_tool_calls.clear();
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
                // Inject cancelled tool_results into the conversation for all
                // pending tool_use blocks so the API doesn't reject the next turn.
                for tc in &self.state.agent.pending_tool_calls {
                    self.state
                        .agent
                        .conversation
                        .push(crate::agent::conversation::Message {
                            role: crate::agent::conversation::Role::User,
                            content: crate::agent::conversation::Content::Blocks(vec![
                                crate::agent::conversation::ContentBlock::ToolResult {
                                    tool_use_id: tc.id.clone(),
                                    content: "Cancelled by user".to_string(),
                                    is_error: true,
                                },
                            ]),
                            tool_call_id: Some(tc.id.clone()),
                        });
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

        // Abort and clear spawn_agent background tasks so their results
        // don't leak into the conversation after the user has moved on.
        for sh in self.state.spawn_agent_handles.drain(..) {
            sh.handle.abort();
        }

        // Clear all sub-agent state so the user gets a clean slate.
        // Dropping approval_tx senders causes background agent tasks to stop
        // waiting for approval and terminate gracefully.
        self.state.sub_agents.clear();
        self.state.sub_agent_approval_showing = None;
        self.state.sub_agent_pending_approvals.clear();
        self.state.agent_stream_overlay = None;
        self.state.sidebar_focused = false;
        self.state.sidebar_agent_selected = 0;

        // Remove task outlines — they'll be repopulated on the next prompt
        self.state
            .chat_messages
            .retain(|m| !matches!(m, ChatMessage::TaskOutline(_)));
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
        let workspace_paths: Vec<&str> = self
            .state
            .config
            .workspaces
            .values()
            .map(|cfg| cfg.path.as_str())
            .collect();
        let decision = crate::agent::permission::check_permission(
            &self.state.agent.permission_mode,
            &tc.name,
            &tc.arguments,
            &self.state.agent.allow_list,
            &self.state.agent.deny_list,
            &self.state.agent.session_allows,
            tool_permission,
            Some(&self.state.primary_root),
            &workspace_paths,
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
                let tool_name_clone = tool_name;
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
                let tool_name_clone = tool_name;
                tokio::spawn(async move {
                    crate::hooks::fire_hooks_for_tool(&hooks, context, &tool_name_clone).await;
                });
            }
        }

        self.state.tool_exec_results.push(result);
    }

    /// Called after all queued tools have executed — tracks files,
    /// records observations, and continues the agent loop.
    pub(super) fn finalize_tool_execution(&mut self) {
        let results = std::mem::take(&mut self.state.tool_exec_results);
        let tool_args = std::mem::take(&mut self.state.tool_exec_args);

        // --- Circuit breaker: track consecutive shell command failures ---
        let mut tripped_commands: Vec<String> = Vec::new();
        for result in &results {
            if result.tool_name.as_deref() == Some("run_command") {
                let cmd_str = tool_args
                    .get(&result.tool_use_id)
                    .and_then(|v| v.get("command"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !cmd_str.is_empty() {
                    let tripped = self
                        .state
                        .agent
                        .record_command_result(cmd_str, result.is_error);
                    if tripped {
                        tripped_commands.push(cmd_str.to_string());
                    }
                }
            }
        }
        // If any command hit the retry limit, inject a user-role system message
        // telling the model to stop retrying and explain the problem.
        if !tripped_commands.is_empty() {
            let cmds = tripped_commands.join(", ");
            let msg = format!(
                "[system] The following command(s) have failed {} consecutive times: {}. \
                 Do not retry them. Instead, explain what is going wrong and ask the user \
                 how they would like to proceed.",
                crate::agent::MAX_COMMAND_RETRIES,
                cmds,
            );
            self.state
                .agent
                .conversation
                .push(crate::agent::conversation::Message {
                    role: crate::agent::conversation::Role::User,
                    content: crate::agent::conversation::Content::Text(msg),
                    tool_call_id: None,
                });
        }

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
                let _ = caboose_core::memory::observations::record(
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
}
