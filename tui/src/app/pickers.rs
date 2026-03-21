use super::*;

impl App {
    pub(super) async fn handle_picker_key(&mut self, key: KeyCode) {
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

        // Refresh session search results when filter changes
        self.refresh_session_search();
    }

    /// Re-query session search results using FTS5 when the filter text changes.
    pub(super) fn refresh_session_search(&mut self) {
        let Some(auto) = self.state.slash_auto.as_mut() else {
            return;
        };
        let crate::tui::slash_auto::DropdownMode::Sessions {
            ref mut results, ..
        } = auto.mode
        else {
            return;
        };

        let filter = auto.filter.trim().to_string();
        if filter.is_empty() {
            // Restore recent sessions when filter is cleared
            if let Ok(recent) = self.state.sessions.list_with_content(50) {
                *results = recent;
            }
        } else {
            // Use FTS5 search
            if let Ok(fts_results) = self.state.sessions.search(&filter, 50) {
                *results = fts_results;
            }
        }
    }

    /// Handle confirm/cancel for session delete.
    pub(super) async fn handle_session_picker_confirm(&mut self, key: KeyCode) {
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
    pub(super) async fn handle_picker_select(&mut self) {
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
            DropdownMode::Models {
                models,
                recent,
                collapsed,
                ..
            } => {
                let selection = crate::tui::slash_auto::resolve_model_selection(
                    models,
                    recent,
                    &auto.filter,
                    auto.selected,
                    collapsed,
                );
                // Look up capabilities before clearing slash_auto (borrows models/recent)
                let (supports_tools, supports_vision, supports_thinking) = selection
                    .as_ref()
                    .and_then(|(_, model_id)| {
                        models
                            .iter()
                            .chain(recent.iter())
                            .find(|(_, m)| m.id == *model_id)
                            .map(|(_, m)| {
                                (m.supports_tools, m.supports_vision, m.supports_thinking)
                            })
                    })
                    .unwrap_or((true, false, false));
                // Build display name for roundhouse before clearing slash_auto
                let display_for_roundhouse = selection.as_ref().map(|(provider, model_id)| {
                    let display = caboose_core::provider::catalog::by_id(provider)
                        .map(|e| e.display_name.to_string())
                        .unwrap_or_else(|| provider.clone());
                    (provider.clone(), display, model_id.clone())
                });
                // Handle group header selection — toggle collapse, keep picker open
                if let Some((ref provider, ref group_id)) = selection
                    && provider == "_group"
                {
                    let group_id = group_id.clone();
                    if let Some(crate::tui::slash_auto::SlashAutoState {
                        mode:
                            crate::tui::slash_auto::DropdownMode::Models {
                                ref mut collapsed, ..
                            },
                        ..
                    }) = self.state.slash_auto
                    {
                        if collapsed.contains(&group_id) {
                            collapsed.remove(&group_id);
                        } else {
                            collapsed.insert(group_id);
                        }
                    }
                    return;
                }

                self.state.slash_auto = None;
                self.state.input.clear();
                if self.state.handoff_agent_pending {
                    self.state.handoff_agent_pending = false;
                    if let Some((provider, model_id)) = selection {
                        self.spawn_handoff_agent(&provider, &model_id).await;
                    }
                } else if self.state.roundhouse_model_add {
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
                    if provider == "_local" {
                        // Open the local connect dialog for the chosen server type
                        let server_type = match model_id.as_str() {
                            "ollama" => caboose_core::provider::local::LocalServerType::Ollama,
                            "lmstudio" => caboose_core::provider::local::LocalServerType::LmStudio,
                            "llamacpp" => caboose_core::provider::local::LocalServerType::LlamaCpp,
                            _ => caboose_core::provider::local::LocalServerType::Custom,
                        };
                        let provider_name = match model_id.as_str() {
                            "ollama" => "Ollama",
                            "lmstudio" => "LM Studio",
                            "llamacpp" => "llama.cpp",
                            _ => "Custom",
                        };
                        self.state.model_picker_connect = true;
                        self.state.dialog_stack.push(
                            crate::tui::dialog::DialogKind::LocalProviderConnect(
                                crate::tui::dialog::LocalProviderConnectState {
                                    provider_id: model_id.clone(),
                                    provider_name: provider_name.to_string(),
                                    address: server_type.default_address().to_string(),
                                    models: vec![],
                                    selected_model: 0,
                                    phase: crate::tui::dialog::LocalConnectPhase::Address,
                                    error: None,
                                    probe_rx: None,
                                },
                            ),
                        );
                    } else {
                        self.state.model_supports_tools = supports_tools;
                        self.state.model_supports_vision = supports_vision;
                        self.state.model_supports_thinking = supports_thinking;
                        // Reset thinking mode when switching to a model that doesn't support it
                        if !supports_thinking {
                            self.state.thinking_mode = caboose_core::provider::ThinkingMode::Off;
                        }
                        self.select_model(&provider, &model_id);
                    }
                }
            }
            DropdownMode::Providers { .. } => {
                use crate::tui::slash_auto::build_provider_entries;
                let collapsed_snapshot =
                    if let DropdownMode::Providers { ref collapsed } = auto.mode {
                        collapsed.clone()
                    } else {
                        std::collections::HashSet::new()
                    };
                let entries = build_provider_entries(
                    &auto.filter,
                    &collapsed_snapshot,
                    &self.state.discovered_locals,
                );
                if let Some(entry) = entries.get(auto.selected) {
                    match entry {
                        crate::tui::slash_auto::ProviderPickerEntry::GroupHeader(group_id) => {
                            // Toggle collapse
                            let group_id = group_id.clone();
                            if let Some(ref mut auto) = self.state.slash_auto
                                && let DropdownMode::Providers {
                                    ref mut collapsed, ..
                                } = auto.mode
                            {
                                if collapsed.contains(&group_id) {
                                    collapsed.remove(&group_id);
                                } else {
                                    collapsed.insert(group_id);
                                }
                            }
                        }
                        crate::tui::slash_auto::ProviderPickerEntry::Provider(provider_id) => {
                            let provider_id = provider_id.clone();
                            self.state.slash_auto = None;
                            self.state.input.clear();

                            // Local providers use address+probe flow instead of API key
                            if caboose_core::provider::catalog::by_id(&provider_id)
                                .map(|p| p.is_local())
                                .unwrap_or(false)
                            {
                                let entry =
                                    caboose_core::provider::catalog::by_id(&provider_id).unwrap();
                                let server_type = match provider_id.as_str() {
                                    "ollama" => {
                                        caboose_core::provider::local::LocalServerType::Ollama
                                    }
                                    "lmstudio" => {
                                        caboose_core::provider::local::LocalServerType::LmStudio
                                    }
                                    "llamacpp" => {
                                        caboose_core::provider::local::LocalServerType::LlamaCpp
                                    }
                                    _ => caboose_core::provider::local::LocalServerType::Custom,
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
                                let has_existing =
                                    self.state.config.keys.get(&provider_id).is_some();
                                self.state.dialog_stack.push(DialogKind::ApiKeyInput(
                                    KeyInputState::new(provider_id, has_existing),
                                ));
                            }
                        }
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
                                let mut config = preset_info.config;
                                config.removed = true;
                                caboose_core::config::save_mcp_server_toggle(&name, &config);
                            }
                        } else {
                            caboose_core::config::remove_mcp_server(&name);
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
                                "suggest.enabled" => {
                                    let suggest_config = self
                                        .state
                                        .config
                                        .suggest
                                        .get_or_insert_with(Default::default);
                                    suggest_config.enabled = enabled;
                                    caboose_core::config::save_suggest_enabled(enabled);
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
                                    let mut prefs = crate::prefs::TuiPrefs::load();
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
                                    caboose_core::config::save_behavior_max_session_cost(new_max);
                                }
                                "reasoning.level" => {
                                    let new_mode = caboose_core::provider::ThinkingMode::ALL
                                        .iter()
                                        .find(|m| m.label() == item.value)
                                        .copied()
                                        .unwrap_or_default();
                                    self.state.thinking_mode = new_mode;
                                    if let Some(ref provider) = self.provider {
                                        provider.set_thinking_mode(new_mode);
                                    }
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
                    // Collect preview before rewinding
                    let preview_entries = self.state.checkpoints.preview(checkpoint_id).ok();
                    self.state.slash_auto = None;
                    self.state.input.clear();
                    match self.state.checkpoints.rewind(checkpoint_id) {
                        Ok(summary) => {
                            // Recompute modified_files from baselines (files are now restored on disk)
                            self.recompute_modified_files();
                            let mut msg = format!("Rewound to before \"{preview}\". {summary}");
                            if let Some(entries) = preview_entries
                                && !entries.is_empty()
                            {
                                msg.push('\n');
                                for entry in &entries {
                                    match &entry.action {
                                        crate::checkpoint::PreviewAction::Restore {
                                            lines_added,
                                            lines_removed,
                                        } => {
                                            msg.push_str(&format!(
                                                "\n  Restore: {} (+{} -{})",
                                                entry.path.display(),
                                                lines_added,
                                                lines_removed,
                                            ));
                                        }
                                        crate::checkpoint::PreviewAction::Delete => {
                                            msg.push_str(&format!(
                                                "\n  Delete: {}",
                                                entry.path.display(),
                                            ));
                                        }
                                        crate::checkpoint::PreviewAction::NoChange => {}
                                    }
                                }
                            }
                            self.state
                                .chat_messages
                                .push(ChatMessage::System { content: msg });
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

    pub(super) fn picker_item_count(&self) -> usize {
        use crate::tui::slash_auto::DropdownMode;

        let Some(auto) = &self.state.slash_auto else {
            return 0;
        };
        match &auto.mode {
            DropdownMode::Sessions { results, .. } => {
                crate::tui::session_picker::filter_search_results(results, &auto.filter).len()
            }
            DropdownMode::Models {
                models,
                recent,
                collapsed,
                ..
            } => crate::tui::slash_auto::filtered_model_count(
                models,
                recent,
                &auto.filter,
                collapsed,
            ),
            DropdownMode::Providers { collapsed } => {
                crate::tui::slash_auto::build_provider_entries(
                    &auto.filter,
                    collapsed,
                    &self.state.discovered_locals,
                )
                .len()
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
}
