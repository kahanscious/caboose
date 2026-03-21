use super::*;

impl App {
    pub(super) fn handle_roundhouse_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // Ctrl+C or Escape: immediately exit roundhouse and return to Chat
        if (key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
            || key == KeyCode::Esc
        {
            self.clear_roundhouse_session();
            self.state.dialog_stack.base = Screen::Chat;
            return;
        }

        let session = match &mut self.state.roundhouse_session {
            Some(s) => s,
            None => return,
        };

        // Annotation input mode — capture all keys
        if session.annotation_input.is_some() {
            match key {
                KeyCode::Enter => {
                    if let Some(text) = session.annotation_input.take() {
                        session.add_annotation(text);
                    }
                }
                KeyCode::Esc => {
                    session.annotation_input = None;
                }
                KeyCode::Backspace => {
                    if let Some(ref mut input) = session.annotation_input {
                        input.pop();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(ref mut input) = session.annotation_input {
                        input.push(c);
                    }
                }
                _ => {}
            }
            return;
        }

        // Check if we need to start critique/synthesis AFTER the match
        // to avoid holding a mutable borrow on self.state.roundhouse_session
        // while calling methods on self.
        let mut action: Option<&str> = None;
        let mut model_switched = false;
        match key {
            // Model navigation — reset to auto-scroll bottom on switch
            KeyCode::Char('j') => {
                session.select_next_model();
                model_switched = true;
            }
            KeyCode::Char('k') => {
                session.select_prev_model();
                model_switched = true;
            }

            // Gate actions — only active during review phases
            KeyCode::Char('c')
                if session.phase == crate::roundhouse::RoundhousePhase::ReviewingPlans
                    && session.critique_enabled =>
            {
                session.phase = crate::roundhouse::RoundhousePhase::Critiquing;
                action = Some("critique");
            }
            KeyCode::Char('s')
                if matches!(
                    session.phase,
                    crate::roundhouse::RoundhousePhase::ReviewingPlans
                        | crate::roundhouse::RoundhousePhase::ReviewingCritiques
                ) =>
            {
                session.phase = crate::roundhouse::RoundhousePhase::Synthesizing;
                action = Some("synthesis");
            }
            KeyCode::Char('a')
                if matches!(
                    session.phase,
                    crate::roundhouse::RoundhousePhase::ReviewingPlans
                        | crate::roundhouse::RoundhousePhase::ReviewingCritiques
                ) =>
            {
                session.annotation_input = Some(String::new());
            }
            KeyCode::Char('q') => {
                // Handled after match to avoid borrow conflict
                action = Some("quit");
            }

            _ => {}
        }

        // Now session borrow is dropped — safe to access other self.state fields
        if model_switched {
            // Jump back to auto-scroll bottom when switching models
            self.state.user_scrolled_up = false;
        }

        match action {
            Some("critique") => self.start_roundhouse_critique(),
            Some("synthesis") => self.start_roundhouse_synthesis(),
            Some("quit") => {
                self.clear_roundhouse_session();
                self.state.dialog_stack.base = Screen::Chat;
            }
            _ => {}
        }
    }

    pub(super) async fn handle_home_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // Ctrl+C always goes to quit logic, even when a picker/dropdown is open
        if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(sel) = self.state.text_selection.take() {
                let text = self.extract_selected_text(&sel);
                if !text.is_empty() {
                    let _ = crate::clipboard::copy_to_clipboard(&text);
                } else if !self.state.input.is_empty() {
                    self.clear_composer_input();
                } else {
                    self.request_quit();
                }
            } else if !self.state.input.is_empty() {
                self.clear_composer_input();
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
                &self.state.agent_definitions,
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
                        &self.state.agent_definitions,
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
            (KeyCode::Char('v'), m) if m.contains(KeyModifiers::CONTROL) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new()
                    && let Ok(text) = clipboard.get_text()
                {
                    self.handle_paste(&text);
                }
            }
            (KeyCode::Char('v'), m) if m.contains(KeyModifiers::SUPER) => {
                // Let terminal/platform paste handling deliver the real text
                // without also inserting the shortcut key as plain input.
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                self.state.dialog_stack.push(DialogKind::FileBrowser(
                    crate::tui::file_browser::FileBrowserState::new(cwd),
                ));
            }
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                if self.state.model_supports_thinking {
                    self.state.thinking_mode = self.state.thinking_mode.toggle();
                    if let Some(ref provider) = self.provider {
                        provider.set_thinking_mode(self.state.thinking_mode);
                    }
                }
            }
            (KeyCode::Enter, KeyModifiers::SHIFT)
            | (KeyCode::Enter, KeyModifiers::ALT)
            | (KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                self.state.input.insert_newline();
                self.reset_text_input_activity();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.should_treat_enter_as_paste_newline() {
                    self.state.input.insert_newline();
                    self.record_text_input_activity(1);
                    return;
                }
                if !self.state.input.is_empty() {
                    let mut message = self.state.input.content();
                    self.state.history.push(message.clone());
                    self.state.history.save();
                    self.state.input.clear();
                    self.reset_text_input_activity();

                    // Handle slash commands via registry
                    let trimmed = message.trim();
                    if let Some(slash) = trimmed.strip_prefix('/') {
                        if let Some(title_rest) = slash.strip_prefix("title ") {
                            let new_title = title_rest.trim().to_string();
                            if !new_title.is_empty() {
                                self.state.session_title = Some(new_title.clone());
                                self.state.title_manually_set = true;
                                self.update_session_meta();
                                self.state.chat_messages.push(ChatMessage::System {
                                    content: format!("Session renamed to \"{new_title}\""),
                                });
                            }
                            return;
                        }
                        if self.handle_shared_slash(slash).await {
                            // /pins on home screen should switch to chat
                            if slash == "pins" {
                                self.state.dialog_stack.base = crate::tui::dialog::Screen::Chat;
                            }
                            return;
                        }

                        // Try skill resolution after shared slash dispatch
                        {
                            // Reload skills and agents from disk before resolution (picks up external changes)
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
                            let command_names: Vec<&str> = self
                                .state
                                .commands
                                .slash_commands()
                                .filter_map(|c| c.slash)
                                .collect();
                            let project_agents_dir = std::path::PathBuf::from(".caboose/agents");
                            let global_agents_dir = dirs::config_dir()
                                .map(|d| d.join("caboose/agents"))
                                .unwrap_or_else(|| std::path::PathBuf::from(".caboose/agents"));
                            self.state.agent_definitions = crate::agents::load_agents_validated(
                                Some(&project_agents_dir),
                                Some(&global_agents_dir),
                                &command_names,
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
                                &self.state.agent_definitions,
                                &self.state.skills,
                            );
                            if let crate::skills::SlashResolution::Agent(agent_def) = resolution {
                                // Invoke agent via spawn_agent with the remaining args as the task
                                let task_text = if args.is_empty() {
                                    format!("Run agent: {}", agent_def.name)
                                } else {
                                    args.to_string()
                                };
                                let spawn_args = serde_json::json!({
                                    "task": task_text,
                                    "agent": agent_def.name,
                                });
                                if !self.require_provider() {
                                    return;
                                }
                                self.state.dialog_stack.base = Screen::Chat;
                                self.state.dialog_stack.clear();
                                match self.spawn_agent_setup(&spawn_args).await {
                                    Ok((
                                        agent_id,
                                        input,
                                        provider,
                                        config,
                                        tx,
                                        task,
                                        branch,
                                        worktree_path,
                                        base_sha,
                                    )) => {
                                        let placeholder_idx = self.state.chat_messages.len();
                                        self.state.chat_messages.push(ChatMessage::Tool(
                                            ToolMessage {
                                                name: "spawn_agent".to_string(),
                                                args: spawn_args.clone(),
                                                output: None,
                                                status: ToolStatus::Running,
                                                expanded: false,
                                                file_path: None,
                                                diff_preview: None,
                                                diff_expanded: false,
                                            },
                                        ));
                                        let tool_use_id = format!("slash-{}", uuid::Uuid::new_v4());
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
                                            content: format!("Agent failed: {err_msg}"),
                                        });
                                    }
                                }
                                return;
                            }
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

                    // ! shell shortcut — run command directly without LLM
                    if let Some(cmd) = trimmed.strip_prefix('!') {
                        let cmd = cmd.trim();
                        if !cmd.is_empty() {
                            self.state.dialog_stack.base = Screen::Chat;
                            self.state.dialog_stack.clear();
                            self.state.chat_messages.push(ChatMessage::System {
                                content: format!("$ {cmd}"),
                            });
                            self.state.user_scrolled_up = false;
                            // Try sh -c first (Unix, macOS, Windows+Git Bash).
                            // On bare Windows (no sh in PATH), fall back to cmd /C.
                            let shell_result = tokio::process::Command::new("sh")
                                .arg("-c")
                                .arg(cmd)
                                .output()
                                .await;
                            let shell_result = match shell_result {
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    #[cfg(windows)]
                                    {
                                        tokio::process::Command::new("cmd")
                                            .arg("/C")
                                            .arg(cmd)
                                            .output()
                                            .await
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        Err(e)
                                    }
                                }
                                other => other,
                            };
                            match shell_result {
                                Ok(output) => {
                                    let stdout = String::from_utf8_lossy(&output.stdout);
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    let mut result = String::new();
                                    if !stdout.is_empty() {
                                        result.push_str(&stdout);
                                    }
                                    if !stderr.is_empty() {
                                        if !result.is_empty() {
                                            result.push('\n');
                                        }
                                        result.push_str(&stderr);
                                    }
                                    if result.is_empty() {
                                        result.push_str("(no output)");
                                    }
                                    // Truncate if very long
                                    let lines: Vec<&str> = result.lines().collect();
                                    let display = if lines.len() > 200 {
                                        let mut truncated: String = lines[..200].join("\n");
                                        truncated.push_str(&format!(
                                            "\n\n... ({} more lines truncated)",
                                            lines.len() - 200
                                        ));
                                        truncated
                                    } else {
                                        result.to_string()
                                    };
                                    let exit_code = output.status.code().unwrap_or(-1);
                                    let content = if exit_code == 0 {
                                        format!("```\n{display}\n```")
                                    } else {
                                        format!("```\n{display}\n```\n[exit code: {exit_code}]")
                                    };
                                    self.state
                                        .chat_messages
                                        .push(ChatMessage::System { content });
                                }
                                Err(e) => {
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: format!("Failed to run command: {e}"),
                                    });
                                }
                            }
                        }
                        return;
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
            (KeyCode::Char(c), m) if Self::should_insert_text(m) => {
                self.state.history.reset();
                self.state.input.insert_char(c);
                self.record_text_input_activity(c.len_utf8());
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

    pub(super) async fn handle_chat_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // If Roundhouse is in an active phase, delegate all keys to its handler
        let roundhouse_active = roundhouse_active(&self.state);
        if roundhouse_active {
            self.handle_roundhouse_key(key, modifiers);
            return;
        }

        // Esc or Ctrl+C while roundhouse exists (passive phases) — clear it
        if self.state.roundhouse_session.is_some()
            && ((key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL))
                || key == KeyCode::Esc)
        {
            self.clear_roundhouse_session();
            self.state.dialog_stack.base = Screen::Chat;
            return;
        }

