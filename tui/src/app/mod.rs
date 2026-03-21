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

mod handoff;
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

    fn handle_roundhouse_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
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

    async fn handle_home_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
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

    async fn handle_chat_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
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

    async fn handle_approval_key(&mut self, key: KeyCode) {
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

    /// Handle a key press when the dropdown is in a picker mode.
    async fn handle_picker_key(&mut self, key: KeyCode) {
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
    fn refresh_session_search(&mut self) {
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
    async fn handle_session_picker_confirm(&mut self, key: KeyCode) {
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
    async fn handle_picker_select(&mut self) {
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

    /// Count of selectable items in current picker mode.
    fn picker_item_count(&self) -> usize {
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

    fn handle_file_browser_key(&mut self, key: KeyCode) {
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

    fn handle_agents_list_key(&mut self, key: KeyCode) {
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

    fn handle_circuits_list_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
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

    fn handle_migration_checklist_key(&mut self, key: KeyCode) {
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

    async fn handle_key_input_key(&mut self, key: KeyCode) {
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

    async fn handle_local_connect_key(&mut self, key: KeyCode) {
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

    fn handle_mcp_input_key(&mut self, key: KeyCode) {
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

    fn handle_agent_stream_overlay_key(
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

    fn handle_workspace_list_key(&mut self, key: crossterm::event::KeyCode) {
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

    async fn handle_workspace_add_key(&mut self, key: crossterm::event::KeyCode) {
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

    async fn handle_workspace_add_confirm(&mut self) {
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
    fn refresh_workspace_list_state(&mut self) {
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

    fn handle_mcp_input_submit(&mut self) {
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

    async fn handle_command_palette_key(&mut self, key: KeyCode) {
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

                        if self.state.agent.pending_tool_calls.is_empty() {
                            if self.state.spawn_agent_handles.is_empty() {
                                self.finalize_tool_execution();
                            }
                            // else: spawn handles running — poll will finalize
                        } else {
                            self.start_tool_execution();
                        }
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

    /// Open the settings picker with current config values.
    fn open_settings_picker(&mut self) {
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
    fn open_rewind_picker(&mut self) {
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
