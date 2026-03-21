use super::*;

impl App {
    pub(super) fn handle_file_browser_key(&mut self, key: KeyCode) {
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
                        match crate::attachment::read_image_attachment(&path, &self.images_config())
                        {
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

    pub(super) fn handle_agents_list_key(&mut self, key: KeyCode) {
        let count = self.state.agent_definitions.len();
        let agents_state = match self.state.dialog_stack.top_mut() {
            Some(DialogKind::AgentsList(s)) => s,
            _ => return,
        };
        if count == 0 {
            self.state.dialog_stack.pop();
            return;
        }
        match key {
            KeyCode::Up | KeyCode::Char('k') => {
                agents_state.selected = if agents_state.selected == 0 {
                    count.saturating_sub(1)
                } else {
                    agents_state.selected - 1
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                agents_state.selected = (agents_state.selected + 1) % count;
            }
            KeyCode::Enter => {
                let agent_name = self.state.agent_definitions[agents_state.selected]
                    .name
                    .clone();
                self.state.dialog_stack.pop();
                self.state.input.clear();
                self.state.input.push_str(&format!("/{agent_name} "));
            }
            KeyCode::Esc => {
                self.state.dialog_stack.pop();
            }
            _ => {}
        }
    }

    pub(super) fn handle_circuits_list_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
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

    pub(super) fn handle_migration_checklist_key(&mut self, key: KeyCode) {
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

    pub(super) async fn handle_key_input_key(&mut self, key: KeyCode) {
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

    pub(super) async fn handle_local_connect_key(&mut self, key: KeyCode) {
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
                        "ollama" => caboose_core::provider::local::LocalServerType::Ollama,
                        "lmstudio" => caboose_core::provider::local::LocalServerType::LmStudio,
                        "llamacpp" => caboose_core::provider::local::LocalServerType::LlamaCpp,
                        _ => caboose_core::provider::local::LocalServerType::Custom,
                    };
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let addr = address;
                    tokio::spawn(async move {
                        match caboose_core::provider::local::probe_server(&addr, &server_type).await
                        {
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
                    let local_config = caboose_core::config::schema::LocalProviderConfig {
                        provider_type: provider_id.clone(),
                        address: address.clone(),
                        model: Some(model_name.clone()),
                        display_name: Some(provider_name.clone()),
                    };
                    caboose_core::config::save_local_provider(&provider_id, &local_config);

                    // Update in-memory config so connect_provider can find it
                    self.state
                        .config
                        .local_providers
                        .insert(provider_id.clone(), local_config);

                    // Always update discovered_locals so this server's models are available
                    // in the picker for the rest of the session.
                    {
                        use caboose_core::provider::local::{LocalServer, LocalServerType};
                        let server_type = match provider_id.as_str() {
                            "ollama" => LocalServerType::Ollama,
                            "lmstudio" => LocalServerType::LmStudio,
                            "llamacpp" => LocalServerType::LlamaCpp,
                            _ => LocalServerType::Custom,
                        };
                        let probed_models = match self.state.dialog_stack.top() {
                            Some(crate::tui::dialog::DialogKind::LocalProviderConnect(s)) => {
                                s.models.clone()
                            }
                            _ => vec![model_name.clone()],
                        };
                        let new_server = LocalServer {
                            server_type,
                            address: address.clone(),
                            available: true,
                            models: probed_models,
                        };
                        if let Some(existing) = self
                            .state
                            .discovered_locals
                            .iter_mut()
                            .find(|s| s.address == address)
                        {
                            *existing = new_server;
                        } else {
                            self.state.discovered_locals.push(new_server);
                        }
                    }

                    let from_picker = self.state.model_picker_connect;
                    self.state.model_picker_connect = false;

                    if from_picker && self.state.roundhouse_model_add {
                        // Add directly as a roundhouse secondary
                        self.state.roundhouse_model_add = false;
                        self.state.dialog_stack.pop(); // close LocalProviderConnect
                        if let Some(crate::tui::dialog::DialogKind::RoundhouseProviderPicker(
                            picker,
                        )) = self.state.dialog_stack.top_mut()
                        {
                            picker
                                .secondaries
                                .push(crate::tui::dialog::RoundhouseSecondary {
                                    provider_id: provider_id.clone(),
                                    display_name: provider_name.clone(),
                                    model: model_name.clone(),
                                });
                        }
                    } else {
                        // Connect as active provider and close dialogs
                        self.connect_provider(&provider_id).await;
                        self.state.dialog_stack.clear();
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    pub(super) fn handle_mcp_input_key(&mut self, key: KeyCode) {
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

    pub(super) fn handle_agent_stream_overlay_key(
        &mut self,
        key: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) {
        use crate::tui::dialog::{AgentStreamOverlayState, DialogKind};
        use crossterm::event::{KeyCode, KeyModifiers};

        match key {
            KeyCode::Esc => {
                self.state.agent_stream_overlay = None;
                self.state.dialog_stack.pop();
            }
            KeyCode::Tab => {
                let agent_count = self.state.sub_agents.len();
                if agent_count > 1 {
                    if modifiers.contains(KeyModifiers::SHIFT) {
                        // Shift+Tab: cycle to previous agent
                        let idx = self.state.agent_stream_overlay.unwrap_or(0);
                        let prev = if idx == 0 { agent_count - 1 } else { idx - 1 };
                        self.state.agent_stream_overlay = Some(prev);
                    } else {
                        // Tab: cycle to next agent
                        let idx = self.state.agent_stream_overlay.unwrap_or(0);
                        let next = (idx + 1) % agent_count;
                        self.state.agent_stream_overlay = Some(next);
                    }
                    // Reset scroll state
                    if let Some(DialogKind::AgentStreamOverlay(state)) =
                        self.state.dialog_stack.top_mut()
                    {
                        *state = AgentStreamOverlayState::new();
                    }
                }
            }
            KeyCode::Up => {
                if let Some(DialogKind::AgentStreamOverlay(state)) =
                    self.state.dialog_stack.top_mut()
                {
                    state.follow = false;
                    state.scroll_offset = state.scroll_offset.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                if let Some(idx) = self.state.agent_stream_overlay
                    && let Some(agent) = self.state.sub_agents.get(idx)
                {
                    let stream_len = agent.stream.len();
                    if let Some(DialogKind::AgentStreamOverlay(state)) =
                        self.state.dialog_stack.top_mut()
                    {
                        let new_offset = state.scroll_offset + 1;
                        // If we've scrolled to the bottom, re-enable follow
                        if new_offset >= stream_len {
                            state.scroll_offset = stream_len.saturating_sub(1);
                            state.follow = true;
                        } else {
                            state.scroll_offset = new_offset;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_workspace_list_key(&mut self, key: crossterm::event::KeyCode) {
        use crate::tui::dialog::{DialogKind, WorkspaceAddState};
        use crossterm::event::KeyCode;

        match key {
            KeyCode::Esc => {
                self.state.dialog_stack.pop();
            }
            KeyCode::Up => {
                if let Some(DialogKind::WorkspaceList(state)) = self.state.dialog_stack.top_mut()
                    && state.selected > 0
                {
                    state.selected -= 1;
                }
            }
            KeyCode::Down => {
                if let Some(DialogKind::WorkspaceList(state)) = self.state.dialog_stack.top_mut() {
                    let max = state.workspaces.len().saturating_sub(1);
                    if state.selected < max {
                        state.selected += 1;
                    }
                }
            }
            KeyCode::Char('a') => {
                self.state
                    .dialog_stack
                    .push(DialogKind::WorkspaceAdd(WorkspaceAddState::default()));
            }
            KeyCode::Char('e') | KeyCode::Enter => {
                // Edit the selected workspace (mode + permissions only)
                let edit_state =
                    if let Some(DialogKind::WorkspaceList(state)) = self.state.dialog_stack.top() {
                        state.workspaces.get(state.selected).map(|(name, cfg, _)| {
                            use caboose_core::config::schema::{WorkspaceAccess, WorkspaceMode};
                            let mode_selected = if cfg.mode == WorkspaceMode::Proactive {
                                0
                            } else {
                                1
                            };
                            let permissions_selected = if cfg.access == WorkspaceAccess::ReadWrite {
                                0
                            } else {
                                1
                            };
                            WorkspaceAddState::for_edit(
                                name.clone(),
                                cfg.path.clone(),
                                mode_selected,
                                permissions_selected,
                            )
                        })
                    } else {
                        None
                    };
                if let Some(s) = edit_state
                    && !s.path_input.is_empty()
                {
                    self.state.dialog_stack.push(DialogKind::WorkspaceAdd(s));
                }
            }
            KeyCode::Char('d') => {
                let name_to_remove = if let Some(DialogKind::WorkspaceList(state)) =
                    self.state.dialog_stack.top_mut()
                {
                    state
                        .workspaces
                        .get(state.selected)
                        .map(|(n, _, _)| n.clone())
                } else {
                    None
                };

                if let Some(name) = name_to_remove {
                    caboose_core::config::remove_workspace(&name);
                    // Update in-memory config
                    self.state.config.workspaces.remove(&name);
                    if let Some(DialogKind::WorkspaceList(state)) =
                        self.state.dialog_stack.top_mut()
                    {
                        state.workspaces.retain(|(n, _, _)| n != &name);
                        state.clamp_selected();
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) async fn handle_workspace_add_key(&mut self, key: crossterm::event::KeyCode) {
        use crate::tui::dialog::{DialogKind, WorkspaceAddPhase};
        use crossterm::event::KeyCode;

        match key {
            KeyCode::Esc => {
                // Clone phase out of the shared borrow before taking any mutable borrow.
                let phase = if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top()
                {
                    s.phase.clone()
                } else {
                    return;
                };
                match phase {
                    WorkspaceAddPhase::Path => {
                        self.state.dialog_stack.pop();
                    }
                    _ => {
                        if let Some(DialogKind::WorkspaceAdd(state)) =
                            self.state.dialog_stack.top_mut()
                        {
                            let prev = match state.phase {
                                WorkspaceAddPhase::Name => WorkspaceAddPhase::Path,
                                WorkspaceAddPhase::Mode => {
                                    if state.editing_name.is_some() {
                                        // In edit mode, Esc on Mode cancels entirely
                                        self.state.dialog_stack.pop();
                                        return;
                                    }
                                    WorkspaceAddPhase::Name
                                }
                                WorkspaceAddPhase::Permissions => WorkspaceAddPhase::Mode,
                                WorkspaceAddPhase::Path => WorkspaceAddPhase::Path,
                            };
                            state.phase = prev;
                            state.error = None;
                        }
                    }
                }
            }
            KeyCode::Up => {
                if let Some(DialogKind::WorkspaceAdd(state)) = self.state.dialog_stack.top_mut() {
                    match state.phase {
                        WorkspaceAddPhase::Path => {
                            if state.path_selected > 0 {
                                state.path_selected -= 1;
                            }
                        }
                        WorkspaceAddPhase::Mode => {
                            if state.mode_selected > 0 {
                                state.mode_selected -= 1;
                            }
                        }
                        WorkspaceAddPhase::Permissions => {
                            if state.permissions_selected > 0 {
                                state.permissions_selected -= 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Down => {
                if let Some(DialogKind::WorkspaceAdd(state)) = self.state.dialog_stack.top_mut() {
                    match state.phase {
                        WorkspaceAddPhase::Path => {
                            let max = state.path_matches.len().saturating_sub(1);
                            if state.path_selected < max {
                                state.path_selected += 1;
                            }
                        }
                        WorkspaceAddPhase::Mode => {
                            if state.mode_selected < 1 {
                                state.mode_selected += 1;
                            }
                        }
                        WorkspaceAddPhase::Permissions => {
                            if state.permissions_selected < 1 {
                                state.permissions_selected += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Enter => {
                self.handle_workspace_add_confirm().await;
            }
            KeyCode::Backspace => {
                if let Some(DialogKind::WorkspaceAdd(state)) = self.state.dialog_stack.top_mut() {
                    match state.phase {
                        WorkspaceAddPhase::Path => {
                            state.path_input.pop();
                            state.error = None;
                            state.path_selected = 0;
                        }
                        WorkspaceAddPhase::Name => {
                            state.name_input.pop();
                            state.error = None;
                        }
                        WorkspaceAddPhase::Mode | WorkspaceAddPhase::Permissions => {}
                    }
                }
            }
            KeyCode::Char(c) => {
                // Track new path_input for scan trigger after the mutable borrow ends
                let new_path: Option<String> = if let Some(DialogKind::WorkspaceAdd(state)) =
                    self.state.dialog_stack.top_mut()
                {
                    match state.phase {
                        WorkspaceAddPhase::Path => {
                            state.path_input.push(c);
                            state.error = None;
                            state.path_selected = 0;
                            Some(state.path_input.clone())
                        }
                        WorkspaceAddPhase::Name => {
                            state.name_input.push(c);
                            state.error = None;
                            None
                        }
                        WorkspaceAddPhase::Mode | WorkspaceAddPhase::Permissions => None,
                    }
                } else {
                    None
                };
                let _ = new_path; // scan trigger in event loop tick detects input change
            }
            _ => {}
        }
    }

    pub(super) async fn handle_workspace_add_confirm(&mut self) {
        use crate::tui::dialog::{DialogKind, WorkspaceAddPhase};
        use caboose_core::config::schema::{WorkspaceAccess, WorkspaceConfig, WorkspaceMode};

        let phase = if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top() {
            s.phase.clone()
        } else {
            return;
        };

        match phase {
            WorkspaceAddPhase::Path => {
                // Determine confirmed path: use highlighted suggestion or raw input
                let confirmed_path =
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top() {
                        if !s.path_matches.is_empty() {
                            s.path_matches
                                .get(s.path_selected)
                                .cloned()
                                .unwrap_or_else(|| s.path_input.clone())
                        } else {
                            s.path_input.clone()
                        }
                    } else {
                        return;
                    };

                // Validate path
                let path = std::path::Path::new(&confirmed_path);
                if !path.exists() {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some("path does not exist".to_string());
                    }
                    return;
                }
                if !path.is_dir() {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some("path is not a directory".to_string());
                    }
                    return;
                }
                let canonical = match std::fs::canonicalize(path) {
                    Ok(p) => p,
                    Err(_) => {
                        if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut()
                        {
                            s.error = Some("cannot resolve path".to_string());
                        }
                        return;
                    }
                };
                if canonical == self.state.primary_root {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some("cannot add the primary repo as a workspace".to_string());
                    }
                    return;
                }
                if canonical.starts_with(&self.state.primary_root)
                    || self.state.primary_root.starts_with(&canonical)
                {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some(
                            "workspace cannot be nested inside the primary repo (or vice versa)"
                                .to_string(),
                        );
                    }
                    return;
                }
                // Check not already registered
                let already_registered = self
                    .state
                    .config
                    .workspaces
                    .values()
                    .any(|w| std::fs::canonicalize(&w.path).ok().as_ref() == Some(&canonical));
                if already_registered {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some("this path is already registered".to_string());
                    }
                    return;
                }

                // Pre-fill name from dirname and advance to Name phase
                let dirname = canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                    s.path_input = canonical.to_string_lossy().to_string();
                    s.name_input = dirname;
                    s.phase = WorkspaceAddPhase::Name;
                    s.error = None;
                }
            }

            WorkspaceAddPhase::Name => {
                let name = if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top()
                {
                    s.name_input.trim().to_string()
                } else {
                    return;
                };

                if name.is_empty() {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some("name cannot be empty".to_string());
                    }
                    return;
                }
                if name.contains(' ') {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some("name cannot contain spaces".to_string());
                    }
                    return;
                }
                // Check uniqueness
                let already_named = self.state.config.workspaces.contains_key(&name);
                if already_named {
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                        s.error = Some(format!("workspace '{name}' already exists"));
                    }
                    return;
                }

                if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                    s.phase = WorkspaceAddPhase::Mode;
                    s.error = None;
                }
            }

            WorkspaceAddPhase::Mode => {
                // Advance to Permissions phase
                if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top_mut() {
                    s.phase = WorkspaceAddPhase::Permissions;
                    s.error = None;
                }
            }

            WorkspaceAddPhase::Permissions => {
                let (path, name, mode, access, editing_name) =
                    if let Some(DialogKind::WorkspaceAdd(s)) = self.state.dialog_stack.top() {
                        let mode = if s.mode_selected == 0 {
                            WorkspaceMode::Proactive
                        } else {
                            WorkspaceMode::Explicit
                        };
                        let access = if s.permissions_selected == 0 {
                            WorkspaceAccess::ReadWrite
                        } else {
                            WorkspaceAccess::ReadOnly
                        };
                        (
                            s.path_input.clone(),
                            s.name_input.trim().to_string(),
                            mode,
                            access,
                            s.editing_name.clone(),
                        )
                    } else {
                        return;
                    };

                let cfg = WorkspaceConfig { path, mode, access };
                caboose_core::config::save_workspace(&name, &cfg);
                self.state.config.workspaces.insert(name.clone(), cfg);
                self.state.dialog_stack.pop();
                self.refresh_workspace_list_state();

                let verb = if editing_name.is_some() {
                    "updated"
                } else {
                    "added"
                };
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!("workspace '{name}' {verb}"),
                });
            }
        }
    }

    /// Rebuild WorkspaceListState from current config if WorkspaceList is in the stack.
    pub(super) fn refresh_workspace_list_state(&mut self) {
        use crate::tui::dialog::DialogKind;

        // Build new state before borrowing dialog_stack mutably — avoids two simultaneous
        // borrows of `self.state` (one for iter_mut, one for &self.state.config).
        let new_state = build_workspace_list_state(&self.state.config);

        for dialog in self.state.dialog_stack.iter_mut() {
            if let DialogKind::WorkspaceList(state) = dialog {
                *state = new_state;
                return;
            }
        }
    }

    pub(super) fn handle_mcp_input_submit(&mut self) {
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
        let server_config = caboose_core::config::schema::McpServerConfig {
            command: Some(command),
            url: None,
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

    pub(super) async fn handle_command_palette_key(&mut self, key: KeyCode) {
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

    /// Open the settings picker with current config values.
    pub(super) fn open_settings_picker(&mut self) {
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
                let suggest_enabled = self.state.config.suggest.as_ref().is_none_or(|c| c.enabled);
                crate::tui::slash_auto::SettingsItem {
                    key: "suggest.enabled".to_string(),
                    label: "Suggest".to_string(),
                    value: if suggest_enabled {
                        "on".to_string()
                    } else {
                        "off".to_string()
                    },
                    kind: crate::tui::slash_auto::SettingsKind::Toggle,
                }
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

    /// Open the rewind picker with current checkpoints.
    pub(super) fn open_rewind_picker(&mut self) {
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
                let label = cp.name.as_deref().unwrap_or(&cp.prompt_preview).to_string();
                (cp.id, label, age, cp.files.len())
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
}
