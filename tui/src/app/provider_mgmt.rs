use super::*;

impl App {
    /// Try to get the active provider, or attempt to resolve one.
    /// Returns None and pushes an error chat message if no provider is available.
    pub(super) fn require_provider(&mut self) -> bool {
        if self.provider.is_some() {
            return true;
        }
        // Try to resolve again (user may have set env var)
        match self.state.providers.get_provider(None, None) {
            Ok(p) => {
                self.state.active_provider_name = p.name().to_string();
                self.state.active_model_name = p.model().to_string();
                self.provider = Some(p);
                self.resolve_compaction_provider();
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

    /// Resolve the `compaction_model` config to a dedicated compaction provider.
    /// Uses the active provider name with a model override. Falls back silently on error.
    pub(super) fn resolve_compaction_provider(&mut self) {
        let model = self
            .state
            .config
            .behavior
            .as_ref()
            .and_then(|b| b.compaction_model.clone());
        if let Some(model) = model {
            match self
                .state
                .providers
                .get_provider(Some(&self.state.active_provider_name), Some(&model))
            {
                Ok(p) => {
                    tracing::info!(compaction_model = %model, "Resolved compaction provider");
                    self.state.agent.compaction_provider = Some(p);
                }
                Err(e) => {
                    tracing::warn!(
                        compaction_model = %model,
                        "Failed to resolve compaction_model, using active provider: {e}"
                    );
                    self.state.agent.compaction_provider = None;
                }
            }
        } else {
            self.state.agent.compaction_provider = None;
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

    pub(super) fn images_config(&self) -> caboose_core::config::schema::ImagesConfig {
        self.state.config.images.clone().unwrap_or_default()
    }

    /// Connect to a provider (resolve + activate + save as last-used).
    pub(super) async fn connect_provider(&mut self, provider_id: &str) {
        self.state.providers = ProviderRegistry::new(&self.state.config);
        match self.state.providers.get_provider(Some(provider_id), None) {
            Ok(p) => {
                self.state.active_provider_name = p.name().to_string();
                self.state.active_model_name = p.model().to_string();

                // If the static table doesn't know this model, fetch from provider API
                if caboose_core::provider::models_dev::context_window(&self.state.active_model_name)
                    .is_none()
                    && let Ok(model_list) = p.list_models().await
                {
                    let cw_entries: Vec<(String, Option<u32>)> = model_list
                        .iter()
                        .map(|m| (m.id.clone(), m.context_window))
                        .collect();
                    caboose_core::provider::models_dev::cache_from_model_list(&cw_entries);
                }

                // Update context window for compaction and sidebar display
                self.state.agent.context_window =
                    caboose_core::provider::models_dev::context_window_or_default(
                        &self.state.active_model_name,
                    );

                let cw_display = caboose_core::provider::models_dev::context_window(
                    &self.state.active_model_name,
                )
                .map(|cw| format!(" ({}k context)", cw / 1000))
                .unwrap_or_default();
                self.state.chat_messages.push(ChatMessage::System {
                    content: format!(
                        "Connected to {}/{}{}",
                        self.state.active_provider_name, self.state.active_model_name, cw_display,
                    ),
                });
                self.provider = Some(p);
                self.resolve_compaction_provider();

                // Persist last-used provider so we reconnect on restart
                let mut prefs = crate::prefs::TuiPrefs::load();
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

    /// Handle `/mcp` slash command with subcommands: list, restart.
    pub(super) async fn handle_mcp_command(&mut self, slash: &str) {
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
    pub(super) async fn open_model_dropdown(&mut self) {
        let mut models = Vec::new();
        let mut error = None;
        let active = self.state.active_provider_name.clone();

        // Fetch models from the active provider
        if let Some(ref provider) = self.provider {
            if active == "openrouter" {
                if let Some(api_key) = self.state.config.keys.get("openrouter") {
                    let or_provider = caboose_core::provider::openrouter::OpenRouterProvider::new(
                        api_key.to_string(),
                        provider.model().to_string(),
                    );
                    match or_provider.list_models_with_pricing().await {
                        Ok((model_list, pricing_entries)) => {
                            for (model_id, model_pricing) in pricing_entries {
                                self.state
                                    .pricing
                                    .insert_with_cross_map(model_id, model_pricing);
                            }
                            for m in model_list {
                                models.push((active.clone(), m));
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
                            models.push((active.clone(), m));
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

        // Also fetch OpenRouter models if key exists and it's not the active provider
        if active != "openrouter"
            && let Some(api_key) = self.state.config.keys.get("openrouter")
        {
            let or_provider = caboose_core::provider::openrouter::OpenRouterProvider::new(
                api_key.to_string(),
                "anthropic/claude-sonnet-4.6".to_string(),
            );
            if let Ok((model_list, pricing_entries)) = or_provider.list_models_with_pricing().await
            {
                for (model_id, model_pricing) in pricing_entries {
                    self.state
                        .pricing
                        .insert_with_cross_map(model_id, model_pricing);
                }
                for m in model_list {
                    if !models
                        .iter()
                        .any(|(p, mo)| p == "openrouter" && mo.id == m.id)
                    {
                        models.push(("openrouter".to_string(), m));
                    }
                }
            }
        }
        // Add models from local providers
        for (name, local_cfg) in &self.state.config.local_providers {
            if let Some(ref model) = local_cfg.model {
                models.push((
                    name.clone(),
                    caboose_core::provider::ModelInfo {
                        id: model.clone(),
                        name: model.clone(),
                        context_window: None,
                        supports_tools: true,
                        supports_vision: false,
                        supports_thinking: false,
                    },
                ));
            }
        }
        // Add models from auto-discovered local servers (Ollama, LM Studio, etc.)
        for server in &self.state.discovered_locals {
            if !server.available {
                continue;
            }
            let provider_id = match server.server_type {
                caboose_core::provider::local::LocalServerType::Ollama => "ollama",
                caboose_core::provider::local::LocalServerType::LmStudio => "lmstudio",
                caboose_core::provider::local::LocalServerType::LlamaCpp => "llamacpp",
                caboose_core::provider::local::LocalServerType::Custom => "custom",
            };
            for model_id in &server.models {
                // Skip if already present from configured local providers
                if models
                    .iter()
                    .any(|(p, m)| p == provider_id && &m.id == model_id)
                {
                    continue;
                }
                models.push((
                    provider_id.to_string(),
                    caboose_core::provider::ModelInfo {
                        id: model_id.clone(),
                        name: model_id.clone(),
                        context_window: None,
                        supports_tools: true,
                        supports_vision: false,
                        supports_thinking: false,
                    },
                ));
            }
        }
        // Cache context windows from provider API for models not in the static table
        let cw_entries: Vec<(String, Option<u32>)> = models
            .iter()
            .map(|(_, m)| (m.id.clone(), m.context_window))
            .collect();
        caboose_core::provider::models_dev::cache_from_model_list(&cw_entries);

        models.sort_by(|(pa, a), (pb, b)| pa.cmp(pb).then(a.id.cmp(&b.id)));

        // Prepend local server connect entries at the very top (shown before all other models).
        // Provider "_local" sorts before any alphabetical id, and they're pinned here explicitly.
        let local_connect_entries: &[(&str, &str)] = &[
            ("ollama", "Ollama"),
            ("lmstudio", "LM Studio"),
            ("llamacpp", "llama.cpp"),
            ("custom", "Custom server"),
        ];
        for (id, name) in local_connect_entries.iter().rev() {
            models.insert(
                0,
                (
                    "_local".to_string(),
                    caboose_core::provider::ModelInfo {
                        id: id.to_string(),
                        name: name.to_string(),
                        context_window: None,
                        supports_tools: true,
                        supports_vision: false,
                        supports_thinking: false,
                    },
                ),
            );
        }

        // Build recent models from prefs
        let prefs = crate::prefs::TuiPrefs::load();
        let recent: Vec<(String, caboose_core::provider::ModelInfo)> = prefs
            .recent_models
            .iter()
            .map(|rm| {
                // Look up capabilities from the fetched model list
                let found = models.iter().find(|(_, m)| m.id == rm.model_id);
                let supports_tools = found.map(|(_, m)| m.supports_tools).unwrap_or(true);
                let supports_vision = found.map(|(_, m)| m.supports_vision).unwrap_or(false);
                let supports_thinking = found.map(|(_, m)| m.supports_thinking).unwrap_or(false);
                (
                    rm.provider.clone(),
                    caboose_core::provider::ModelInfo {
                        id: rm.model_id.clone(),
                        name: rm.model_id.clone(),
                        context_window: None,
                        supports_tools,
                        supports_vision,
                        supports_thinking,
                    },
                )
            })
            .collect();

        // By default, collapse all providers except the active one
        let mut collapsed: std::collections::HashSet<String> = models
            .iter()
            .map(|(p, _)| p.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .filter(|p| p != &active && p != "_local")
            .collect();
        // Also collapse "_local" if active provider is not a local one
        let local_providers = ["ollama", "lmstudio", "llamacpp", "custom"];
        if !local_providers.contains(&active.as_str()) {
            collapsed.insert("_local".to_string());
        }

        self.state.input.clear();
        self.state.slash_auto = Some(crate::tui::slash_auto::SlashAutoState::with_models(
            models, error, recent, collapsed,
        ));
    }

    /// Open the MCP server picker (inline dropdown mode).
    pub(super) fn open_mcp_picker(&mut self) {
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
    pub(super) async fn handle_mcp_tab(&mut self) {
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
                caboose_core::config::save_mcp_server_toggle(
                    &name,
                    &self.state.mcp_manager.servers[&name].config,
                );
            } else {
                // Enable preset — mark enabled, save, background connect
                if let Some(server) = self.state.mcp_manager.servers.get_mut(&name) {
                    server.config.disabled = false;
                }
                caboose_core::config::save_mcp_server_toggle(
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
    pub(super) fn select_model(&mut self, provider_name: &str, model_id: &str) {
        let old_provider_name = self.state.active_provider_name.clone();
        let old_model_name = self.state.active_model_name.clone();
        match self
            .state
            .providers
            .get_provider(Some(provider_name), Some(model_id))
        {
            Ok(new_provider) => {
                self.state.active_provider_name = new_provider.name().to_string();
                self.state.active_model_name = new_provider.model().to_string();
                // Sync thinking mode to the new provider
                new_provider.set_thinking_mode(self.state.thinking_mode);
                self.provider = Some(new_provider);
                self.resolve_compaction_provider();

                // Update context window for compaction and sidebar display
                self.state.agent.context_window =
                    caboose_core::provider::models_dev::context_window_or_default(
                        &self.state.active_model_name,
                    );

                let cw_display = caboose_core::provider::models_dev::context_window(
                    &self.state.active_model_name,
                )
                .map(|cw| format!(" ({}k context)", cw / 1000))
                .unwrap_or_default();
                let switch_handoff = self.build_model_switch_handoff_context(
                    &old_provider_name,
                    &old_model_name,
                    &self.state.active_provider_name,
                    &self.state.active_model_name,
                );
                if let Some(handoff_text) = switch_handoff {
                    self.persist_message("model_switch_context", &handoff_text);
                    self.state.agent.conversation.messages.push(
                        crate::agent::conversation::Message {
                            role: crate::agent::conversation::Role::User,
                            content: crate::agent::conversation::Content::Text(handoff_text),
                            tool_call_id: None,
                        },
                    );
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!(
                            "Switched to {}/{}{}. Handoff summary injected for continuity.",
                            self.state.active_provider_name,
                            self.state.active_model_name,
                            cw_display
                        ),
                    });
                } else {
                    self.state.chat_messages.push(ChatMessage::System {
                        content: format!(
                            "Switched to {}/{}{}",
                            self.state.active_provider_name,
                            self.state.active_model_name,
                            cw_display,
                        ),
                    });
                }

                // Persist last-used provider + model + recent history
                let mut prefs = crate::prefs::TuiPrefs::load();
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

    /// Build tool definitions to send to the LLM, respecting model capability.
    pub(super) fn build_tool_defs(&self) -> Vec<caboose_core::provider::ToolDefinition> {
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
}