        // Ctrl+C always goes to quit/cancel logic, even when a picker/dropdown is open
        if key == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(sel) = self.state.text_selection.take() {
                let text = self.extract_selected_text(&sel);
                if !text.is_empty() {
                    let _ = crate::clipboard::copy_to_clipboard(&text);
                } else if !self.state.input.is_empty() {
                    self.clear_composer_input();
                } else if matches!(self.state.agent.state, AgentState::PendingApproval { .. }) {
                    self.cancel_all_operations();
                } else if !matches!(self.state.agent.state, AgentState::Idle) {
                    self.cancel_all_operations();
                    self.request_quit();
                } else {
                    self.request_quit();
                }
            } else if !self.state.input.is_empty() {
                self.clear_composer_input();
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

        // Sidebar agent navigation (when agents exist and sidebar is focused)
        if !self.state.sub_agents.is_empty() && self.state.sidebar_visible {
            // Alt+A toggles sidebar focus
            if key == KeyCode::Char('a') && modifiers.contains(KeyModifiers::ALT) {
                self.state.sidebar_focused = !self.state.sidebar_focused;
                return;
            }
            if self.state.sidebar_focused {
                match key {
                    KeyCode::Up => {
                        self.state.sidebar_agent_selected =
                            self.state.sidebar_agent_selected.saturating_sub(1);
                        return;
                    }
                    KeyCode::Down => {
                        let max = self.state.sub_agents.len().saturating_sub(1);
                        if self.state.sidebar_agent_selected < max {
                            self.state.sidebar_agent_selected += 1;
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        let idx = self.state.sidebar_agent_selected;
                        self.state.agent_stream_overlay = Some(idx);
                        self.state.dialog_stack.push(
                            crate::tui::dialog::DialogKind::AgentStreamOverlay(
                                crate::tui::dialog::AgentStreamOverlayState::new(),
                            ),
                        );
                        return;
                    }
                    KeyCode::Esc => {
                        self.state.sidebar_focused = false;
                        return;
                    }
                    _ => {}
                }
            }
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
                &self.state.agent_definitions,
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
                        &self.state.agent_definitions,
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
                    self.state.title_rx = None;
                    self.state.title_manually_set = false;
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

        // Hover-copy: y key copies hovered code block or assistant message
        if key == KeyCode::Char('y') && !roundhouse_active {
            if self.state.hovered_code_block.is_some() {
                self.copy_hovered_code_block();
                return;
            }
            if self.state.hovered_message.is_some() {
                self.copy_hovered_message();
                return;
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
            (KeyCode::Char('v'), m) if m.contains(KeyModifiers::CONTROL) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new()
                    && let Ok(text) = clipboard.get_text()
                {
                    self.handle_paste(&text);
                }
            }
            (KeyCode::Char('v'), m) if m.contains(KeyModifiers::SUPER) => {
                // Let terminal/platform paste handling deliver the real text
                // without also inserting the shortcut key as plain input.
            }
            (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
                let cwd = std::env::current_dir().unwrap_or_default();
                self.state.dialog_stack.push(DialogKind::FileBrowser(
                    crate::tui::file_browser::FileBrowserState::new(cwd),
                ));
            }
            (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                if self.state.model_supports_thinking {
                    self.state.thinking_mode = self.state.thinking_mode.toggle();
                    if let Some(ref provider) = self.provider {
                        provider.set_thinking_mode(self.state.thinking_mode);
                    }
                }
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
                self.reset_text_input_activity();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if self.should_treat_enter_as_paste_newline() {
                    self.state.input.insert_newline();
                    self.record_text_input_activity(1);
                    return;
                }
                if !self.state.input.is_empty()
                    && !matches!(self.state.agent.state, AgentState::Idle)
                    && self.state.message_queue.len() < 3
                {
                    // Agent is busy — queue the message
                    let content = self.state.input.content();
                    self.state.history.push(content.clone());
                    self.state.input.clear();
                    self.reset_text_input_activity();
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
                    self.reset_text_input_activity();
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
                                self.state.title_manually_set = true;
                                self.update_session_meta();
                                self.state.chat_messages.push(ChatMessage::System {
                                    content: format!("Session renamed to \"{new_title}\""),
                                });
                            }
                            return;
                        }
                        // Chat-only slash commands
                        if slash == "cancel" && self.state.skill_creation.is_some() {
                            self.state.skill_creation = None;
                            self.state.chat_messages.push(ChatMessage::System {
                                content: "Skill creation cancelled.".into(),
                            });
                            return;
                        }
                        if slash == "fork" {
                            self.handle_fork_command();
                            return;
                        }
                        if slash == "handoff" || slash.starts_with("handoff ") {
                            let args = slash.strip_prefix("handoff").unwrap_or("").trim();
                            self.handle_handoff_command(args).await;
                            return;
                        }
                        // Shared slash commands
                        if self.handle_shared_slash(slash).await {
                            return;
                        }
                    }

                    // ! shell shortcut — run command directly without LLM
                    if let Some(cmd) = trimmed.strip_prefix('!') {
                        let cmd = cmd.trim();
                        if !cmd.is_empty() {
                            self.state.dialog_stack.base = Screen::Chat;
                            self.state.dialog_stack.clear();
                            self.state.chat_messages.push(ChatMessage::System {
                                content: format!("$ {cmd}"),
                            });
                            self.state.user_scrolled_up = false;
                            // Try sh -c first (Unix, macOS, Windows+Git Bash).
                            // On bare Windows (no sh in PATH), fall back to cmd /C.
                            let shell_result = tokio::process::Command::new("sh")
                                .arg("-c")
                                .arg(cmd)
                                .output()
                                .await;
                            let shell_result = match shell_result {
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    #[cfg(windows)]
                                    {
                                        tokio::process::Command::new("cmd")
                                            .arg("/C")
                                            .arg(cmd)
                                            .output()
                                            .await
                                    }
                                    #[cfg(not(windows))]
                                    {
                                        Err(e)
                                    }
                                }
                                other => other,
                            };
                            match shell_result {
                                Ok(output) => {
                                    let stdout = String::from_utf8_lossy(&output.stdout);
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    let mut result = String::new();
                                    if !stdout.is_empty() {
                                        result.push_str(&stdout);
                                    }
                                    if !stderr.is_empty() {
                                        if !result.is_empty() {
                                            result.push('\n');
                                        }
                                        result.push_str(&stderr);
                                    }
                                    if result.is_empty() {
                                        result.push_str("(no output)");
                                    }
                                    let lines: Vec<&str> = result.lines().collect();
                                    let display = if lines.len() > 200 {
                                        let mut truncated: String = lines[..200].join("\n");
                                        truncated.push_str(&format!(
                                            "\n\n... ({} more lines truncated)",
                                            lines.len() - 200
                                        ));
                                        truncated
                                    } else {
                                        result.to_string()
                                    };
                                    let exit_code = output.status.code().unwrap_or(-1);
                                    let content = if exit_code == 0 {
                                        format!("```\n{display}\n```")
                                    } else {
                                        format!("```\n{display}\n```\n[exit code: {exit_code}]")
                                    };
                                    self.state
                                        .chat_messages
                                        .push(ChatMessage::System { content });
                                }
                                Err(e) => {
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: format!("Failed to run command: {e}"),
                                    });
                                }
                            }
                        }
                        return;
                    }

