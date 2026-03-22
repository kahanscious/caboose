use anyhow::Result;
use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::agent::conversation::ContentBlock;
use crate::agent::permission::PermissionMode;
use crate::agent::{AgentLoop, AgentState};
use crate::session::SessionManager;
use crate::tools::ToolRegistry;
use crate::tui::Terminal;
use crate::tui::dialog::{DialogKind, DialogStack, Screen};
use crate::tui::key_input::KeyInputState;
use caboose_core::config::Config;
use caboose_core::config::auth::AuthStore;
use caboose_core::provider::{Provider, ProviderRegistry};

mod input;
mod types;
use types::slice_chars;
pub use types::*;

mod helpers;
mod session_mgmt;
mod skills;
pub(crate) use helpers::*;

mod state;
pub use state::State;

mod dialogs;
mod handoff;
mod key_dispatch;
mod pickers;
mod provider_mgmt;
mod roundhouse;
mod slash_commands;
mod tool_handlers;

/// Top-level application state machine.
pub struct App {
    pub state: State,
    pub terminal: Terminal,
    pub(super) provider: Option<Box<dyn Provider>>,
}

impl App {
    /// Handle a quit request (ctrl+c). Requires two presses within 2 seconds.
    /// On second press, force-exits immediately to avoid cleanup lag.
    fn request_quit(&mut self) {
        const QUIT_TIMEOUT: Duration = Duration::from_secs(2);
        if let Some(first) = self.state.quit_first_press
            && first.elapsed() < QUIT_TIMEOUT
        {
            // Force-exit: restore terminal immediately and bail out.
            // Skips async cleanup (memory extraction, MCP disconnect) to
            // avoid the multi-second lag the user experiences.
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = crossterm::execute!(
                std::io::stdout(),
                crossterm::event::DisableMouseCapture,
                crossterm::terminal::LeaveAlternateScreen,
                crossterm::event::DisableBracketedPaste,
                crossterm::event::PopKeyboardEnhancementFlags,
                crossterm::cursor::Show
            );
            std::process::exit(0);
        }
        self.state.quit_first_press = Some(Instant::now());
    }

    /// Clear the main composer input and any transient completion state.
    fn clear_composer_input(&mut self) {
        self.state.input.clear();
        self.state.slash_auto = None;
        self.state.file_auto = None;
        self.state.text_selection = None;
        self.state.quit_first_press = None;
        self.reset_text_input_activity();
    }

    /// Extract plain text from the rendered chat lines within the given selection.
    fn extract_selected_text(&self, sel: &TextSelection) -> String {
        let (start_row, start_col, end_row, end_col) =
            if (sel.anchor_row, sel.anchor_col) <= (sel.end_row, sel.end_col) {
                (sel.anchor_row, sel.anchor_col, sel.end_row, sel.end_col)
            } else {
                (sel.end_row, sel.end_col, sel.anchor_row, sel.anchor_col)
            };

        let chat_area = match self.state.chat_area.get() {
            Some(a) => a,
            None => return String::new(),
        };
        let rendered = self.state.rendered_chat_text.borrow();
        let effective_offset = if self.state.user_scrolled_up {
            let max_scroll = self
                .state
                .total_chat_lines
                .get()
                .saturating_sub(self.state.chat_area_height.get());
            self.state.scroll_offset.min(max_scroll)
        } else {
            self.state
                .total_chat_lines
                .get()
                .saturating_sub(self.state.chat_area_height.get())
        };

        let mut result = Vec::new();
        for screen_row in start_row..=end_row {
            if screen_row < chat_area.y || screen_row >= chat_area.y + chat_area.height {
                continue;
            }
            let row_idx = effective_offset as usize + (screen_row - chat_area.y) as usize;
            let Some(row_text) = rendered.get(row_idx) else {
                continue;
            };

            let col_start = if screen_row == start_row {
                (start_col.saturating_sub(chat_area.x)) as usize
            } else {
                0
            };
            let col_end = if screen_row == end_row {
                (end_col.saturating_sub(chat_area.x)) as usize + 1
            } else {
                row_text.chars().count()
            };

            let slice = slice_chars(row_text, col_start, col_end);
            if !slice.is_empty() {
                result.push(slice);
            } else if start_row != end_row {
                result.push(String::new());
            }
        }

        result.join("\n")
    }

