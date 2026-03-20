use super::*;

impl App {
    pub(super) async fn handle_roundhouse_picker_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) {
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
                self.state.roundhouse_update_rx = None;
                self.state.roundhouse_synthesis_rx = None;
                self.state.roundhouse_critique_rx = None;
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

    pub(super) fn start_roundhouse_planning(&mut self) {
        // Reset scroll so output auto-follows streaming text from the start
        self.state.scroll_offset = 0;
        self.state.user_scrolled_up = false;

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
            let sys = crate::roundhouse::planner::planning_system_prompt(&prompt);
            let p = prompt.clone();
            let t = tools.clone();
            tokio::spawn(async move {
                let result = crate::roundhouse::planner::run_planner(
                    primary_provider,
                    sys,
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
                let sys = crate::roundhouse::planner::planning_system_prompt(&prompt);
                let p = prompt.clone();
                let t = tools.clone();
                let idx = i + 1;
                tokio::spawn(async move {
                    let result = crate::roundhouse::planner::run_planner(
                        provider,
                        sys,
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

    /// Spawn parallel critique tasks for Roundhouse mode.
    /// Each model reviews all plans except its own.
    pub(super) fn start_roundhouse_critique(&mut self) {
        self.state.scroll_offset = 0;
        self.state.user_scrolled_up = false;
        // Extract everything we need from session before releasing the borrow
        let (
            prompt,
            timeout,
            all_plans,
            primary_provider_name,
            primary_model_name,
            secondaries,
            annotations,
        ) = {
            let session = match self.state.roundhouse_session.as_ref() {
                Some(s) => s,
                None => return,
            };
            let prompt = match session.prompt.clone() {
                Some(p) => p,
                None => return,
            };
            let timeout = session.config.critique_timeout_secs;
            let all_plans: Vec<(String, String)> = session
                .successful_plans()
                .iter()
                .map(|(p, t)| (p.to_string(), t.to_string()))
                .collect();
            let primary_provider_name = session.primary_provider.clone();
            let primary_model_name = session.primary_model.clone();
            let secondaries: Vec<(usize, String, String)> = session
                .secondaries
                .iter()
                .enumerate()
                .map(|(i, s)| (i, s.provider_name.clone(), s.model_name.clone()))
                .collect();
            let annotations = session.annotations.clone();
            (
                prompt,
                timeout,
                all_plans,
                primary_provider_name,
                primary_model_name,
                secondaries,
                annotations,
            )
        };

        // No tools for critique phase
        let tools: Vec<caboose_core::provider::ToolDefinition> = Vec::new();

        let (update_tx, update_rx) = tokio::sync::mpsc::unbounded_channel();
        self.state.roundhouse_critique_rx = Some(update_rx);

        // Build plan refs for critique_system_prompt
        let plan_refs: Vec<(&str, &str)> = all_plans
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();

        // Spawn primary critique (index 0)
        if let Ok(primary_provider) = self
            .state
            .providers
            .get_provider(Some(&primary_provider_name), Some(&primary_model_name))
        {
            let tx = update_tx.clone();
            let sys = crate::roundhouse::planner::critique_system_prompt(
                &prompt,
                &primary_provider_name,
                &plan_refs,
                &annotations,
            );
            let t = tools.clone();
            tokio::spawn(async move {
                let result = crate::roundhouse::planner::run_planner(
                    primary_provider,
                    sys,
                    "Review the plans above and provide your critique.".to_string(),
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
        } else {
            // Mark primary critique as failed
            if let Some(ref mut session) = self.state.roundhouse_session {
                session.primary_critique_status = crate::roundhouse::PlannerStatus::Failed(
                    "Could not create provider for critique".to_string(),
                );
            }
        }

        for (i, provider_name, model_name) in secondaries {
            if let Ok(provider) = self
                .state
                .providers
                .get_provider(Some(&provider_name), Some(&model_name))
            {
                let tx = update_tx.clone();
                let sys = crate::roundhouse::planner::critique_system_prompt(
                    &prompt,
                    &provider_name,
                    &plan_refs,
                    &annotations,
                );
                let t = tools.clone();
                let idx = i + 1;
                tokio::spawn(async move {
                    let result = crate::roundhouse::planner::run_planner(
                        provider,
                        sys,
                        "Review the plans above and provide your critique.".to_string(),
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
                    s.critique_status = crate::roundhouse::PlannerStatus::Failed(format!(
                        "Could not create provider '{provider_name}'"
                    ));
                }
            }
        }
    }

    /// Send all collected plans to the primary provider for synthesis.
    pub(super) fn start_roundhouse_synthesis(&mut self) {
        self.state.scroll_offset = 0;
        self.state.user_scrolled_up = false;

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
        let critiques = session.successful_critiques();
        let critiques_opt = if critiques.is_empty() {
            None
        } else {
            Some(critiques)
        };
        let annotations = session.annotations.clone();
        let system = crate::roundhouse::planner::synthesis_system_prompt(
            &prompt,
            &plans,
            critiques_opt.as_deref(),
            &annotations,
        );

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
            caboose_core::provider::Message {
                role: "system".to_string(),
                content: serde_json::json!(system),
            },
            caboose_core::provider::Message {
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
                    Ok(caboose_core::provider::StreamEvent::TextDelta(delta)) => {
                        let _ = tx.send(delta);
                    }
                    Ok(caboose_core::provider::StreamEvent::Error(_))
                    | Ok(caboose_core::provider::StreamEvent::ProviderError { .. })
                    | Ok(caboose_core::provider::StreamEvent::Done { .. }) => {
                        break;
                    }
                    _ => {}
                }
            }
            // tx drops here, signalling completion
        });

        self.state.roundhouse_synthesis_rx = Some(rx);
    }

    pub(super) fn clear_roundhouse_session(&mut self) {
        self.state.roundhouse_session = None;
        self.state.roundhouse_update_rx = None;
        self.state.roundhouse_synthesis_rx = None;
        self.state.roundhouse_critique_rx = None;
        self.state.roundhouse_model_add = false;
    }

    /// Handle `/roundhouse cancel` and `/roundhouse clear` subcommands.
    pub(super) fn handle_roundhouse_subcommand(&mut self, sub: &str) {
        match sub {
            "cancel" => {
                if self.state.roundhouse_session.is_some() {
                    self.state.roundhouse_session = None;
                    self.state.roundhouse_update_rx = None;
                    self.state.roundhouse_synthesis_rx = None;
                    self.state.roundhouse_critique_rx = None;
                    self.state.roundhouse_model_add = false;
                    self.state.dialog_stack.base = Screen::Chat;
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
                    self.state.roundhouse_critique_rx = None;
                    self.state.roundhouse_model_add = false;
                    self.state.dialog_stack.base = Screen::Chat;
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
                        "Unknown roundhouse subcommand: `{other}`. Use `cancel` or `clear`."
                    ),
                });
            }
        }
    }

    /// Extract code block bodies from markdown content.
    fn extract_code_blocks(content: &str) -> Vec<String> {
        let mut blocks = Vec::new();
        let mut in_block = false;
        let mut current: Vec<&str> = Vec::new();

        for line in content.lines() {
            if line.trim_start().starts_with("```") {
                if in_block {
                    blocks.push(current.join("\n"));
                    current.clear();
                    in_block = false;
                } else {
                    in_block = true;
                }
            } else if in_block {
                current.push(line);
            }
        }
        if in_block && !current.is_empty() {
            blocks.push(current.join("\n"));
        }
        blocks
    }

    pub(super) fn copy_hovered_code_block(&mut self) {
        let Some((mi, bi)) = self.state.hovered_code_block else {
            return;
        };
        let text = match self.state.chat_messages.get(mi) {
            Some(ChatMessage::Assistant { content, .. }) => {
                let blocks = Self::extract_code_blocks(content);
                match blocks.get(bi) {
                    Some(b) => b.clone(),
                    None => return,
                }
            }
            _ => return,
        };
        match crate::clipboard::copy_to_clipboard(&text) {
            Ok(()) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Copied code block to clipboard.".to_string(),
                });
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Copy failed: {e}"),
                });
            }
        }
    }

    pub(super) fn copy_hovered_message(&mut self) {
        let Some(i) = self.state.hovered_message else {
            return;
        };
        let text = match self.state.chat_messages.get(i) {
            Some(ChatMessage::Assistant { content, .. }) => content.clone(),
            _ => return,
        };
        match crate::clipboard::copy_to_clipboard(&text) {
            Ok(()) => {
                self.state.chat_messages.push(ChatMessage::System {
                    content: "Copied to clipboard.".to_string(),
                });
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Copy failed: {e}"),
                });
            }
        }
    }
}