                    if !self.require_provider() {
                        self.state.input.set(&message);
                        return;
                    }

                    // Clear stale task outlines — agent will recreate if still relevant
                    self.state
                        .chat_messages
                        .retain(|m| !matches!(m, ChatMessage::TaskOutline(_)));

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
                        match crate::attachment::read_image_attachment(
                            &full_path,
                            &self.images_config(),
                        ) {
                            Ok(att) => {
                                if let Some(ref info) = att.compression {
                                    let msg = format!(
                                        "Compressed {}: {} → {}",
                                        att.display_name,
                                        crate::attachment::format_size(info.original_size),
                                        crate::attachment::format_size(info.compressed_size),
                                    );
                                    self.state
                                        .chat_messages
                                        .push(ChatMessage::System { content: msg });
                                }
                                self.state.attachments.push(att);
                            }
                            Err(e) => {
                                self.state.chat_messages.push(ChatMessage::Error {
                                    content: format!("Failed to attach {path_str}: {e}"),
                                });
                            }
                        }
                    }

                    // Also detect bare image paths in the message text (e.g. from drag-and-drop
                    // that landed in the input area instead of triggering Event::Paste)
                    let (bare_paths, cleaned_text) =
                        crate::attachment::extract_bare_image_paths(&msg_to_send);
                    if !bare_paths.is_empty() {
                        msg_to_send = cleaned_text;
                        for path in &bare_paths {
                            match crate::attachment::read_image_attachment(
                                path,
                                &self.images_config(),
                            ) {
                                Ok(att) => {
                                    if let Some(ref info) = att.compression {
                                        let msg = format!(
                                            "Compressed {}: {} → {}",
                                            att.display_name,
                                            crate::attachment::format_size(info.original_size),
                                            crate::attachment::format_size(info.compressed_size),
                                        );
                                        self.state
                                            .chat_messages
                                            .push(ChatMessage::System { content: msg });
                                    }
                                    self.state.attachments.push(att);
                                }
                                Err(e) => {
                                    self.state.chat_messages.push(ChatMessage::Error {
                                        content: format!(
                                            "Failed to attach {}: {e}",
                                            path.display()
                                        ),
                                    });
                                }
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
            (KeyCode::Char('G'), _) if self.state.input.is_empty() => {
                let max_scroll = self
                    .state
                    .total_chat_lines
                    .get()
                    .saturating_sub(self.state.chat_area_height.get());
                self.state.scroll_offset = max_scroll;
                self.state.user_scrolled_up = false;
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
            (KeyCode::Char(c), m) if Self::should_insert_text(m) => {
                self.state.focused_tool = None;
                self.state.history.reset();
                self.state.input.insert_char(c);
                self.record_text_input_activity(c.len_utf8());
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

    pub(super) async fn handle_approval_key(&mut self, key: KeyCode) {
        self.state.diff_expanded = false;
        self.state.diff_scroll = 0;
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
}