    /// Main event loop.
    pub async fn run(&mut self) -> Result<()> {
        self.terminal.enter()?;
        // Set terminal tab title
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle("caboose"));
        // Enable bracketed paste for API key input
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste)?;

        // Fire SessionStart lifecycle hooks
        if let Some(ref hooks_config) = self.state.config.hooks
            && !hooks_config.session_start.is_empty()
        {
            let hooks = hooks_config.session_start.clone();
            let context = serde_json::json!({
                "event": "SessionStart",
                "session_id": self.state.current_session_id,
            });
            tokio::spawn(async move {
                crate::hooks::fire_hooks(&hooks, context).await;
            });
        }

        // Background update check
        {
            let current_version = env!("CARGO_PKG_VERSION").to_string();
            let (tx, rx) = tokio::sync::oneshot::channel::<String>();
            tokio::spawn(async move {
                if let Ok(latest) = crate::update::fetch_latest_version().await {
                    let latest_bare = latest.strip_prefix('v').unwrap_or(&latest);
                    if crate::update::is_newer(latest_bare, &current_version) {
                        let _ = tx.send(latest_bare.to_string());
                    }
                }
            });
            self.state.update_check_rx = Some(rx);
        }

        // Background local LLM discovery
        {
            let (tx, rx) =
                tokio::sync::oneshot::channel::<Vec<caboose_core::provider::local::LocalServer>>();
            tokio::spawn(async move {
                let servers = caboose_core::provider::local::discover_local_servers().await;
                let _ = tx.send(servers);
            });
            self.state.local_discovery_rx = Some(rx);
        }

        loop {
            // Expire quit confirmation after 2 seconds
            if let Some(first) = self.state.quit_first_press
                && first.elapsed() >= Duration::from_secs(2)
            {
                self.state.quit_first_press = None;
            }

            // Advance animation tick
            self.state.tick = self.state.tick.wrapping_add(1);

            // Advance caboose position when agent or /init is active (every other tick for ~10 chars/sec)
            let agent_active = matches!(
                self.state.agent.state,
                crate::agent::AgentState::Streaming
                    | crate::agent::AgentState::ExecutingTools
                    | crate::agent::AgentState::PendingApproval { .. }
                    | crate::agent::AgentState::Compacting
            );
            let init_active = self.state.init_rx.is_some();
            if (agent_active || init_active) && self.state.tick.is_multiple_of(2) {
                self.state.caboose_pos = self.state.caboose_pos.wrapping_add(1);
            }

            // Check for update check result
            if let Some(ref mut rx) = self.state.update_check_rx {
                match rx.try_recv() {
                    Ok(version) => {
                        self.state.update_available = Some(version);
                        self.state.update_check_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.state.update_check_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Poll background local LLM discovery
            if let Some(ref mut rx) = self.state.local_discovery_rx {
                match rx.try_recv() {
                    Ok(servers) => {
                        self.state.discovered_locals = servers;
                        self.state.local_discovery_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.state.local_discovery_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Poll search setup background task
            if let Some(ref mut rx) = self.state.search_setup_rx {
                match rx.try_recv() {
                    Ok(msg) => {
                        if msg.starts_with("ERROR:") {
                            self.state.chat_messages.push(ChatMessage::Error {
                                content: msg.strip_prefix("ERROR: ").unwrap_or(&msg).to_string(),
                            });
                        } else {
                            self.state
                                .chat_messages
                                .push(ChatMessage::System { content: msg });
                        }
                        self.state.search_setup_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.state.search_setup_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Check for LLM-generated title
            if let Some(rx) = &mut self.state.title_rx {
                match rx.try_recv() {
                    Ok(title) => {
                        if !self.state.title_manually_set {
                            let truncated =
                                crate::tui::session_picker::truncate_at_word_boundary(&title, 60);
                            self.state.session_title = Some(truncated);
                            self.update_session_meta();
                        }
                        self.state.title_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        self.state.title_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Poll local provider probe result
            if let Some(DialogKind::LocalProviderConnect(lpc)) = self.state.dialog_stack.top_mut()
                && let Some(rx) = &mut lpc.probe_rx
            {
                match rx.try_recv() {
                    Ok(Ok(models)) => {
                        if models.is_empty() {
                            lpc.error = Some("Server responded but no models found".to_string());
                            lpc.phase = crate::tui::dialog::LocalConnectPhase::Address;
                        } else {
                            lpc.models = models;
                            lpc.selected_model = 0;
                            lpc.phase = crate::tui::dialog::LocalConnectPhase::ModelSelect;
                        }
                        lpc.probe_rx = None;
                    }
                    Ok(Err(msg)) => {
                        lpc.error = Some(msg);
                        lpc.phase = crate::tui::dialog::LocalConnectPhase::Address;
                        lpc.probe_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        lpc.error = Some("Probe failed unexpectedly".to_string());
                        lpc.phase = crate::tui::dialog::LocalConnectPhase::Address;
                        lpc.probe_rx = None;
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                }
            }

            // Poll workspace dir scan results
            if let Some(ref mut rx) = self.state.workspace_scan_rx {
                match rx.try_recv() {
                    Ok(matches) => {
                        if let Some(crate::tui::dialog::DialogKind::WorkspaceAdd(state)) =
                            self.state.dialog_stack.top_mut()
                        {
                            // Filter out the current primary repo from suggestions
                            let primary = self.state.primary_root.to_string_lossy().to_string();
                            let primary_canon = std::fs::canonicalize(&self.state.primary_root)
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or(primary.clone());
                            state.path_matches = matches
                                .into_iter()
                                .filter(|p| {
                                    let canon = std::fs::canonicalize(p)
                                        .map(|c| c.to_string_lossy().to_string())
                                        .unwrap_or_else(|_| p.clone());
                                    canon != primary_canon
                                })
                                .collect();
                        }
                        self.state.workspace_scan_rx = None;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        self.state.workspace_scan_rx = None;
                    }
                }
            }

            // Trigger workspace dir scan when query changes and has 2+ chars
            if let Some(crate::tui::dialog::DialogKind::WorkspaceAdd(add_state)) =
                self.state.dialog_stack.top()
            {
                let query = add_state.path_input.clone();
                if query.len() >= 2
                    && query != self.state.workspace_scan_last_query
                    && self.state.workspace_scan_rx.is_none()
                {
                    self.state.workspace_scan_last_query = query.clone();
                    // If the user typed a partial path, walk from its parent.
                    // Otherwise prioritize the project neighbourhood first, then
                    // all drive roots so nearby repos surface before timeout.
                    let roots = if query.contains('/') || query.contains('\\') {
                        let parent = std::path::Path::new(&query)
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|| query.clone());
                        vec![parent]
                    } else {
                        // Put the two ancestors of primary_root first so sibling
                        // repos are found immediately, then fall back to full scan.
                        let mut roots: Vec<String> = Vec::new();
                        let pr = &self.state.primary_root;
                        if let Some(p) = pr.parent() {
                            roots.push(p.to_string_lossy().to_string());
                            if let Some(gp) = p.parent() {
                                roots.push(gp.to_string_lossy().to_string());
                            }
                        }
                        for r in scan_roots() {
                            if !roots.contains(&r) {
                                roots.push(r);
                            }
                        }
                        roots
                    };
                    self.state.workspace_scan_rx = Some(spawn_dir_scan(roots, query));
                }
            }

            // Draw UI
            let state = &self.state;
            self.terminal.draw(|frame| {
                crate::tui::layout::render(frame, state);
            })?;

            // Keep scroll_offset tracking max_scroll whenever auto-following.
            // This ensures that if the user scrolls down later, their offset
            // is already at the right position (not stuck at some old value).
            if !self.state.user_scrolled_up {
                let max_scroll = self
                    .state
                    .total_chat_lines
                    .get()
                    .saturating_sub(self.state.chat_area_height.get());
                self.state.scroll_offset = max_scroll;
            }

            // Poll for keyboard/paste/mouse events — drain all pending to prevent
            // mouse tracking events from delaying key events
            if event::poll(Duration::from_millis(50))? {
                loop {
                    match event::read()? {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            self.handle_key(key.code, key.modifiers).await;
                        }
                        Event::Paste(text) => {
                            self.handle_paste(&text);
                        }
                        Event::Mouse(mouse) => {
                            let in_terminal = self
                                .state
                                .terminal_area
                                .get()
                                .map(|area| {
                                    mouse.row >= area.y
                                        && mouse.row < area.y + area.height
                                        && mouse.column >= area.x
                                        && mouse.column < area.x + area.width
                                })
                                .unwrap_or(false);

                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    self.state.text_selection = None;
                                    // Route to menus/dropdowns first
                                    if !self.handle_menu_scroll(true) {
                                        if in_terminal {
                                            if let Some(panel) = &mut self.state.terminal_panel {
                                                panel.scroll_up(3);
                                            }
                                        } else {
                                            let scroll_lines: u16 = 3;
                                            self.state.scroll_offset = self
                                                .state
                                                .scroll_offset
                                                .saturating_sub(scroll_lines);
                                            self.state.user_scrolled_up = true;
                                        }
                                    }
                                }
                                MouseEventKind::ScrollDown => {
                                    self.state.text_selection = None;
                                    // Route to menus/dropdowns first
                                    if !self.handle_menu_scroll(false) {
                                        if in_terminal {
                                            if let Some(panel) = &mut self.state.terminal_panel {
                                                panel.scroll_down(3);
                                            }
                                        } else {
                                            let scroll_lines: u16 = 3;
                                            self.state.scroll_offset = self
                                                .state
                                                .scroll_offset
                                                .saturating_add(scroll_lines);
                                            let max_scroll =
                                                self.state.total_chat_lines.get().saturating_sub(
                                                    self.state.chat_area_height.get(),
                                                );
                                            if self.state.scroll_offset >= max_scroll {
                                                self.state.scroll_offset = max_scroll;
                                                self.state.user_scrolled_up = false;
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::Down(_) => {
                                    self.state.text_selection = None;
                                    // Sidebar border drag to resize
                                    if self.state.sidebar_visible {
                                        let (tw, _) =
                                            crossterm::terminal::size().unwrap_or((80, 24));
                                        let border_col =
                                            tw.saturating_sub(self.state.sidebar_width);
                                        if mouse.column >= border_col.saturating_sub(1)
                                            && mouse.column <= border_col + 1
                                        {
                                            self.state.sidebar_drag = Some(mouse.column);
                                            continue;
                                        }
                                    }
                                    if in_terminal {
                                        // Check for [x] close button click (header row, last 5 cols)
                                        if let Some(area) = self.state.terminal_area.get()
                                            && mouse.row == area.y
                                            && mouse.column >= area.x + area.width.saturating_sub(5)
                                        {
                                            if let Some(panel) = &mut self.state.terminal_panel {
                                                panel.visible = false;
                                                self.state.terminal_focused = false;
                                            }
                                            continue;
                                        }
                                        self.state.terminal_focused = true;
                                    } else {
                                        self.state.terminal_focused = false;

                                        // Agents dismiss click
                                        if let Some(dismiss_y) = self.state.agents_dismiss_row.get()
                                            && mouse.row == dismiss_y
                                        {
                                            self.state
                                                .sub_agents
                                                .retain(|a| !a.state.is_terminal());
                                            // Clean up stashed changes for dismissed agents
                                            self.state.agent_changes.retain(|c| {
                                                self.state
                                                    .sub_agents
                                                    .iter()
                                                    .any(|a| a.id == c.agent_id)
                                            });
                                            self.state.conflict_report = None;
                                            if !self.state.sub_agents.is_empty() {
                                                let max =
                                                    self.state.sub_agents.len().saturating_sub(1);
                                                if self.state.sidebar_agent_selected > max {
                                                    self.state.sidebar_agent_selected = max;
                                                }
                                            } else {
                                                self.state.sidebar_focused = false;
                                            }
                                            continue;
                                        }

                                        // Files Modified header click to toggle collapse
                                        if let Some(header_y) =
                                            self.state.files_modified_header_row.get()
                                            && mouse.row == header_y
                                        {
                                            self.state.files_modified_collapsed =
                                                !self.state.files_modified_collapsed;
                                            continue;
                                        }

                                        // Pin bar toggle click
                                        if !self.state.pins.is_empty() {
                                            let pin_bar_end = if self.state.pins_expanded {
                                                1 + self.state.pins.len() as u16
                                            } else {
                                                1
                                            };
                                            if mouse.row >= 1 && mouse.row <= pin_bar_end {
                                                self.state.pins_expanded =
                                                    !self.state.pins_expanded;
                                                continue;
                                            }
                                        }

                                        // Diff toggle click zone logic — runs BEFORE truncation zones.
                                        // Extract the hit message index first (drops borrow before mutating chat_messages).
                                        let toggle_hit = {
                                            let rects = self.state.tool_toggle_rects.borrow();
                                            rects.iter().find(|&&(y, _)| y == mouse.row).copied()
                                        };
                                        if let Some((_, msg_idx)) = toggle_hit {
                                            // Determine if this is the active pending message
                                            let is_pending = matches!(
                                                self.state.chat_messages.get(msg_idx),
                                                Some(ChatMessage::Tool(t)) if t.status == ToolStatus::Pending
                                            );
                                            if is_pending {
                                                // Pending diff state lives on State, not ToolMessage
                                                self.state.diff_expanded =
                                                    !self.state.diff_expanded;
                                            } else if let Some(ChatMessage::Tool(tool_msg)) =
                                                self.state.chat_messages.get_mut(msg_idx)
                                            {
                                                tool_msg.diff_expanded = !tool_msg.diff_expanded;
                                            }
                                            continue;
                                        }

                                        // Thinking block click zone logic
                                        let thinking_zones =
                                            self.state.thinking_click_zones.borrow();
                                        let mut thinking_handled = false;
                                        for &(zone_y, msg_idx) in thinking_zones.iter() {
                                            if mouse.row == zone_y {
                                                if self.state.expanded_thinking.contains(&msg_idx) {
                                                    self.state.expanded_thinking.remove(&msg_idx);
                                                } else {
                                                    self.state.expanded_thinking.insert(msg_idx);
                                                }
                                                thinking_handled = true;
                                                break;
                                            }
                                        }
                                        drop(thinking_zones);
                                        if thinking_handled {
                                            continue;
                                        }

                                        // Code block copy badge click
                                        if let Some(badge) = self.state.code_block_badge_rect.get()
                                            && mouse.row == badge.y
                                            && mouse.column >= badge.x
                                            && mouse.column < badge.x + badge.width
                                        {
                                            self.copy_hovered_code_block();
                                            continue;
                                        }

                                        // Copy badge click
                                        let mut badge_handled = false;
                                        if let Some(badge) = self.state.copy_badge_rect.get()
                                            && mouse.row == badge.y
                                            && mouse.column >= badge.x
                                            && mouse.column < badge.x + badge.width
                                        {
                                            self.copy_hovered_message();
                                            badge_handled = true;
                                        }
                                        if badge_handled {
                                            continue;
                                        }

                                        // Scroll-to-bottom badge click
                                        if let Some(badge) =
                                            self.state.scroll_to_bottom_badge_rect.get()
                                            && mouse.row == badge.y
                                            && mouse.column >= badge.x
                                            && mouse.column < badge.x + badge.width
                                        {
                                            let max_scroll =
                                                self.state.total_chat_lines.get().saturating_sub(
                                                    self.state.chat_area_height.get(),
                                                );
                                            self.state.scroll_offset = max_scroll;
                                            self.state.user_scrolled_up = false;
                                            continue;
                                        }

                                        // Truncation click zone logic
                                        let zones = self.state.truncation_click_zones.borrow();
                                        let mut truncation_handled = false;
                                        for &(zone_y, msg_idx) in zones.iter() {
                                            if mouse.row == zone_y {
                                                if self.state.expanded_messages.contains(&msg_idx) {
                                                    self.state.expanded_messages.remove(&msg_idx);
                                                } else {
                                                    self.state.expanded_messages.insert(msg_idx);
                                                }
                                                truncation_handled = true;
                                                break;
                                            }
                                        }

                                        if !truncation_handled {
                                            self.state.text_selection = Some(TextSelection {
                                                anchor_row: mouse.row,
                                                anchor_col: mouse.column,
                                                end_row: mouse.row,
                                                end_col: mouse.column,
                                            });
                                        }
                                    }
                                }
                                MouseEventKind::Drag(MouseButton::Left) => {
                                    if self.state.sidebar_drag.is_some() {
                                        let (tw, _) =
                                            crossterm::terminal::size().unwrap_or((80, 24));
                                        let new_width = tw.saturating_sub(mouse.column);
                                        self.state.sidebar_width = new_width.clamp(
                                            crate::tui::layout::SIDEBAR_MIN_WIDTH,
                                            crate::tui::layout::SIDEBAR_MAX_WIDTH,
                                        );
                                        continue;
                                    }
                                    if let Some(ref mut sel) = self.state.text_selection {
                                        sel.end_row = mouse.row;
                                        sel.end_col = mouse.column;

                                        // Auto-scroll when dragging near viewport edges
                                        if let Some(chat_rect) = self.state.chat_area.get() {
                                            let scroll_margin: u16 = 2;
                                            let scroll_speed: u16 = 2;

                                            if mouse.row < chat_rect.y + scroll_margin {
                                                // Near top edge — scroll up
                                                self.state.scroll_offset = self
                                                    .state
                                                    .scroll_offset
                                                    .saturating_sub(scroll_speed);
                                                self.state.user_scrolled_up = true;
                                            } else if mouse.row
                                                >= chat_rect.y + chat_rect.height - scroll_margin
                                            {
                                                // Near bottom edge — scroll down
                                                self.state.scroll_offset = self
                                                    .state
                                                    .scroll_offset
                                                    .saturating_add(scroll_speed);
                                                let max_scroll = self
                                                    .state
                                                    .total_chat_lines
                                                    .get()
                                                    .saturating_sub(
                                                        self.state.chat_area_height.get(),
                                                    );
                                                if self.state.scroll_offset >= max_scroll {
                                                    self.state.scroll_offset = max_scroll;
                                                    self.state.user_scrolled_up = false;
                                                }
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::Up(_) => {
                                    self.state.sidebar_drag = None;
                                }
                                MouseEventKind::Moved => {
                                    // Mouse hover selects items in command palette
                                    let palette_hit =
                                        if let Some(DialogKind::CommandPalette(palette)) =
                                            self.state.dialog_stack.top()
                                        {
                                            let (tw, th) =
                                                crossterm::terminal::size().unwrap_or((80, 24));
                                            crate::tui::command_palette::hit_test(
                                                palette,
                                                &self.state,
                                                mouse.row,
                                                th,
                                                tw,
                                            )
                                        } else {
                                            None
                                        };
                                    if let Some(idx) = palette_hit
                                        && let Some(DialogKind::CommandPalette(palette)) =
                                            self.state.dialog_stack.top_mut()
                                    {
                                        palette.selected = idx;
                                    }

                                    // Hover detection for copy badge
                                    let in_chat = self
                                        .state
                                        .chat_area
                                        .get()
                                        .map(|r| {
                                            mouse.row >= r.y
                                                && mouse.row < r.y + r.height
                                                && mouse.column >= r.x
                                                && mouse.column < r.x + r.width
                                        })
                                        .unwrap_or(false);
                                    if in_chat && !roundhouse_active(&self.state) {
                                        // Code block hover takes priority
                                        let cb_zones = self.state.code_block_hover_zones.borrow();
                                        let cb_hit = cb_zones
                                            .iter()
                                            .find(|&&(sy, ey, _, _)| {
                                                mouse.row >= sy && mouse.row < ey
                                            })
                                            .map(|&(_, _, mi, bi)| (mi, bi));
                                        drop(cb_zones);

                                        if let Some(hit) = cb_hit {
                                            self.state.hovered_code_block = Some(hit);
                                            self.state.hovered_message = None;
                                        } else {
                                            self.state.hovered_code_block = None;
                                            let zones = self.state.copy_hover_zones.borrow();
                                            self.state.hovered_message = zones
                                                .iter()
                                                .find(|&&(sy, ey, _)| {
                                                    mouse.row >= sy && mouse.row < ey
                                                })
                                                .map(|&(_, _, idx)| idx);
                                        }
                                    } else {
                                        self.state.hovered_message = None;
                                        self.state.hovered_code_block = None;
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                    // Drain remaining pending events without waiting
                    if !event::poll(Duration::from_millis(0))? {
                        break;
                    }
                }
            }

            // Drain agent events
            let events = self.state.agent.poll();
            for event in &events {
                match event {
                    crate::agent::AgentEvent::ThinkingDelta(_) => {
                        // Thinking accumulates in agent.streaming_thinking (in poll()).
                        // Ensure the streaming thinking block is expanded by default.
                        self.state.expanded_thinking.insert(usize::MAX);
                    }
                    crate::agent::AgentEvent::TextDelta(_) => {
                        // Text accumulates in agent.streaming_text,
                        // which layout.rs reads during render

                        // Auto-collapse thinking when text response begins
                        if !self.state.agent.streaming_thinking.is_empty() {
                            self.state.expanded_thinking.remove(&usize::MAX);
                        }
                    }
                    crate::agent::AgentEvent::TurnComplete { .. } => {
                        // finalize_turn() already ran inside poll().
                        // Check if we need to execute tools or show approval.
                        self.handle_turn_complete().await;
                    }
                    crate::agent::AgentEvent::ProviderError {
                        category,
                        provider,
                        message,
                        hint,
                    } => {
                        let json = serde_json::json!({
                            "category": category,
                            "provider": provider,
                            "message": message,
                            "hint": hint,
                        });
                        self.persist_message("provider_error", &json.to_string());
                        self.state.chat_messages.push(ChatMessage::ProviderError {
                            category: category.clone(),
                            provider: provider.to_string(),
                            message: message.to_string(),
                            hint: hint.clone(),
                        });
                    }
                    crate::agent::AgentEvent::Error(e) => {
                        self.state
                            .chat_messages
                            .push(ChatMessage::Error { content: e.clone() });
                    }
                    crate::agent::AgentEvent::CompactionComplete => {
                        self.state.chat_messages.push(ChatMessage::System {
                            content: "Context compacted — conversation summarized.".to_string(),
                        });

                        // Re-inject active task outline so the agent retains awareness
                        if let Some(outline) = self.state.chat_messages.iter().rev().find_map(|m| {
                            if let ChatMessage::TaskOutline(o) = m {
                                Some(o.clone())
                            } else {
                                None
                            }
                        }) {
                            let active: Vec<_> = outline
                                .tasks
                                .iter()
                                .filter(|t| {
                                    matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress)
                                })
                                .collect();
                            if !active.is_empty() {
                                let mut task_text = String::from(
                                    "[Active task list (preserved across compaction)]\n",
                                );
                                for t in &active {
                                    let marker = match t.status {
                                        TaskStatus::InProgress => "[in_progress]",
                                        _ => "[pending]",
                                    };
                                    task_text.push_str(&format!("- {marker} {}\n", t.content));
                                }
                                self.state.agent.conversation.push(
                                    crate::agent::conversation::Message {
                                        role: crate::agent::conversation::Role::User,
                                        content: crate::agent::conversation::Content::Text(
                                            task_text,
                                        ),
                                        tool_call_id: None,
                                    },
                                );
                            }
                        }

                        // If compaction was auto-triggered, resume the stream
                        if !self.state.agent.stashed_tool_defs.is_empty()
                            && let Some(ref provider) = self.provider
                        {
                            let tool_defs: Vec<_> =
                                std::mem::take(&mut self.state.agent.stashed_tool_defs);
                            self.state.agent.start_stream(provider.as_ref(), &tool_defs);
                        }
                    }
                    _ => {}
                }
            }

            // Non-blocking tool execution: poll spawned tool results and
            // kick off the next tool when the previous one finishes.
            self.poll_tool_execution().await;
            self.poll_spawn_agent_handles().await;
            self.poll_mcp_connections();

            // Poll terminal panel output
            if let Some(panel) = &mut self.state.terminal_panel
                && panel.visible
            {
                panel.poll_output();

                // Resize PTY only when dimensions actually change
                if let Some(area) = self.state.terminal_area.get() {
                    let body_h = area.height.saturating_sub(1);
                    if body_h > 0 {
                        let new_size = (area.width, body_h);
                        if self.state.terminal_last_size != Some(new_size) {
                            let _ = panel.resize(area.width, body_h);
                            self.state.terminal_last_size = Some(new_size);
                        }
                    }
                }

                // Respawn if shell exited
                if !panel.is_alive() {
                    let was_focused = panel.focused;
                    let (cols, rows) = self
                        .state
                        .terminal_area
                        .get()
                        .map(|a| (a.width, a.height.saturating_sub(1).max(1)))
                        .unwrap_or((80, 24));
                    let cwd =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let cwd_str = cwd.to_string_lossy();
                    if let Ok(mut new_panel) =
                        crate::terminal::panel::TerminalPanel::new(cols, rows, &cwd_str)
                    {
                        new_panel.visible = true;
                        new_panel.focused = was_focused;
                        self.state.terminal_panel = Some(new_panel);
                    }
                }
            }

            // Drain /init generation events (non-blocking)
            if let Some(ref mut rx) = self.state.init_rx {
                let mut done = false;
                while let Ok(event) = rx.try_recv() {
                    match event {
                        crate::init::handler::InitEvent::TextDelta(text) => {
                            self.state.init_text.push_str(&text);
                        }
                        crate::init::handler::InitEvent::Done {
                            input_tokens,
                            output_tokens,
                        } => {
                            self.state.agent.last_input_tokens = input_tokens;
                            self.state.agent.last_output_tokens = output_tokens;
                            done = true;
                            break;
                        }
                        crate::init::handler::InitEvent::Error(e) => {
                            self.state
                                .chat_messages
                                .push(ChatMessage::Error { content: e });
                            done = true;
                            break;
                        }
                    }
                }
                if done {
                    self.state.init_rx = None;
                    self.finalize_init();
                }
            }

            // Poll roundhouse planner updates (non-blocking)
            if let Some(ref mut rx) = self.state.roundhouse_update_rx {
                let mut all_done = false;
                let mut cancelled = false;
                while let Ok(update) = rx.try_recv() {
                    match update {
                        crate::roundhouse::PlannerUpdate::StatusChanged {
                            planner_index,
                            status,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                let tick = self.state.tick;
                                if planner_index == 0 {
                                    // Never clear streaming text — accumulate across tool rounds
                                    session.primary_status = status;
                                    session.primary_status_tick = tick;
                                } else if let Some(s) =
                                    session.secondaries.get_mut(planner_index - 1)
                                {
                                    s.status = status;
                                    s.status_tick = tick;
                                }
                            }
                        }
                        crate::roundhouse::PlannerUpdate::StreamingDelta {
                            planner_index,
                            text,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                if planner_index == 0 {
                                    session.primary_streaming_text.push_str(&text);
                                } else if let Some(s) =
                                    session.secondaries.get_mut(planner_index - 1)
                                {
                                    s.streaming_text.push_str(&text);
                                }
                            }
                        }
                        crate::roundhouse::PlannerUpdate::ToolStarted {
                            planner_index,
                            tool_name,
                            args_summary,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                // Inject tool call marker into streaming text so it's visible
                                let marker = format!("\n\n⚙ {tool_name}({args_summary})…\n");
                                if planner_index == 0 {
                                    session.primary_streaming_text.push_str(&marker);
                                    session.primary_tool_calls.push(
                                        crate::roundhouse::RoundhouseToolCall {
                                            tool_name,
                                            args_summary,
                                            status: crate::roundhouse::ToolCallStatus::Running,
                                            result_summary: None,
                                        },
                                    );
                                } else if let Some(s) =
                                    session.secondaries.get_mut(planner_index - 1)
                                {
                                    s.streaming_text.push_str(&marker);
                                }
                            }
                        }
                        crate::roundhouse::PlannerUpdate::ToolCompleted {
                            planner_index,
                            tool_name: _,
                            summary,
                            is_error,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                let icon = if is_error { "✗" } else { "✓" };
                                let marker = format!("{icon} {summary}\n\n");
                                if planner_index == 0 {
                                    session.primary_streaming_text.push_str(&marker);
                                    if let Some(tc) =
                                        session.primary_tool_calls.iter_mut().rev().find(|tc| {
                                            tc.status == crate::roundhouse::ToolCallStatus::Running
                                        })
                                    {
                                        tc.status = if is_error {
                                            crate::roundhouse::ToolCallStatus::Failed
                                        } else {
                                            crate::roundhouse::ToolCallStatus::Success
                                        };
                                        tc.result_summary = Some(summary);
                                    }
                                } else if let Some(s) =
                                    session.secondaries.get_mut(planner_index - 1)
                                {
                                    s.streaming_text.push_str(&marker);
                                }
                            }
                        }
                        crate::roundhouse::PlannerUpdate::TokensUsed {
                            planner_index: _,
                            input_tokens: _,
                            output_tokens: _,
                        } => {
                            // Token tracking — rolled up for future cost display
                        }
                        crate::roundhouse::PlannerUpdate::PlanComplete {
                            planner_index,
                            result,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                match result {
                                    Ok(plan) => {
                                        if planner_index == 0 {
                                            session.primary_plan = Some(plan);
                                            session.primary_status =
                                                crate::roundhouse::PlannerStatus::Done;
                                        } else if let Some(s) =
                                            session.secondaries.get_mut(planner_index - 1)
                                        {
                                            s.plan = Some(plan);
                                            s.status = crate::roundhouse::PlannerStatus::Done;
                                        }
                                    }
                                    Err(e) => {
                                        let provider_name = if planner_index == 0 {
                                            session.primary_provider.clone()
                                        } else {
                                            session
                                                .secondaries
                                                .get(planner_index - 1)
                                                .map(|s| s.provider_name.clone())
                                                .unwrap_or_else(|| {
                                                    format!("planner-{planner_index}")
                                                })
                                        };

                                        if planner_index == 0 {
                                            session.primary_status =
                                                crate::roundhouse::PlannerStatus::Failed(e.clone());
                                        } else if let Some(s) =
                                            session.secondaries.get_mut(planner_index - 1)
                                        {
                                            s.status =
                                                crate::roundhouse::PlannerStatus::Failed(e.clone());
                                        }

                                        // Any planner failure cancels the entire roundhouse
                                        self.state.chat_messages.push(ChatMessage::Error {
                                            content: format!(
                                                "Roundhouse cancelled: {} failed — {e}",
                                                provider_name
                                            ),
                                        });
                                        self.state.roundhouse_session = None;
                                        self.state.roundhouse_model_add = false;
                                        cancelled = true;
                                        break;
                                    }
                                }

                                if session.all_planners_done() {
                                    let plan_count = session.successful_plans().len();
                                    session.phase =
                                        crate::roundhouse::RoundhousePhase::ReviewingPlans;
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: format!(
                                            "All planners complete ({plan_count} plans). Review plans to continue."
                                        ),
                                    });
                                    all_done = true;
                                }
                            }
                        }
                    }
                }
                if cancelled {
                    self.state.roundhouse_update_rx = None;
                    self.state.roundhouse_synthesis_rx = None;
                    self.state.roundhouse_critique_rx = None;
                } else if all_done {
                    self.state.roundhouse_update_rx = None;
                    // Plans are now in ReviewingPlans — user decides next step via gate actions
                }
            }

            // Poll roundhouse critique updates (non-blocking)
            if let Some(ref mut rx) = self.state.roundhouse_critique_rx {
                let mut all_critiques_done = false;
                while let Ok(update) = rx.try_recv() {
                    match update {
                        crate::roundhouse::PlannerUpdate::StatusChanged {
                            planner_index,
                            status,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                let tick = self.state.tick;
                                if planner_index == 0 {
                                    if matches!(status, crate::roundhouse::PlannerStatus::Streaming)
                                    {
                                        session.primary_critique_streaming_text.clear();
                                    }
                                    session.primary_critique_status = status;
                                    session.primary_critique_status_tick = tick;
                                } else if let Some(s) =
                                    session.secondaries.get_mut(planner_index - 1)
                                {
                                    if matches!(status, crate::roundhouse::PlannerStatus::Streaming)
                                    {
                                        s.critique_streaming_text.clear();
                                    }
                                    s.critique_status = status;
                                    s.critique_status_tick = tick;
                                }
                            }
                        }
                        crate::roundhouse::PlannerUpdate::StreamingDelta {
                            planner_index,
                            text,
                        } => {
                            if planner_index == 0
                                && let Some(ref mut session) = self.state.roundhouse_session
                            {
                                session.primary_critique_streaming_text.push_str(&text);
                            }
                        }
                        crate::roundhouse::PlannerUpdate::ToolStarted { .. }
                        | crate::roundhouse::PlannerUpdate::ToolCompleted { .. } => {
                            // Critiques don't use tools, ignore
                        }
                        crate::roundhouse::PlannerUpdate::TokensUsed { .. } => {
                            // No-op for now
                        }
                        crate::roundhouse::PlannerUpdate::PlanComplete {
                            planner_index,
                            result,
                        } => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                match result {
                                    Ok(critique_text) => {
                                        if planner_index == 0 {
                                            session.primary_critique = Some(critique_text);
                                            session.primary_critique_status =
                                                crate::roundhouse::PlannerStatus::Done;
                                        } else if let Some(s) =
                                            session.secondaries.get_mut(planner_index - 1)
                                        {
                                            s.critique = Some(critique_text);
                                            s.critique_status =
                                                crate::roundhouse::PlannerStatus::Done;
                                        }
                                    }
                                    Err(e) => {
                                        // Critique failures are NON-FATAL — just mark as failed
                                        if planner_index == 0 {
                                            session.primary_critique_status =
                                                crate::roundhouse::PlannerStatus::Failed(e);
                                        } else if let Some(s) =
                                            session.secondaries.get_mut(planner_index - 1)
                                        {
                                            s.critique_status =
                                                crate::roundhouse::PlannerStatus::Failed(e);
                                        }
                                    }
                                }

                                if session.all_critiques_done() {
                                    let critique_count = session.successful_critiques().len();
                                    session.phase =
                                        crate::roundhouse::RoundhousePhase::ReviewingCritiques;
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: format!(
                                            "All critiques complete ({critique_count} critiques). Review critiques to continue."
                                        ),
                                    });
                                    all_critiques_done = true;
                                }
                            }
                        }
                    }
                }
                if all_critiques_done {
                    self.state.roundhouse_critique_rx = None;
                    // Critiques are now in ReviewingCritiques — user decides next step via gate actions
                }
            }

            // Poll roundhouse synthesis streaming deltas (non-blocking)
            if let Some(ref mut rx) = self.state.roundhouse_synthesis_rx {
                let mut synthesis_done = false;
                loop {
                    match rx.try_recv() {
                        Ok(delta) => {
                            if let Some(ref mut session) = self.state.roundhouse_session {
                                session.synthesis_streaming_text.push_str(&delta);
                            }
                        }
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                            synthesis_done = true;
                            break;
                        }
                    }
                }
                if synthesis_done {
                    if let Some(session) = &mut self.state.roundhouse_session {
                        let plan_text = session.synthesis_streaming_text.clone();
                        let prompt = session.prompt.clone().unwrap_or_default();
                        let individual_plans: Vec<(String, String)> = session
                            .successful_plans()
                            .iter()
                            .map(|(p, t)| (p.to_string(), t.to_string()))
                            .collect();
                        let individual_refs: Vec<(&str, &str)> = individual_plans
                            .iter()
                            .map(|(p, t)| (p.as_str(), t.as_str()))
                            .collect();

                        let critique_plans: Vec<(String, String)> = session
                            .successful_critiques()
                            .iter()
                            .map(|(p, t)| (p.to_string(), t.to_string()))
                            .collect();
                        let critique_refs: Vec<(&str, &str)> = critique_plans
                            .iter()
                            .map(|(p, t)| (p.as_str(), t.as_str()))
                            .collect();
                        let critiques_opt = if critique_refs.is_empty() {
                            None
                        } else {
                            Some(critique_refs.as_slice())
                        };
                        let annotations = session.annotations.clone();

                        session.synthesized_plan = Some(plan_text.clone());
                        session.phase = crate::roundhouse::RoundhousePhase::Complete;

                        // Write plan file
                        let cwd = std::env::current_dir().unwrap_or_default();
                        let full_doc = crate::roundhouse::output::format_plans_document(
                            &prompt,
                            &individual_refs,
                            &plan_text,
                            critiques_opt,
                            &annotations,
                        );
                        match crate::roundhouse::output::write_plan_file(&cwd, &full_doc, &prompt) {
                            Ok(path) => {
                                session.plan_file = Some(path.clone());
                                self.state.chat_messages.push(ChatMessage::Assistant {
                                    content: format!(
                                        "## Roundhouse Plan\n\n{}\n\n---\n*Plan saved to `{}`*\n\nPaste the plan into chat to execute, or `/roundhouse clear` to dismiss.",
                                        plan_text,
                                        path.display()
                                    ),
                                    thinking: None,
                                });
                            }
                            Err(e) => {
                                self.state.chat_messages.push(ChatMessage::Assistant {
                                    content: format!(
                                        "## Roundhouse Plan\n\n{}\n\n---\n*Failed to save plan file: {}*\n\nPaste the plan into chat to execute, or `/roundhouse clear` to dismiss.",
                                        plan_text, e
                                    ),
                                    thinking: None,
                                });
                            }
                        }
                    }
                    self.state.roundhouse_synthesis_rx = None;
                    self.state.dialog_stack.base = Screen::Chat;
                }
            }

            // Poll subagent events (non-blocking)
            if let Some(ref mut rx) = self.state.sub_agent_rx {
                use crate::sub_agent::SubAgentEvent;
                type AgentUpdate = (
                    uuid::Uuid,
                    Option<crate::sub_agent::SubAgentState>,
                    Option<crate::sub_agent::SubAgentStreamLine>,
                    Option<f64>,
                );
                let mut agent_updates: Vec<AgentUpdate> = Vec::new();

                while let Ok(event) = rx.try_recv() {
                    match event {
                        SubAgentEvent::StateChange { id, state } => {
                            agent_updates.push((id, Some(state), None, None));
                        }
                        SubAgentEvent::StreamLine { id, line } => {
                            agent_updates.push((id, None, Some(line), None));
                        }
                        SubAgentEvent::CostUpdate { id, cost_usd } => {
                            agent_updates.push((id, None, None, Some(cost_usd)));
                        }
                        SubAgentEvent::ApprovalRequest {
                            id,
                            tool_name,
                            arguments,
                        } => {
                            if let Some(agent) =
                                self.state.sub_agents.iter_mut().find(|a| a.id == id)
                            {
                                if agent.auto_approve {
                                    // Auto-approve: send true immediately, stay Running
                                    if let Some(ref tx) = agent.approval_tx {
                                        let _ = tx.send(true);
                                    }
                                } else {
                                    agent.state =
                                        crate::sub_agent::SubAgentState::WaitingApproval {
                                            tool_name: tool_name.clone(),
                                        };
                                    self.state
                                        .sub_agent_pending_approvals
                                        .push_back((id, tool_name, arguments));
                                }
                            }
                        }
                    }
                }

                // Apply agent updates
                for (id, state_opt, line_opt, cost_opt) in agent_updates {
                    if let Some(agent) = self.state.sub_agents.iter_mut().find(|a| a.id == id) {
                        if let Some(state) = state_opt {
                            if matches!(state, crate::sub_agent::SubAgentState::Running) {
                                agent.started_at = Some(std::time::Instant::now());
                            }
                            agent.state = state;
                        }
                        if let Some(line) = line_opt {
                            agent.stream.push(line);
                        }
                        if let Some(cost) = cost_opt {
                            // Add the delta to session cost
                            let delta = cost - agent.cost_usd;
                            if delta > 0.0 {
                                self.state.session_cost += delta;
                            }
                            agent.cost_usd = cost;
                        }
                    }
                }
                // Drain approval queue: show next if nothing currently showing
                if self.state.sub_agent_approval_showing.is_none()
                    && !self.state.sub_agent_pending_approvals.is_empty()
                {
                    self.state.sub_agent_approval_showing =
                        self.state.sub_agent_pending_approvals.pop_front();
                }
            }

            // Poll core events (background agent lifecycle)
            if let Some(ref mut rx) = self.state.core_event_rx {
                let mut bg_changed = false;
                while let Ok(event) = rx.try_recv() {
                    use caboose_core::events::CoreEvent;
                    match event {
                        CoreEvent::BackgroundAgentStarted {
                            id, prompt_summary, ..
                        } => {
                            tracing::info!("Background agent started: {id} — {prompt_summary}");
                            bg_changed = true;
                        }
                        CoreEvent::BackgroundAgentComplete {
                            id: _, tokens_used, ..
                        } => {
                            self.state.chat_messages.push(ChatMessage::System {
                                content: format!(
                                    "Background agent completed ({tokens_used} tokens)."
                                ),
                            });
                            bg_changed = true;
                        }
                        CoreEvent::BackgroundAgentFailed { id: _, reason, .. } => {
                            if reason == "killed" {
                                // Intentional kill — don't show as error
                            } else {
                                self.state.chat_messages.push(ChatMessage::Error {
                                    content: format!("Background agent failed: {reason}"),
                                });
                            }
                            bg_changed = true;
                        }
                        _ => {}
                    }
                }
                if bg_changed && let Some(ref mgr) = self.state.background_manager {
                    self.state.background_agents_cache = mgr.list().await;
                }
            }

            // Poll circuit events (non-blocking)
            self.poll_circuit_events().await;

            if self.state.should_quit {
                break;
            }
        }

        // Fire SessionEnd hooks
        if let Some(ref hooks_config) = self.state.config.hooks
            && !hooks_config.session_end.is_empty()
        {
            let context = serde_json::json!({
                "event": "SessionEnd",
                "session_id": self.state.current_session_id.as_deref().unwrap_or(""),
                "message_count": self.state.agent.conversation.messages.len(),
            });
            // Fire-and-forget — SessionEnd hooks are non-blocking
            let hooks = hooks_config.session_end.clone();
            tokio::spawn(async move {
                let _ = crate::hooks::fire_hooks(&hooks, context).await;
            });
            // Give hooks a brief moment to start
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Extract memories before exit
        self.extract_session_memories().await;

        // Clean up terminal panel
        if let Some(mut panel) = self.state.terminal_panel.take() {
            panel.kill();
        }

        // Clean up MCP servers
        self.state.mcp_manager.disconnect_all().await;

        // Gracefully shut down LSP servers
        if let Some(lsp) = self.state.lsp_manager.take() {
            lsp.shutdown_all().await;
        }

        crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste)?;
        self.terminal.exit()?;
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // If terminal panel is focused, Esc closes/minimizes it
        if self.state.terminal_focused && key == KeyCode::Esc {
            if let Some(panel) = &mut self.state.terminal_panel {
                panel.visible = false;
            }
            self.state.terminal_focused = false;
            return;
        }

        // Escape cancels active agent operations before any UI dismissal.
        // Also fires when the main agent is idle but sub-agents/tasks/approvals
        // are still active — the user expects a clean slate after Escape.
        if key == KeyCode::Esc
            && (!matches!(self.state.agent.state, AgentState::Idle)
                || !self.state.sub_agents.is_empty()
                || self.state.sub_agent_approval_showing.is_some()
                || !self.state.sub_agent_pending_approvals.is_empty())
        {
            self.cancel_all_operations();
            return;
        }

        // Escape with empty input and non-empty queue → clear queue
        if key == KeyCode::Esc
            && self.state.input.is_empty()
            && !self.state.message_queue.is_empty()
        {
            self.state
                .chat_messages
                .retain(|m| !matches!(m, ChatMessage::Queued { .. }));
            self.state.message_queue.clear();
            return;
        }

        // If terminal panel is focused, forward keys to PTY
        if self.state.terminal_focused
            && let Some(panel) = &mut self.state.terminal_panel
            && let Some(bytes) = crate::terminal::input::key_to_bytes(key, modifiers)
        {
            let _ = panel.write_input(&bytes);
            return;
        }

        // Ctrl+H: handoff to another model via subagent
        if key == KeyCode::Char('h')
            && modifiers.contains(KeyModifiers::CONTROL)
            && !self.state.dialog_stack.has_overlay()
            && self.state.current_session_id.is_some()
        {
            self.state.handoff_agent_pending = true;
            self.open_model_dropdown().await;
            return;
        }

        // Check command registry for keybind match (only when no overlay captures input)
        if !self.state.dialog_stack.has_overlay()
            && let Some(cmd) = self.state.commands.find_keybind(key, modifiers)
            && (cmd.available)(&self.state)
        {
            // /model needs async model loading — handle specially
            if cmd.id == "model.open" {
                self.state.handoff_agent_pending = false;
                self.open_model_dropdown().await;
                return;
            }
            let action = (cmd.execute)(&mut self.state);
            self.process_action(action).await;
            return;
        }

        // Subagent approval bar — intercept y/n/a before main agent approval
        if self.state.sub_agent_approval_showing.is_some() {
            match key {
                KeyCode::Char('y') | KeyCode::Char('n') | KeyCode::Char('a') => {
                    if let Some((agent_id, _tool_name, _args)) =
                        self.state.sub_agent_approval_showing.take()
                    {
                        let approved = matches!(key, KeyCode::Char('y') | KeyCode::Char('a'));
                        if let Some(agent) =
                            self.state.sub_agents.iter_mut().find(|a| a.id == agent_id)
                        {
                            if matches!(key, KeyCode::Char('a')) {
                                agent.auto_approve = true;
                            }
                            if let Some(ref tx) = agent.approval_tx {
                                let _ = tx.send(approved);
                            }
                            if matches!(
                                agent.state,
                                crate::sub_agent::SubAgentState::WaitingApproval { .. }
                            ) {
                                agent.state = crate::sub_agent::SubAgentState::Running;
                            }
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        // Conflict report approval — intercept y/n when blocking overlaps are pending
        if self.state.conflict_report.is_some() {
            match key {
                KeyCode::Char('y') => {
                    let report = self.state.conflict_report.take().unwrap();
                    let pending_ids: std::collections::HashSet<uuid::Uuid> = report
                        .overlaps
                        .iter()
                        .filter(|o| {
                            matches!(
                                o.resolution,
                                crate::sub_agent::conflict::OverlapResolution::RequiresReview
                            )
                        })
                        .flat_map(|o| o.participants.iter().map(|p| p.agent_id))
                        .collect();
                    for id in pending_ids {
                        self.merge_single_agent(id).await;
                    }
                    return;
                }
                KeyCode::Char('n') => {
                    let report = self.state.conflict_report.take().unwrap();
                    let pending_ids: std::collections::HashSet<uuid::Uuid> = report
                        .overlaps
                        .iter()
                        .filter(|o| {
                            matches!(
                                o.resolution,
                                crate::sub_agent::conflict::OverlapResolution::RequiresReview
                            )
                        })
                        .flat_map(|o| o.participants.iter().map(|p| p.agent_id))
                        .collect();
                    for agent in &mut self.state.sub_agents {
                        if pending_ids.contains(&agent.id) {
                            agent.state = crate::sub_agent::SubAgentState::Conflict;
                        }
                    }
                    return;
                }
                _ => {}
            }
        }

        // Inline approval bar — intercept y/n/a before dialog dispatch
        if matches!(self.state.agent.state, AgentState::PendingApproval { .. }) {
            match key {
                KeyCode::Char('y') | KeyCode::Char('n') | KeyCode::Char('a') => {
                    self.handle_approval_key(key).await;
                    return;
                }
                KeyCode::Char('d') => {
                    self.state.diff_expanded = !self.state.diff_expanded;
                    if !self.state.diff_expanded {
                        self.state.diff_scroll = 0;
                    }
                    return;
                }
                KeyCode::Char('j') if self.state.diff_expanded => {
                    self.state.diff_scroll = self.state.diff_scroll.saturating_add(1);
                    return;
                }
                KeyCode::Char('k') if self.state.diff_expanded => {
                    self.state.diff_scroll = self.state.diff_scroll.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    if self.state.diff_expanded {
                        self.state.diff_scroll = self.state.diff_scroll.saturating_add(1);
                    }
                    return;
                }
                KeyCode::Up => {
                    if self.state.diff_expanded {
                        self.state.diff_scroll = self.state.diff_scroll.saturating_sub(1);
                    }
                    return;
                }
                _ => {}
            }
        }

        // Ctrl+C dismisses any overlay and starts quit timer
        if key == KeyCode::Char('c')
            && modifiers.contains(KeyModifiers::CONTROL)
            && self.state.dialog_stack.has_overlay()
        {
            self.state.dialog_stack.pop();
            self.request_quit();
            return;
        }

        match self.state.dialog_stack.top() {
            Some(DialogKind::ApiKeyInput(_)) => self.handle_key_input_key(key).await,
            Some(DialogKind::LocalProviderConnect(_)) => self.handle_local_connect_key(key).await,
            Some(DialogKind::FileBrowser(_)) => self.handle_file_browser_key(key),
            Some(DialogKind::McpServerInput(_)) => self.handle_mcp_input_key(key),
            Some(DialogKind::CommandPalette(_)) => self.handle_command_palette_key(key).await,
            Some(DialogKind::PasteConfirm { .. }) => match key {
                KeyCode::Char('y') | KeyCode::Enter => {
                    if let Some(DialogKind::PasteConfirm { text, .. }) =
                        self.state.dialog_stack.pop()
                    {
                        self.state.input.push_str(&text);
                        self.record_text_input_activity(text.len().max(16));
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.state.dialog_stack.pop();
                }
                _ => {}
            },
            Some(DialogKind::Confirm { .. }) => match key {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(DialogKind::Confirm { on_confirm, .. }) =
                        self.state.dialog_stack.pop()
                    {
                        match on_confirm {
                            crate::tui::dialog::ConfirmAction::NewSession => {
                                self.execute_new_session().await;
                            }
                        }
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.state.dialog_stack.pop();
                }
                _ => {}
            },
            Some(DialogKind::RoundhouseProviderPicker(_)) => {
                self.handle_roundhouse_picker_key(key, modifiers).await;
            }
            Some(DialogKind::CircuitsList(_)) => {
                self.handle_circuits_list_key(key, modifiers);
            }
            Some(DialogKind::MigrationChecklist(_)) => {
                self.handle_migration_checklist_key(key);
            }
            Some(DialogKind::WorkspaceList(_)) => self.handle_workspace_list_key(key),
            Some(DialogKind::WorkspaceAdd(_)) => self.handle_workspace_add_key(key).await,
            Some(DialogKind::AgentStreamOverlay(_)) => {
                self.handle_agent_stream_overlay_key(key, modifiers);
            }
            Some(DialogKind::AgentsList(_)) => {
                self.handle_agents_list_key(key);
            }
            Some(DialogKind::Help(_)) => match key {
                KeyCode::Esc => {
                    self.state.dialog_stack.pop();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some(DialogKind::Help(h)) = self.state.dialog_stack.top_mut() {
                        h.scroll_offset = h.scroll_offset.saturating_sub(1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some(DialogKind::Help(h)) = self.state.dialog_stack.top_mut() {
                        h.scroll_offset = h.scroll_offset.saturating_add(1);
                    }
                }
                _ => {}
            },
            Some(DialogKind::Status) => match key {
                KeyCode::Esc | KeyCode::Enter => {
                    self.state.dialog_stack.pop();
                }
                _ => {}
            },
            None => match self.state.dialog_stack.base {
                Screen::Home => self.handle_home_key(key, modifiers).await,
                Screen::Chat => self.handle_chat_key(key, modifiers).await,
                Screen::Roundhouse => {
                    self.handle_roundhouse_key(key, modifiers);
                }
            },
        }
    }

    async fn process_action(&mut self, action: crate::tui::command::Action) {
        use crate::tui::command::Action;
        match action {
            Action::None => {}
            Action::PushDialog(dialog) => self.state.dialog_stack.push(dialog),
            Action::EnterPickerMode(auto_state) => {
                self.state.input.clear();
                self.state.slash_auto = Some(auto_state);
            }
            Action::Quit => self.state.should_quit = true,
        }
    }

    /// Handle scroll wheel in menus/dropdowns. Returns `true` if a menu consumed the event.
    fn handle_menu_scroll(&mut self, up: bool) -> bool {
        // 1. Command palette
        if let Some(DialogKind::CommandPalette(palette)) = self.state.dialog_stack.top_mut() {
            if up {
                palette.selected = palette.selected.saturating_sub(1);
            } else {
                // Need count — drop mutable borrow, get count, re-borrow
                let selected = palette.selected;
                let count = match self.state.dialog_stack.top() {
                    Some(DialogKind::CommandPalette(p)) => {
                        crate::tui::command_palette::filtered_count(p, &self.state)
                    }
                    _ => 0,
                };
                if let Some(DialogKind::CommandPalette(p)) = self.state.dialog_stack.top_mut()
                    && selected + 1 < count
                {
                    p.selected += 1;
                }
            }
            return true;
        }

        // 2. Picker (sessions, models, MCP, providers)
        if self
            .state
            .slash_auto
            .as_ref()
            .map(|a| a.is_picker())
            .unwrap_or(false)
        {
            if up {
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.selected = auto.selected.saturating_sub(1);
                }
            } else {
                let max = self.picker_item_count().saturating_sub(1);
                if let Some(auto) = self.state.slash_auto.as_mut()
                    && auto.selected < max
                {
                    auto.selected += 1;
                }
            }
            return true;
        }

        // 3. File autocomplete
        if let Some(ref mut auto) = self.state.file_auto {
            if up {
                auto.select_up();
            } else {
                auto.select_down();
            }
            return true;
        }

        // 4. Slash autocomplete
        if self.state.slash_auto.is_some() {
            if up {
                if let Some(auto) = self.state.slash_auto.as_mut() {
                    auto.selected = auto.selected.saturating_sub(1);
                }
            } else {
                let input_text = self.state.input.content();
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
            }
            return true;
        }

        false
    }

    /// Called when a turn completes. Handles tool execution or transitions to idle.
    async fn handle_turn_complete(&mut self) {
        // Accumulate session tokens and cost from this turn
        self.state.session_input_tokens = self
            .state
            .session_input_tokens
            .saturating_add(self.state.agent.last_input_tokens as u64);
        self.state.session_output_tokens = self
            .state
            .session_output_tokens
            .saturating_add(self.state.agent.last_output_tokens as u64);
        if let Some(cost) = self.state.pricing.estimate_cost_with_cache(
            &self.state.active_model_name,
            self.state.agent.last_input_tokens,
            self.state.agent.last_output_tokens,
            self.state.agent.last_cache_read_tokens,
            self.state.agent.last_cache_creation_tokens,
        ) {
            self.state.session_cost += cost;
        }

        let t0 = Instant::now();
        // Flush any accumulated assistant text to chat display
        self.flush_assistant_text();
        let flush_ms = t0.elapsed().as_millis();
        if flush_ms > 5 {
            tracing::debug!("flush_assistant_text took {flush_ms}ms");
        }

        // Text-based task fallback for models without tool support
        if !self.state.model_supports_tools
            && let Some(text) = self.state.chat_messages.iter().rev().find_map(|m| {
                if let ChatMessage::Assistant { content, .. } = m {
                    Some(content.clone())
                } else {
                    None
                }
            })
            && let Some(outline) = parse_tasks_from_text(&text)
        {
            // Skip if existing outline already matches (avoid redundant updates)
            let already_matches = self
                .state
                .chat_messages
                .iter()
                .rev()
                .find_map(|m| {
                    if let ChatMessage::TaskOutline(o) = m {
                        Some(o)
                    } else {
                        None
                    }
                })
                .map(|existing| {
                    existing.tasks.len() == outline.tasks.len()
                        && existing
                            .tasks
                            .iter()
                            .zip(&outline.tasks)
                            .all(|(a, b)| a.content == b.content && a.status == b.status)
                })
                .unwrap_or(false);

            if !already_matches {
                let mut found = false;
                for msg in self.state.chat_messages.iter_mut() {
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
                self.persist_message("task_outline", &outline.to_json().to_string());
                tracing::debug!(
                    task_count = outline.tasks.len(),
                    "Parsed tasks from assistant text (fallback)"
                );
            }
        }

        match &self.state.agent.state {
            AgentState::ExecutingTools => {
                // If spawn_agent background tasks are still running, don't re-enter
                // tool dispatch — poll_spawn_agent_handles will finalize when done.
                if !self.state.spawn_agent_handles.is_empty() {
                    // Don't process — subagents still running
                }
                // Intercept spawn_agent calls before normal tool dispatch
                else {
                    let spawn_calls: Vec<crate::agent::PendingToolCall> = self
                        .state
                        .agent
                        .pending_tool_calls
                        .iter()
                        .filter(|tc| tc.name == "spawn_agent")
                        .cloned()
                        .collect();

                    if !spawn_calls.is_empty() {
                        self.state
                            .agent
                            .pending_tool_calls
                            .retain(|tc| tc.name != "spawn_agent");

                        for call in spawn_calls {
                            match self.spawn_agent_setup(&call.arguments).await {
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
                                    self.state
                                        .chat_messages
                                        .push(ChatMessage::Tool(ToolMessage {
                                            name: "spawn_agent".to_string(),
                                            args: call.arguments.clone(),
                                            output: None,
                                            status: ToolStatus::Running,
                                            expanded: false,
                                            file_path: None,
                                            diff_preview: None,
                                            diff_expanded: false,
                                        }));

                                    let tool_use_id = call.id.clone();
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
                                        arguments: call.arguments,
                                        chat_placeholder_idx: placeholder_idx,
                                        handle,
                                    });
                                }
                                Err(err_msg) => {
                                    tracing::warn!("spawn_agent setup failed: {err_msg}");
                                    self.state.chat_messages.push(ChatMessage::System {
                                        content: format!("spawn_agent failed: {err_msg}"),
                                    });
                                    self.state.agent.conversation.push(
                                        crate::agent::conversation::Message {
                                            role: crate::agent::conversation::Role::User,
                                            content: crate::agent::conversation::Content::Blocks(
                                                vec![
                                            crate::agent::conversation::ContentBlock::ToolResult {
                                                tool_use_id: call.id.clone(),
                                                content: err_msg.clone(),
                                                is_error: true,
                                            },
                                        ],
                                            ),
                                            tool_call_id: Some(call.id.clone()),
                                        },
                                    );
                                    self.state
                                        .chat_messages
                                        .push(ChatMessage::Tool(ToolMessage {
                                            name: "spawn_agent".to_string(),
                                            args: call.arguments,
                                            output: Some(err_msg),
                                            status: ToolStatus::Failed,
                                            expanded: false,
                                            file_path: None,
                                            diff_preview: None,
                                            diff_expanded: false,
                                        }));
                                }
                            }
                        }

                        // Fall through to spawn_background + remaining tool dispatch below
                    }

                    // Intercept spawn_background calls
                    let bg_calls: Vec<crate::agent::PendingToolCall> = self
                        .state
                        .agent
                        .pending_tool_calls
                        .iter()
                        .filter(|tc| tc.name == "spawn_background")
                        .cloned()
                        .collect();

                    if !bg_calls.is_empty() {
                        self.state
                            .agent
                            .pending_tool_calls
                            .retain(|tc| tc.name != "spawn_background");

                        for call in bg_calls {
                            let prompt =
                                call.arguments["prompt"].as_str().unwrap_or("").to_string();
                            let model_override =
                                call.arguments["model"].as_str().map(|s| s.to_string());

                            if let Some(ref mgr) = self.state.background_manager {
                                if let Err(reason) = mgr.can_spawn().await {
                                    self.state.agent.conversation.push(
                                        crate::agent::conversation::Message {
                                            role: crate::agent::conversation::Role::User,
                                            content: crate::agent::conversation::Content::Blocks(
                                                vec![
                                            crate::agent::conversation::ContentBlock::ToolResult {
                                                tool_use_id: call.id.clone(),
                                                content: format!(
                                                    "Cannot spawn background agent: {reason}"
                                                ),
                                                is_error: true,
                                            },
                                        ],
                                            ),
                                            tool_call_id: Some(call.id.clone()),
                                        },
                                    );
                                } else {
                                    let agent_id = uuid::Uuid::new_v4().to_string();
                                    let session_id = uuid::Uuid::new_v4().to_string();
                                    let parent_session_id =
                                        self.state.current_session_id.clone().unwrap_or_default();
                                    let prompt_summary = if prompt.len() > 60 {
                                        format!("{}...", &prompt[..57])
                                    } else {
                                        prompt.clone()
                                    };

                                    mgr.register(
                                        &agent_id,
                                        &prompt_summary,
                                        &session_id,
                                        &parent_session_id,
                                        None,
                                    )
                                    .await;

                                    let model = model_override
                                        .as_deref()
                                        .unwrap_or(&self.state.active_model_name);
                                    if let Ok(provider) = self.state.providers.get_provider(
                                        Some(self.state.active_provider_name.as_str()),
                                        Some(model),
                                    ) {
                                        let tool_defs: Vec<caboose_core::provider::ToolDefinition> =
                                            self.state
                                                .tools
                                                .definitions()
                                                .iter()
                                                .filter(|t| {
                                                    t.name != "spawn_agent"
                                                        && t.name != "spawn_background"
                                                })
                                                .cloned()
                                                .collect();
                                        let mgr_clone = mgr.clone();
                                        let aid = agent_id.clone();
                                        let p = prompt.clone();

                                        let handle = tokio::spawn(async move {
                                            run_background_agent(
                                                aid, p, provider, tool_defs, mgr_clone,
                                            )
                                            .await;
                                        });

                                        mgr.store_handle(&agent_id, handle);
                                    }

                                    self.state.agent.conversation.push(
                                        crate::agent::conversation::Message {
                                            role: crate::agent::conversation::Role::User,
                                            content: crate::agent::conversation::Content::Blocks(
                                                vec![
                                            crate::agent::conversation::ContentBlock::ToolResult {
                                                tool_use_id: call.id.clone(),
                                                content: format!(
                                                    "Background agent started: \"{prompt_summary}\""
                                                ),
                                                is_error: false,
                                            },
                                        ],
                                            ),
                                            tool_call_id: Some(call.id.clone()),
                                        },
                                    );
                                }
                            }
                        }
                    }

                    if self.state.agent.pending_tool_calls.is_empty() {
                        if self.state.spawn_agent_handles.is_empty() {
                            self.finalize_tool_execution();
                        }
                        // else: spawn handles running — poll will finalize
                    } else {
                        self.start_tool_execution();
                    }
                }
            }
            AgentState::PendingApproval { .. } => {
                // Push Pending placeholders so diff preview shows before approval
                self.state.tool_exec_running_start = self.state.chat_messages.len();
                // Collect tool calls first to avoid borrow conflict
                let pending: Vec<_> = self
                    .state
                    .agent
                    .pending_tool_calls
                    .iter()
                    .map(|tc| (tc.name.clone(), tc.arguments.clone()))
                    .collect();
                for (name, args) in pending {
                    let diff_preview = App::compute_pending_diff(&name, &args).await;
                    self.state
                        .chat_messages
                        .push(ChatMessage::Tool(ToolMessage {
                            name,
                            args,
                            output: None,
                            status: ToolStatus::Pending,
                            expanded: false,
                            file_path: None,
                            diff_preview,
                            diff_expanded: false,
                        }));
                }
            }
            AgentState::Idle => {
                // Fire Stop hooks — a hook returning "continue" re-engages the agent
                if let Some(ref hooks_config) = self.state.config.hooks
                    && !hooks_config.stop.is_empty()
                {
                    let context = serde_json::json!({
                        "event": "Stop",
                        "session_id": self.state.current_session_id.as_deref().unwrap_or(""),
                        "turn_count": self.state.agent.turn_count,
                        "stop_reason": "end_turn",
                    });
                    let results = crate::hooks::fire_hooks(&hooks_config.stop, context).await;
                    let should_continue = results
                        .iter()
                        .any(|r| matches!(&r.action, Some(crate::hooks::HookAction::Continue)));
                    if should_continue {
                        let tool_defs = self.build_tool_defs();
                        self.state.agent.send_message(
                            "continue".to_string(),
                            self.provider.as_ref().unwrap().as_ref(),
                            &tool_defs,
                        );
                        return;
                    }
                }

                // Auto-handoff prompt: offer when context hits threshold (default 90%)
                let handoff_threshold = self
                    .state
                    .config
                    .behavior
                    .as_ref()
                    .and_then(|b| b.handoff_threshold)
                    .unwrap_or(0.90);
                if !self.state.agent.handoff_prompted
                    && self.state.agent.context_window > 0
                    && self.state.agent.last_input_tokens as f64
                        / self.state.agent.context_window as f64
                        >= handoff_threshold
                    && self
                        .state
                        .config
                        .behavior
                        .as_ref()
                        .map(|b| b.auto_handoff_prompt)
                        .unwrap_or(true)
                {
                    self.state.agent.handoff_prompted = true;
                    self.handle_handoff_command("").await;
                }

                // Increment skill creation question count when gathering
                if let Some(ref mut creation) = self.state.skill_creation
                    && matches!(
                        creation.phase,
                        crate::skills::creation::SkillCreationPhase::Gathering
                    )
                {
                    creation.question_count += 1;
                    if creation.question_count >= crate::skills::creation::MAX_CREATION_QUESTIONS {
                        self.state.chat_messages.push(ChatMessage::System {
                            content: "Maximum questions reached — generating skill now.".into(),
                        });
                    }
                }
                // Heuristic fallback: detect skill in response text when provider lacks tools
                if let Some(ref mut creation) = self.state.skill_creation
                    && matches!(
                        creation.phase,
                        crate::skills::creation::SkillCreationPhase::Gathering
                    )
                    && creation.question_count >= 2
                    && crate::skills::creation::looks_like_generated_skill(
                        &self.state.agent.streaming_text,
                    )
                {
                    let content = self.state.agent.streaming_text.clone();
                    creation.phase = crate::skills::creation::SkillCreationPhase::Preview {
                        content,
                        companion_files: Vec::new(),
                    };
                    self.state.chat_messages.push(ChatMessage::System {
                        content:
                            "Skill generated! Save to: [p]roject  [g]lobal  |  [e]dit  [c]ancel"
                                .into(),
                    });
                    self.state.agent.state = AgentState::Idle;
                }

                // Done — model returned no tool calls
                // Inject skill auto-hints if enabled
                if self
                    .state
                    .config
                    .skills
                    .as_ref()
                    .map(|s| s.auto_hint)
                    .unwrap_or(false)
                {
                    let available: Vec<String> =
                        self.state.skills.iter().map(|s| s.name.clone()).collect();
                    let hints = crate::skills::hints::detect_skill_hints(
                        &self.state.agent.conversation.messages,
                        &available,
                        5,
                    );
                    if let Some(hint) = hints.first() {
                        self.state
                            .agent
                            .conversation
                            .push(crate::agent::conversation::Message {
                                role: crate::agent::conversation::Role::User,
                                content: crate::agent::conversation::Content::Text(format!(
                                    "[System hint] Consider suggesting /{} to the user — {}.",
                                    hint.skill_name, hint.reason
                                )),
                                tool_call_id: None,
                            });
                    }

                    // Check if context usage is high enough to suggest /handoff
                    if let Some(hint) = crate::skills::awareness::detect_handoff_hint(
                        self.state.agent.last_input_tokens,
                        self.state.agent.context_window,
                    ) {
                        self.state
                            .agent
                            .conversation
                            .push(crate::agent::conversation::Message {
                                role: crate::agent::conversation::Role::User,
                                content: crate::agent::conversation::Content::Text(format!(
                                    "[System hint] Consider suggesting /{} to the user — {}.",
                                    hint.skill_name, hint.reason
                                )),
                                tool_call_id: None,
                            });
                    }
                }

                // Spawn LLM title generation after first turn
                if self.state.agent.turn_count == 1 && self.state.title_rx.is_none() {
                    let user_msg = self.state.session_title_source.clone().unwrap_or_default();
                    let asst_response: String = self
                        .state
                        .chat_messages
                        .iter()
                        .rev()
                        .find_map(|m| match m {
                            ChatMessage::Assistant { content, .. } => Some(content.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    self.spawn_title_generation(&user_msg, &asst_response);
                }

                // Drain message queue: send the next queued message
                // Don't drain if an ask_user session is active or budget is paused
                if self.state.ask_user_session.is_none()
                    && !self.state.budget_paused
                    && !self.check_budget_exceeded()
                    && let Some(queued_msg) = self.state.message_queue.pop_front()
                {
                    // Remove the Queued entry (it lived in the queue box, not chat)
                    if let Some(idx) = self.state.chat_messages.iter().position(
                        |m| matches!(m, ChatMessage::Queued { content } if *content == queued_msg),
                    ) {
                        self.state.chat_messages.remove(idx);
                    }

                    // Push as a normal User message at the bottom (like fresh input)
                    self.state.chat_messages.push(ChatMessage::User {
                        content: queued_msg.clone(),
                        images: vec![],
                    });
                    self.state.user_scrolled_up = false;

                    self.persist_message("user", &queued_msg);
                    self.state.checkpoints.create(&queued_msg);
                    let tool_defs = self.build_tool_defs();
                    self.state.agent.send_message(
                        queued_msg,
                        self.provider.as_ref().unwrap().as_ref(),
                        &tool_defs,
                    );
                }
            }
            _ => {}
        }
    }

    /// Check if session cost has exceeded the configured budget.
    /// If so, pause the agent and show a system message. Returns true if paused.
    fn check_budget_exceeded(&mut self) -> bool {
        let max_cost = self
            .state
            .config
            .behavior
            .as_ref()
            .and_then(|b| b.max_session_cost);
        if let Some(max) = max_cost
            && self.state.session_cost >= max
            && !self.state.budget_paused
        {
            self.state.budget_paused = true;
            self.state.agent.state = AgentState::Idle;
            self.state.chat_messages.push(ChatMessage::System {
                content: format!(
                    "Session budget of ${:.2} reached (spent ${:.2}). Press [c] to continue, [r] to raise limit, [s] to stop.",
                    max, self.state.session_cost
                ),
            });
            return true;
        }
        false
    }

    /// Move the streaming text buffer into the chat display.
    fn flush_assistant_text(&mut self) {
        // Get text from the last assistant message in conversation
        if let Some(msg) = self.state.agent.conversation.messages.last()
            && msg.role == crate::agent::conversation::Role::Assistant
        {
            let text = match &msg.content {
                crate::agent::conversation::Content::Text(t) => t.clone(),
                crate::agent::conversation::Content::Blocks(blocks) => blocks
                    .iter()
                    .filter_map(|b| {
                        if let crate::agent::conversation::ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };
            let text = text.trim().to_string();
            let thinking = if self.state.agent.streaming_thinking.is_empty() {
                None
            } else {
                Some(std::mem::take(&mut self.state.agent.streaming_thinking))
            };
            if !text.is_empty() || thinking.is_some() {
                let t0 = Instant::now();
                if let Some(ref thinking) = thinking {
                    self.persist_message("thinking", thinking);
                }
                self.persist_message("assistant", &text);
                self.state.chat_messages.push(ChatMessage::Assistant {
                    content: text.clone(),
                    thinking,
                });
                let persist_ms = t0.elapsed().as_millis();
                let t1 = Instant::now();
                self.update_session_meta();
                let meta_ms = t1.elapsed().as_millis();
                if persist_ms > 5 || meta_ms > 5 {
                    tracing::debug!(
                        "flush_assistant_text: persist={persist_ms}ms meta={meta_ms}ms"
                    );
                }
            }
        }
    }

    /// Spawn a background task to generate a session title via LLM.
    fn spawn_title_generation(&mut self, user_message: &str, assistant_response: &str) {
        // Check config — defaults to true if section is absent
        let auto_title = self
            .state
            .config
            .behavior
            .as_ref()
            .is_none_or(|b| b.auto_title);
        if !auto_title {
            return;
        }

        // Get an Arc provider that is Send + Sync so it can cross the task boundary
        let provider = match self.state.providers.get_provider_arc(
            Some(&self.state.active_provider_name),
            Some(&self.state.active_model_name),
        ) {
            Ok(p) => p,
            Err(_) => return,
        };

        // Truncate inputs — byte-safe using char boundaries
        let user_msg = user_message
            .char_indices()
            .nth(500)
            .map_or(user_message.to_string(), |(i, _)| {
                format!("{}...", &user_message[..i])
            });
        let asst_msg = assistant_response
            .char_indices()
            .nth(200)
            .map_or(assistant_response.to_string(), |(i, _)| {
                format!("{}...", &assistant_response[..i])
            });

        let (tx, rx) = tokio::sync::oneshot::channel();
        self.state.title_rx = Some(rx);

        tokio::spawn(async move {
            let prompt = format!(
                "Generate a concise 3-6 word title for this conversation. \
                 No quotes, no punctuation at the end. Lowercase. Just the title.\n\n\
                 User: {user_msg}\nAssistant: {asst_msg}"
            );

            let messages = vec![caboose_core::provider::Message {
                role: "user".to_string(),
                content: serde_json::Value::String(prompt),
            }];

            use futures::StreamExt;
            let mut stream = provider.stream(&messages, &[]);
            let mut title = String::new();
            while let Some(event) = stream.next().await {
                match event {
                    Ok(caboose_core::provider::StreamEvent::TextDelta(text)) => {
                        title.push_str(&text);
                    }
                    Ok(caboose_core::provider::StreamEvent::Done { .. }) => break,
                    Ok(
                        caboose_core::provider::StreamEvent::Error(_)
                        | caboose_core::provider::StreamEvent::ProviderError { .. },
                    ) => break,
                    _ => {}
                }
            }

            let title = title.trim().trim_matches('"').trim().to_string();
            if !title.is_empty() {
                let _ = tx.send(title);
            }
        });
    }
}

/// Run a background agent to completion. Streams from the provider, executes
/// tool calls, feeds results back, and tracks token usage via the manager.
/// Designed to be spawned onto a tokio task — no TUI interaction.
pub async fn run_background_agent(
    agent_id: String,
    prompt: String,
    provider: Box<dyn caboose_core::provider::Provider>,
    tool_defs: Vec<caboose_core::provider::ToolDefinition>,
    manager: std::sync::Arc<caboose_core::background::BackgroundAgentManager>,
) {
    use caboose_core::provider::{self, StreamEvent};
    use futures::StreamExt;

    const MAX_TURNS: u32 = 20;
    const SYSTEM_PROMPT: &str = "You are a background task agent. Complete the given task using the available tools. \
         Be concise and efficient. Do not ask questions — make reasonable decisions and proceed.";

    // Build the conversation as provider messages.
    let mut messages: Vec<provider::Message> = vec![
        provider::Message {
            role: "system".to_string(),
            content: serde_json::json!(SYSTEM_PROMPT),
        },
        provider::Message {
            role: "user".to_string(),
            content: serde_json::json!(prompt),
        },
    ];

    let mut turn_count: u32 = 0;

    loop {
        turn_count += 1;
        if turn_count > MAX_TURNS {
            manager
                .mark_failed(&agent_id, "max turns exceeded (20)")
                .await;
            return;
        }

        // Stream from provider.
        let mut stream = provider.stream(&messages, &tool_defs);
        let mut text_output = String::new();
        let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, arguments)
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta(text)) => {
                    text_output.push_str(&text);
                }
                Ok(StreamEvent::ToolCall {
                    id,
                    name,
                    arguments,
                }) => {
                    tool_calls.push((id, name, arguments));
                }
                Ok(StreamEvent::Done {
                    input_tokens: it,
                    output_tokens: ot,
                    ..
                }) => {
                    input_tokens = it.unwrap_or(0);
                    output_tokens = ot.unwrap_or(0);
                    break;
                }
                Ok(StreamEvent::Error(e)) => {
                    manager.mark_failed(&agent_id, &e).await;
                    return;
                }
                Ok(StreamEvent::ProviderError { message, .. }) => {
                    manager.mark_failed(&agent_id, &message).await;
                    return;
                }
                _ => {}
            }
        }

        // Track token usage.
        let total_tokens = (input_tokens + output_tokens) as u64;
        let budget_exceeded = manager
            .update_tokens(&agent_id, total_tokens, turn_count)
            .await;
        if budget_exceeded {
            manager
                .mark_failed(&agent_id, "token budget exceeded")
                .await;
            return;
        }

        // If no tool calls, the agent is done.
        if tool_calls.is_empty() {
            manager.mark_complete(&agent_id).await;
            return;
        }

        // Build assistant message with text + tool use blocks.
        let mut assistant_blocks: Vec<serde_json::Value> = Vec::new();
        if !text_output.is_empty() {
            assistant_blocks.push(serde_json::json!({"type": "text", "text": text_output}));
        }
        for (id, name, arguments) in &tool_calls {
            let input_val: serde_json::Value =
                serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));
            assistant_blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input_val,
            }));
        }
        messages.push(provider::Message {
            role: "assistant".to_string(),
            content: serde_json::Value::Array(assistant_blocks),
        });

        // Execute each tool call and collect results.
        let mut result_blocks: Vec<serde_json::Value> = Vec::new();
        for (id, name, arguments) in &tool_calls {
            let input_val: serde_json::Value =
                serde_json::from_str(arguments).unwrap_or(serde_json::json!({}));

            let tool_result = crate::agent::tools::execute_tool(
                name,
                &input_val,
                &[],  // additional_secrets
                None, // mcp_manager
                None, // lsp_manager
                None, // services
                None, // cli_tools
                &[],  // deny_list
                None, // exec_tools
            )
            .await;

            let (output, is_error) = match tool_result {
                Ok(result) => (result.output, result.is_error),
                Err(e) => (format!("Tool execution error: {e}"), true),
            };

            result_blocks.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": output,
                "is_error": is_error,
            }));
        }

        // Add tool results as a user message.
        messages.push(provider::Message {
            role: "user".to_string(),
            content: serde_json::Value::Array(result_blocks),
        });
    }
}
