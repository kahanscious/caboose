//! Main layout — composes header, chat, sidebar, input, and footer.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::agent::AgentState;
use crate::agent::permission::Mode;
use crate::app::{ChatMessage, State, ToolStatus};
use crate::tui::dialog::{DialogKind, Screen};
use crate::tui::theme;

/// Map current mode to its accent color.
fn mode_accent(mode: Mode, colors: &theme::Colors) -> Color {
    match mode {
        Mode::Plan => colors.info,
        Mode::Create => colors.brand,
        Mode::Chug => colors.warning,
    }
}

/// Returns true if this tool message should render a diff toggle indicator (▶/▼).
fn has_diff_toggle(tool: &crate::app::ToolMessage) -> bool {
    match tool.status {
        crate::app::ToolStatus::Pending => tool.diff_preview.is_some(),
        crate::app::ToolStatus::Success => {
            (tool.name == "edit_file"
                && tool
                    .args
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .is_some()
                && tool
                    .args
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .is_some())
                || (tool.name == "apply_patch"
                    && tool.args.get("diff").and_then(|v| v.as_str()).is_some())
        }
        _ => false,
    }
}

/// Apply a turn margin indicator to a line, replacing the leading indent with │.
fn apply_turn_margin(line: &mut Line, accent_style: Style) {
    if line.spans.is_empty() {
        // Line::from("") creates empty spans in ratatui — treat as separator connector
        line.spans
            .push(Span::styled("\u{2502}".to_string(), accent_style));
        return;
    }

    let content = line.spans[0].content.to_string();

    // Skip role headers (● ), tool dot nodes (• ), and focused tool indicators (▸ )
    if content.starts_with("● ")
        || content.starts_with("\u{2022} ")
        || content.starts_with("\u{25b8} ")
    {
        return;
    }

    // Skip code block lines (have background color)
    if line.spans[0].style.bg.is_some() {
        return;
    }

    // Empty separator line → thin connector
    if line.spans.len() == 1 && content.is_empty() {
        line.spans[0] = Span::styled("\u{2502}".to_string(), accent_style);
        return;
    }

    // Separate indent span (exactly "  ")
    if content == "  " {
        line.spans[0] = Span::styled("\u{2502} ".to_string(), accent_style);
        return;
    }

    // Merged content starting with "  " — split into margin + rest
    if let Some(stripped) = content.strip_prefix("  ") {
        let rest = stripped.to_string();
        let original_style = line.spans[0].style;
        line.spans[0] = Span::styled("\u{2502} ".to_string(), accent_style);
        if !rest.is_empty() {
            line.spans.insert(1, Span::styled(rest, original_style));
        }
    }
}

const SIDEBAR_MIN_TERMINAL_WIDTH: u16 = 120;
pub const SIDEBAR_MIN_WIDTH: u16 = 20;
pub const SIDEBAR_MAX_WIDTH: u16 = 80;

/// Render the full application layout.
pub fn render(frame: &mut Frame, app: &State) {
    let colors = theme::Colors::default();

    // Render base screen
    match app.dialog_stack.base {
        Screen::Home => {
            crate::tui::home::render(frame, app);
        }
        Screen::Chat => {
            render_chat_layout(frame, app, &colors);
        }
    }

    // Render top overlay if any
    if let Some(dialog) = app.dialog_stack.top() {
        match dialog {
            DialogKind::ApiKeyInput(state) => {
                crate::tui::key_input::render(frame, state);
            }
            DialogKind::FileBrowser(state) => {
                crate::tui::file_browser::render(frame, state, &colors);
            }
            DialogKind::McpServerInput(state) => {
                crate::tui::mcp_input::render(frame, state);
            }
            DialogKind::CommandPalette(palette) => {
                crate::tui::command_palette::render(frame, palette, app, &colors);
            }
            DialogKind::PasteConfirm {
                line_count,
                char_count,
                ..
            } => {
                render_paste_confirm(frame, *line_count, *char_count, &colors);
            }
            DialogKind::LocalProviderConnect(state) => {
                render_local_connect(frame, state, &colors);
            }
            DialogKind::RoundhouseProviderPicker(picker) => {
                render_roundhouse_picker(frame, picker, app, &colors);
            }
            DialogKind::CircuitsList(list_state) => {
                render_circuits_list(frame, list_state, app, &colors);
            }
            DialogKind::MigrationChecklist(checklist) => {
                render_migration_checklist(frame, checklist, &colors);
            }
            DialogKind::WorkspaceList(state) => {
                crate::tui::workspace_list::render(frame, frame.area(), state);
            }
            DialogKind::WorkspaceAdd(state) => {
                crate::tui::workspace_add::render(frame, frame.area(), state);
            }
            DialogKind::AgentStreamOverlay(overlay_state) => {
                render_agent_stream_overlay(frame, frame.area(), overlay_state, app, &colors);
            }
        }
    }
}

/// Render the full chat layout (header, chat, sidebar, input, footer).
fn render_chat_layout(frame: &mut Frame, app: &State, colors: &theme::Colors) {
    let area = frame.area();

    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(colors.bg_primary)),
        area,
    );

    let show_sidebar = app.sidebar_visible && area.width >= SIDEBAR_MIN_TERMINAL_WIDTH;

    let terminal_visible = app
        .terminal_panel
        .as_ref()
        .map(|p| p.visible)
        .unwrap_or(false);
    let terminal_height = if terminal_visible {
        (area.height * 25 / 100).max(6)
    } else {
        0
    };

    // Top-level vertical split: [main content | footer | terminal?]
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if terminal_visible {
            vec![
                Constraint::Min(1),                  // Main content (header + chat + input)
                Constraint::Length(4),               // Footer
                Constraint::Length(terminal_height), // Terminal panel (bottommost)
            ]
        } else {
            vec![
                Constraint::Min(1),    // Main content (header + chat + input)
                Constraint::Length(4), // Footer
            ]
        })
        .split(area);

    // Horizontal split: [chat area | sidebar] (sidebar optional)
    let h_constraints = if show_sidebar {
        vec![Constraint::Min(1), Constraint::Length(app.sidebar_width)]
    } else {
        vec![Constraint::Min(1)]
    };
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(h_constraints)
        .split(outer[0]);

    let main_area = h_chunks[0];

    // Main area vertical split: [header | chat | input]
    let text_width = (main_area.width as usize).saturating_sub(3).max(1);
    let extra_lines = app
        .input
        .visual_line_count(text_width as u16)
        .saturating_sub(1)
        .min(4) as u16;
    // Reserve space for queued messages box above input (border + 1 line per msg)
    let queue_height = if app.message_queue.is_empty() {
        0u16
    } else {
        app.message_queue.len() as u16 + 2
    };
    let has_approval = matches!(app.agent.state, AgentState::PendingApproval { .. })
        || app.sub_agent_approval_showing.is_some();
    let approval_height = if has_approval {
        crate::tui::approval::APPROVAL_BAR_HEIGHT
    } else {
        0u16
    };
    let attachment_height = if app.attachments.is_empty() { 0u16 } else { 1 };
    let input_height = 4 + extra_lines + queue_height + approval_height + attachment_height;

    let v_constraints: Vec<Constraint> = vec![
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(input_height),
    ];

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(v_constraints)
        .split(main_area);

    // --- Header bar ---
    crate::tui::header::render(frame, v_chunks[0]);

    // --- Chat area ---
    render_chat(frame, v_chunks[1], app, colors);

    // --- Input area ---
    render_input(frame, v_chunks[2], app, colors);

    // --- Sidebar ---
    if show_sidebar {
        let mcp_servers: Vec<(String, crate::mcp::ServerStatus, usize, bool)> = {
            let mut servers: Vec<_> = app
                .mcp_manager
                .servers
                .values()
                .filter(|s| {
                    !s.config.disabled || matches!(s.status, crate::mcp::ServerStatus::Connected)
                })
                .map(|s| (s.name.clone(), s.status.clone(), s.tools.len(), s.is_preset))
                .collect();
            servers.sort_by(|a, b| a.0.cmp(&b.0));
            servers
        };

        // Find the latest TaskOutline from chat messages
        let task_outline = app.chat_messages.iter().rev().find_map(|msg| {
            if let ChatMessage::TaskOutline(outline) = msg {
                Some(outline)
            } else {
                None
            }
        });

        let dismiss_row = crate::tui::sidebar::render(
            frame,
            h_chunks[1],
            app.agent.last_input_tokens,
            app.agent.last_output_tokens,
            app.agent.context_window,
            app.agent.turn_count,
            app.agent.last_tokens_per_sec,
            &mcp_servers,
            &app.active_model_name,
            &app.pricing,
            &app.modified_files,
            task_outline,
            app.tick,
            app.roundhouse_session.as_ref(),
            &app.active_watchers,
            &app.sub_agents,
        );
        app.agents_dismiss_row.set(dismiss_row);
    }

    // --- Footer ---
    let budget = app
        .config
        .behavior
        .as_ref()
        .and_then(|b| b.max_session_cost)
        .map(|max| crate::tui::footer::BudgetInfo {
            session_cost: app.session_cost,
            max_cost: max,
        });
    let is_active = matches!(
        app.agent.state,
        crate::agent::AgentState::Streaming
            | crate::agent::AgentState::ExecutingTools
            | crate::agent::AgentState::PendingApproval { .. }
            | crate::agent::AgentState::Compacting
    ) || app.init_rx.is_some();
    crate::tui::footer::render(
        frame,
        outer[1],
        app.mode,
        app.caboose_pos,
        is_active,
        budget,
        app.update_available.as_deref(),
    );

    // --- Terminal panel (bottommost, below footer) ---
    if terminal_visible {
        if let Some(panel) = &app.terminal_panel {
            let terminal_area = outer[2];
            app.terminal_area.set(Some(terminal_area));
            let widget = crate::terminal::widget::TerminalWidget {
                panel,
                focused: app.terminal_focused,
                colors,
            };
            frame.render_widget(widget, terminal_area);
        }
    } else {
        app.terminal_area.set(None);
    }
}

/// Render the chat message area.
fn render_chat(frame: &mut Frame, area: Rect, app: &State, colors: &theme::Colors) {
    // Add left padding so content isn't flush against the edge.
    // Use 4 columns so text aligns with message headers (● You / ● Caboose)
    // and wrapped lines stay aligned (ratatui wraps to column 0 of area).
    let area = Rect {
        x: area.x + 4,
        width: area.width.saturating_sub(4),
        ..area
    };
    // Store padded chat area rect for mouse hit-testing
    app.chat_area.set(Some(area));

    let mut lines: Vec<Line> = Vec::new();
    // Track (logical_line_index, message_index) for truncation indicators
    let mut truncation_lines: Vec<(usize, usize)> = Vec::new();
    // Track (logical_line_index, message_index) for diff toggle indicators
    let mut tool_toggle_lines: Vec<(usize, usize)> = Vec::new();
    // Track (logical_line_index, message_index) for thinking block toggle indicators
    let mut thinking_lines: Vec<(usize, usize)> = Vec::new();
    // Track message boundaries for connector grouping: 0=other, 1=tool, 2=assistant
    let mut msg_boundaries: Vec<(usize, u8)> = Vec::new();

    // Index of the last Pending tool message — receives live diff state.
    let last_pending_idx = app.chat_messages.iter().rposition(|m| {
        if let ChatMessage::Tool(t) = m {
            t.status == ToolStatus::Pending
        } else {
            false
        }
    });

    for (i, msg) in app.chat_messages.iter().enumerate() {
        let start_idx = lines.len();
        let kind = match msg {
            ChatMessage::Tool(_) => 1u8,
            ChatMessage::Assistant { .. } => 2u8,
            _ => 0u8,
        };
        msg_boundaries.push((start_idx, kind));
        let msg_lines = match msg {
            ChatMessage::User { content, images } => {
                let accent = mode_accent(app.mode, colors);
                crate::tui::chat::render_user_message(content, images, colors, accent)
            }
            ChatMessage::Assistant {
                content, thinking, ..
            } => {
                let mut msg_lines = Vec::new();

                // Thinking block (if present) — rendered above the assistant header
                if let Some(thinking_text) = thinking {
                    let thinking_expanded = app.expanded_thinking.contains(&i);
                    let thinking_rendered = crate::tui::chat::render_thinking_block(
                        thinking_text,
                        !thinking_expanded,
                        colors,
                        app.tick,
                    );
                    // Record logical line index of the arrow for post-render click zone computation
                    thinking_lines.push((start_idx + msg_lines.len(), i));
                    msg_lines.extend(thinking_rendered);
                }

                // Standard assistant rendering (header + truncated text)
                let expanded = app.expanded_messages.contains(&i);
                let accent = mode_accent(app.mode, colors);
                msg_lines.extend(crate::tui::chat::render_assistant_message_truncated(
                    content, colors, expanded, accent,
                ));
                msg_lines
            }
            ChatMessage::Tool(tool_msg) => {
                let focused = app.focused_tool == Some(i);
                let (de, ds) =
                    if tool_msg.status == ToolStatus::Pending && Some(i) == last_pending_idx {
                        (app.diff_expanded, app.diff_scroll)
                    } else {
                        (tool_msg.diff_expanded, 0)
                    };
                let rendered = app
                    .tool_renderers
                    .render(tool_msg, colors, focused, app.tick, de, ds);
                // Record the header row for mouse click hit-testing.
                if has_diff_toggle(tool_msg) {
                    tool_toggle_lines.push((lines.len(), i));
                }
                rendered
            }
            ChatMessage::System { content } => {
                crate::tui::chat::render_system_message(content, colors)
            }
            ChatMessage::ProviderError {
                category,
                provider,
                message,
                hint,
            } => crate::tui::chat::render_provider_error(
                category,
                provider,
                message,
                hint.as_deref(),
                colors,
            ),
            ChatMessage::Error { content } => {
                crate::tui::chat::render_error_message(content, colors)
            }
            ChatMessage::TaskOutline(outline) => {
                crate::tui::tools::todo::render(outline, colors, app.tick)
            }
            ChatMessage::Skill { name, description } => {
                crate::tui::tools::skill::render(name, description, colors)
            }
            ChatMessage::Queued { .. } => {
                // Queued messages render in the input area box, not inline in chat
                Vec::new()
            }
            ChatMessage::AskUser {
                header,
                question,
                options,
                answer,
                multi_select,
            } => {
                let empty_set = std::collections::HashSet::new();
                let toggled = app
                    .ask_user_session
                    .as_ref()
                    .map(|s| &s.toggled)
                    .unwrap_or(&empty_set);
                let accent = mode_accent(app.mode, colors);
                crate::tui::ask_user::render_question(
                    header,
                    question,
                    options,
                    answer.as_deref(),
                    *multi_select,
                    toggled,
                    colors,
                    accent,
                )
            }
        };
        // Detect truncation indicator lines by content
        for (offset, line) in msg_lines.iter().enumerate() {
            if line
                .spans
                .iter()
                .any(|s| s.content.contains("lines hidden"))
            {
                truncation_lines.push((start_idx + offset, i));
            }
        }
        lines.extend(msg_lines);
    }

    // Show streaming thinking block (when thinking arrives before text)
    if matches!(app.agent.state, AgentState::Streaming) && !app.agent.streaming_thinking.is_empty()
    {
        // If text hasn't started yet, add assistant header so thinking has context
        if app.agent.streaming_text.is_empty() {
            msg_boundaries.push((lines.len(), 2u8)); // assistant streaming
            let accent = mode_accent(app.mode, colors);
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(colors.text_dim)),
                Span::styled("Caboose", Style::default().fg(colors.text_secondary).bold()),
            ]));
            let _ = accent; // suppress unused warning
        }
        let collapsed = !app.expanded_thinking.contains(&usize::MAX);
        thinking_lines.push((lines.len(), usize::MAX));
        let thinking_rendered = crate::tui::chat::render_thinking_block(
            &app.agent.streaming_thinking,
            collapsed,
            colors,
            app.tick,
        );
        lines.extend(thinking_rendered);
    }

    // Show streaming text if actively streaming
    if matches!(app.agent.state, AgentState::Streaming) && !app.agent.streaming_text.is_empty() {
        let accent = mode_accent(app.mode, colors);
        if app.agent.streaming_thinking.is_empty() {
            // No thinking — render full assistant message with header (existing behavior)
            msg_boundaries.push((lines.len(), 2u8)); // assistant streaming
            let streaming_lines = crate::tui::chat::render_assistant_message(
                &app.agent.streaming_text,
                colors,
                accent,
            );
            // Remove the trailing blank line and add an animated spinner
            if !streaming_lines.is_empty() {
                let mut sl = streaming_lines;
                // Remove trailing blank separator
                if sl.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
                    sl.pop();
                }
                lines.extend(sl);
            }
        } else {
            // Thinking already rendered header — just render text content without header
            let parsed =
                crate::tui::chat::parse_markdown(&app.agent.streaming_text, colors, accent);
            if !parsed.is_empty() {
                let mut sl = parsed;
                if sl.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
                    sl.pop();
                }
                lines.extend(sl);
            }
        }
        // Animated spinner on last line (rotates every 5 ticks ~4 Hz)
        const SPINNER: &[&str] = &["◐", "◓", "◑", "◒"];
        let spinner = SPINNER[(app.tick / 5) as usize % SPINNER.len()];
        let accent = mode_accent(app.mode, colors);
        lines.push(Line::from(Span::styled(
            format!("{spinner} "),
            Style::default().fg(accent),
        )));
    }

    // Show Roundhouse streaming text in main window (planning + synthesis)
    if let Some(ref rh) = app.roundhouse_session {
        let accent = colors.roundhouse;

        // Primary planner streaming
        if !rh.primary_streaming_text.is_empty() {
            let still_streaming = matches!(
                rh.primary_status,
                crate::roundhouse::PlannerStatus::Streaming
                    | crate::roundhouse::PlannerStatus::Thinking
                    | crate::roundhouse::PlannerStatus::UsingTool(_)
            );
            msg_boundaries.push((lines.len(), 2u8));
            lines.push(Line::from(vec![
                Span::styled("\u{25CF} ", Style::default().fg(accent)),
                Span::styled(
                    format!("{} ", rh.primary_provider),
                    Style::default().fg(colors.text_secondary).bold(),
                ),
                Span::styled("(planning)", Style::default().fg(colors.text_muted)),
            ]));
            let parsed =
                crate::tui::chat::parse_markdown(&rh.primary_streaming_text, colors, accent);
            if !parsed.is_empty() {
                let mut sl = parsed;
                if sl.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
                    sl.pop();
                }
                lines.extend(sl);
            }
            if still_streaming {
                const RH_SPINNER: &[&str] = &["\u{25D0}", "\u{25D3}", "\u{25D1}", "\u{25D2}"];
                let spinner = RH_SPINNER[(app.tick / 5) as usize % RH_SPINNER.len()];
                lines.push(Line::from(Span::styled(
                    format!("{spinner} "),
                    Style::default().fg(accent),
                )));
            }
        }

        // Synthesis streaming
        if !rh.synthesis_streaming_text.is_empty() {
            let still_synthesizing =
                matches!(rh.phase, crate::roundhouse::RoundhousePhase::Synthesizing);
            msg_boundaries.push((lines.len(), 2u8));
            lines.push(Line::from(vec![
                Span::styled("\u{25CF} ", Style::default().fg(accent)),
                Span::styled(
                    format!("{} ", rh.primary_provider),
                    Style::default().fg(colors.text_secondary).bold(),
                ),
                Span::styled("(synthesizing)", Style::default().fg(colors.text_muted)),
            ]));
            let parsed =
                crate::tui::chat::parse_markdown(&rh.synthesis_streaming_text, colors, accent);
            if !parsed.is_empty() {
                let mut sl = parsed;
                if sl.last().map(|l| l.spans.is_empty()).unwrap_or(false) {
                    sl.pop();
                }
                lines.extend(sl);
            }
            if still_synthesizing {
                const RH_SPINNER: &[&str] = &["\u{25D0}", "\u{25D3}", "\u{25D1}", "\u{25D2}"];
                let spinner = RH_SPINNER[(app.tick / 5) as usize % RH_SPINNER.len()];
                lines.push(Line::from(Span::styled(
                    format!("{spinner} "),
                    Style::default().fg(accent),
                )));
            }
            // Show plan file path after synthesis completes
            if let Some(ref path) = rh.plan_file {
                let cwd = std::env::current_dir().unwrap_or_default();
                let display = path
                    .strip_prefix(&cwd)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("Saved to ", Style::default().fg(colors.text_muted)),
                    Span::styled(display, Style::default().fg(colors.text_secondary).bold()),
                ]));
            }
        }
    }

    // Show streaming text during /init generation
    if app.init_rx.is_some() && !app.init_text.is_empty() {
        msg_boundaries.push((lines.len(), 0u8)); // init generation (other)
        // Header: ● CABOOSE.md (using mode accent color)
        let init_accent = mode_accent(app.mode, colors);
        lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(init_accent)),
            Span::styled(
                "CABOOSE.md",
                Style::default().fg(colors.text_secondary).bold(),
            ),
        ]));
        // Strip leading "# CABOOSE.md" line to avoid duplicate header
        let display_text = if app.init_text.starts_with("# CABOOSE.md") {
            app.init_text.split_once('\n').map(|x| x.1).unwrap_or("")
        } else {
            &app.init_text
        };
        let parsed = crate::tui::chat::parse_markdown(display_text, colors, init_accent);
        lines.extend(parsed);
        // Animated spinner on last line
        const SPINNER: &[&str] = &["◐", "◓", "◑", "◒"];
        let spinner = SPINNER[(app.tick / 5) as usize % SPINNER.len()];
        lines.push(Line::from(Span::styled(
            format!("{spinner} "),
            Style::default().fg(init_accent),
        )));
    }

    // End of turn-tracked content (status indicators below are not part of turns)
    let turn_content_end = lines.len();

    // Apply turn margin indicators (│ connecting lines)
    {
        let accent = mode_accent(app.mode, colors);
        let accent_style = Style::default().fg(accent);

        // Build connected ranges: Caboose sections (assistant + tools).
        // │ connects all dots, but the LAST assistant message's text in a
        // section is excluded — it's the terminal node with no │ through it.
        let mut turn_ranges: Vec<(usize, usize)> = Vec::new();
        let mut section_start: Option<usize> = None;
        let mut last_assistant: Option<usize> = None;
        for &(start_idx, kind) in &msg_boundaries {
            if kind == 0 {
                // user/system/error — section break
                if let Some(start) = section_start {
                    let end = last_assistant.unwrap_or(start_idx);
                    if end > start {
                        turn_ranges.push((start, end));
                    }
                    section_start = None;
                    last_assistant = None;
                }
            } else {
                if section_start.is_none() {
                    section_start = Some(start_idx);
                }
                if kind == 2 {
                    // assistant
                    last_assistant = Some(start_idx);
                } else {
                    // tool — clears last_assistant since tools follow
                    last_assistant = None;
                }
            }
        }
        if let Some(start) = section_start {
            let end = last_assistant.unwrap_or(turn_content_end);
            if end > start {
                turn_ranges.push((start, end));
            }
        }

        // Post-process lines within each turn to add │ margin
        for (start, end) in turn_ranges {
            for idx in start..end.min(lines.len()) {
                apply_turn_margin(&mut lines[idx], accent_style);
            }
        }
    }

    // Show animated status indicators for non-idle states
    {
        const THINKING_PHRASES: &[&str] = &[
            "Thinking...",
            "Working...",
            "Caboosing...",
            "Chugging along...",
            "Choo chooing...",
        ];
        // Rotate phrase every ~2.5 seconds (50 ticks at 20 FPS)
        const PHRASE_TICKS: u64 = 50;
        let phrase_idx = (app.tick / PHRASE_TICKS) as usize;
        // Typewriter: reveal one char every 2 ticks (~10 chars/sec)
        let chars_visible = ((app.tick % PHRASE_TICKS) / 2 + 1) as usize;

        /// Truncate a string to at most `n` characters.
        fn typewriter(s: &str, n: usize) -> String {
            s.chars().take(n).collect()
        }

        /// Map tool name to "-ing" form.
        fn tool_label(name: &str) -> &str {
            match name {
                "read_file" => "Reading...",
                "write_file" => "Writing...",
                "edit_file" => "Editing...",
                "list_directory" => "Listing...",
                "glob" => "Searching...",
                "grep" => "Grepping...",
                "shell_command" => "Running...",
                "fetch_url" => "Fetching...",
                "mcp_tool" => "Calling tool...",
                _ => "Running...",
            }
        }

        const EXEC_IDLE_PHRASES: &[&str] = &[
            "Chugging...",
            "Choo chooing...",
            "Full steam ahead...",
            "On the rails...",
            "Hauling freight...",
        ];

        match &app.agent.state {
            AgentState::Streaming
                if app.agent.streaming_text.is_empty()
                    && app.agent.streaming_thinking.is_empty() =>
            {
                let phrase = THINKING_PHRASES[phrase_idx % THINKING_PHRASES.len()];
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", typewriter(phrase, chars_visible)),
                    Style::default().fg(colors.text_muted),
                )));
            }
            AgentState::ExecutingTools => {
                // Tool exec queue holds remaining tools; first item is the next to run
                let fallback = EXEC_IDLE_PHRASES[phrase_idx % EXEC_IDLE_PHRASES.len()];
                let label = app
                    .tool_exec_queue
                    .front()
                    .map(|tc| tool_label(&tc.name))
                    .unwrap_or(fallback);
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", typewriter(label, chars_visible)),
                    Style::default().fg(colors.text_muted),
                )));
            }
            AgentState::Compacting => {
                let phrase = "Compacting conversation...";
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", typewriter(phrase, chars_visible)),
                    Style::default().fg(colors.warning),
                )));
            }
            _ => {}
        }

        // Show animated status when /init is streaming but no text yet
        if app.init_rx.is_some() && app.init_text.is_empty() {
            let phrase = "Generating CABOOSE.md...";
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", typewriter(phrase, chars_visible)),
                Style::default().fg(colors.warning),
            )));
        }
    }

    // Breathing room between last message and input field
    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Compute total wrapped lines and truncation click zones before moving lines into Paragraph.
    let total_lines: u16;
    {
        let mut zones: Vec<(u16, usize)> = Vec::new();

        // Use ratatui's Paragraph::line_count for accurate wrapped height
        // instead of manual heuristic that over/under-estimates.
        let tmp_paragraph = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
        total_lines = tmp_paragraph.line_count(area.width) as u16;

        let max_scroll = total_lines.saturating_sub(area.height);
        let effective_offset = if app.user_scrolled_up {
            app.scroll_offset.min(max_scroll)
        } else {
            max_scroll
        };

        // Now compute screen y for each truncation indicator
        if !truncation_lines.is_empty() {
            let mut wr: u16 = 0;
            let mut ti = truncation_lines.iter().peekable();
            for (logical_idx, line) in lines.iter().enumerate() {
                if let Some(&&(trunc_logical, msg_idx)) = ti.peek()
                    && trunc_logical == logical_idx
                {
                    let screen_y = area.y as i32 + wr as i32 - effective_offset as i32;
                    if screen_y >= area.y as i32 && screen_y < (area.y + area.height) as i32 {
                        zones.push((screen_y as u16, msg_idx));
                    }
                    ti.next();
                    if ti.peek().is_none() {
                        break;
                    }
                }
                let w = line.width().max(1) as u16;
                wr += w.div_ceil(area.width);
            }
        }
        *app.truncation_click_zones.borrow_mut() = zones;

        // Compute screen y for each diff toggle indicator (same two-pass approach)
        let mut toggle_zones: Vec<(u16, usize)> = Vec::new();
        if !tool_toggle_lines.is_empty() {
            let mut wr: u16 = 0;
            let mut ti = tool_toggle_lines.iter().peekable();
            for (logical_idx, line) in lines.iter().enumerate() {
                if let Some(&&(toggle_logical, msg_idx)) = ti.peek()
                    && toggle_logical == logical_idx
                {
                    let screen_y = area.y as i32 + wr as i32 - effective_offset as i32;
                    if screen_y >= area.y as i32 && screen_y < (area.y + area.height) as i32 {
                        toggle_zones.push((screen_y as u16, msg_idx));
                    }
                    ti.next();
                    if ti.peek().is_none() {
                        break;
                    }
                }
                let w = line.width().max(1) as u16;
                wr += w.div_ceil(area.width);
            }
        }
        *app.tool_toggle_rects.borrow_mut() = toggle_zones;

        // Compute screen y for each thinking block toggle indicator
        {
            let mut thinking_zones: Vec<(u16, usize)> = Vec::new();
            if !thinking_lines.is_empty() {
                let mut wr: u16 = 0;
                let mut ti = thinking_lines.iter().peekable();
                for (logical_idx, line) in lines.iter().enumerate() {
                    if let Some(&&(think_logical, msg_idx)) = ti.peek()
                        && think_logical == logical_idx
                    {
                        let screen_y = area.y as i32 + wr as i32 - effective_offset as i32;
                        if screen_y >= area.y as i32 && screen_y < (area.y + area.height) as i32 {
                            thinking_zones.push((screen_y as u16, msg_idx));
                        }
                        ti.next();
                        if ti.peek().is_none() {
                            break;
                        }
                    }
                    let w = line.width().max(1) as u16;
                    wr += w.div_ceil(area.width);
                }
            }
            *app.thinking_click_zones.borrow_mut() = thinking_zones;
        }
    }

    let max_scroll = total_lines.saturating_sub(area.height);

    // Cache for keybinding math
    app.total_chat_lines.set(total_lines);
    app.chat_area_height.set(area.height);

    // Compute effective scroll offset
    let effective_offset = if app.user_scrolled_up {
        app.scroll_offset.min(max_scroll)
    } else {
        max_scroll
    };

    // Store plain text for text selection extraction
    let plain_lines: Vec<String> = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    *app.rendered_chat_text.borrow_mut() = plain_lines;

    // Build paragraph, apply scroll, and render
    let chat = Paragraph::new(lines)
        .style(Style::default().bg(colors.bg_primary))
        .wrap(Wrap { trim: false })
        .scroll((effective_offset, 0));
    frame.render_widget(chat, area);

    // Selection highlighting overlay (inverted fg/bg)
    if let Some(ref sel) = app.text_selection {
        let (start_row, start_col, end_row, end_col) =
            if (sel.anchor_row, sel.anchor_col) <= (sel.end_row, sel.end_col) {
                (sel.anchor_row, sel.anchor_col, sel.end_row, sel.end_col)
            } else {
                (sel.end_row, sel.end_col, sel.anchor_row, sel.anchor_col)
            };

        for row in start_row..=end_row {
            if row < area.y || row >= area.y + area.height {
                continue;
            }
            let col_start = if row == start_row { start_col } else { area.x };
            let col_end = if row == end_row {
                end_col
            } else {
                area.x + area.width - 1
            };

            for col in col_start..=col_end {
                if col < area.x || col >= area.x + area.width {
                    continue;
                }
                if let Some(cell) = frame
                    .buffer_mut()
                    .cell_mut(ratatui::prelude::Position { x: col, y: row })
                {
                    std::mem::swap(&mut cell.fg, &mut cell.bg);
                }
            }
        }
    }

    // "New messages" indicator when scrolled up
    if app.user_scrolled_up && effective_offset < max_scroll {
        let indicator = Line::from(Span::styled(
            " \u{2193} New messages ",
            Style::default().fg(colors.bg_primary).bg(colors.info),
        ));
        let indicator_area = Rect {
            x: area.x + area.width.saturating_sub(16),
            y: area.y + area.height.saturating_sub(1),
            width: 16.min(area.width),
            height: 1,
        };
        frame.render_widget(Paragraph::new(indicator), indicator_area);
    }
}

/// Render the input area (with optional queued messages box above).
fn render_input(frame: &mut Frame, area: Rect, app: &State, colors: &theme::Colors) {
    use crate::tui::input::{build_info_left, build_info_right, render_input_field};

    // Split area: queued messages box on top, approval bar, input field below
    let queue_count = app.message_queue.len();
    let queue_box_height = if queue_count > 0 {
        queue_count as u16 + 2
    } else {
        0
    };

    let has_approval = matches!(app.agent.state, AgentState::PendingApproval { .. })
        || app.sub_agent_approval_showing.is_some();
    let approval_height = if has_approval {
        crate::tui::approval::APPROVAL_BAR_HEIGHT
    } else {
        0u16
    };

    let has_attachments = !app.attachments.is_empty();
    let attach_height: u16 = if has_attachments { 1 } else { 0 };

    let extra_height = queue_box_height + approval_height + attach_height;
    let (queue_area, approval_area, attach_area, input_area) =
        if extra_height > 0 && area.height > extra_height {
            let mut constraints = Vec::new();
            if queue_box_height > 0 {
                constraints.push(Constraint::Length(queue_box_height));
            }
            if approval_height > 0 {
                constraints.push(Constraint::Length(approval_height));
            }
            if attach_height > 0 {
                constraints.push(Constraint::Length(attach_height));
            }
            constraints.push(Constraint::Min(1));
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(area);

            let mut idx = 0;
            let q = if queue_box_height > 0 {
                idx += 1;
                Some(chunks[idx - 1])
            } else {
                None
            };
            let a = if approval_height > 0 {
                idx += 1;
                Some(chunks[idx - 1])
            } else {
                None
            };
            let att = if attach_height > 0 {
                idx += 1;
                Some(chunks[idx - 1])
            } else {
                None
            };
            let i = chunks[idx];
            (q, a, att, i)
        } else {
            (None, None, None, area)
        };

    // Render queued messages box
    if let Some(q_area) = queue_area {
        let queue_lines: Vec<Line> = app
            .message_queue
            .iter()
            .map(|msg| {
                let display = if msg.len() > q_area.width.saturating_sub(4) as usize {
                    format!("{}…", &msg[..q_area.width.saturating_sub(5) as usize])
                } else {
                    msg.clone()
                };
                Line::from(Span::styled(display, Style::default().fg(colors.text_dim)))
            })
            .collect();

        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(colors.border))
            .title(Span::styled(
                format!(" {queue_count}/3 queued "),
                Style::default().fg(colors.text_dim),
            ));

        let paragraph = Paragraph::new(queue_lines).block(block);
        frame.render_widget(paragraph, q_area);
    }

    // Render approval bar
    if let Some(a_area) = approval_area
        && let Some((name, args)) = app.agent.current_approval_prompt()
    {
        let has_diff = app
            .chat_messages
            .iter()
            .rev()
            .find_map(|m| {
                if let ChatMessage::Tool(t) = m
                    && t.status == ToolStatus::Pending
                {
                    return Some(t.diff_preview.is_some());
                }
                None
            })
            .unwrap_or(false);
        crate::tui::approval::render(frame, a_area, name, args, has_diff);
    } else if let Some(a_area) = approval_area
        && let Some((agent_id, ref tool_name, ref arguments)) = app.sub_agent_approval_showing
    {
        let task_label = app
            .sub_agents
            .iter()
            .find(|a| a.id == agent_id)
            .map(|a| a.task.as_str())
            .unwrap_or("subagent");
        let header = format!("agent '{task_label}' requests: {tool_name}");
        let args_val = serde_json::from_str::<serde_json::Value>(arguments)
            .unwrap_or(serde_json::Value::String(arguments.clone()));
        crate::tui::approval::render(frame, a_area, &header, &args_val, false);
    }

    // Render attachment chips
    if let Some(att_area) = attach_area {
        let chips: Vec<Span> = app
            .attachments
            .iter()
            .flat_map(|att| {
                vec![
                    Span::styled(
                        format!(" [image: {}] ", att.display_name),
                        Style::default().fg(colors.text).bg(colors.bg_elevated),
                    ),
                    Span::raw(" "),
                ]
            })
            .collect();
        let chip_line = Line::from(chips);
        frame.render_widget(Paragraph::new(chip_line), att_area);
    }

    let quit_confirm = app.quit_first_press.is_some();

    let is_roundhouse_awaiting = app
        .roundhouse_session
        .as_ref()
        .map(|s| s.phase == crate::roundhouse::types::RoundhousePhase::AwaitingPrompt)
        .unwrap_or(false);

    let agent_label: Option<&str> = if is_roundhouse_awaiting {
        Some("Roundhouse \u{203a} Enter your planning prompt")
    } else {
        None
    };

    let (mut accent_color, info_left) = build_info_left(
        agent_label,
        quit_confirm,
        app.mode,
        &app.active_model_name,
        &app.active_provider_name,
        colors,
    );

    // PendingApproval uses brand accent (not warning)
    if matches!(app.agent.state, AgentState::PendingApproval { .. }) {
        accent_color = colors.brand;
    }

    let info_right = build_info_right(colors);

    render_input_field(
        frame,
        input_area,
        &app.input,
        accent_color,
        info_left,
        info_right,
        colors,
    );

    // Slash autocomplete dropdown (renders above input)
    if let Some(auto) = &app.slash_auto {
        crate::tui::slash_auto::render_slash_autocomplete(
            frame,
            input_area,
            auto,
            &app.input.content(),
            &app.commands,
            &app.skills,
            colors,
            true,
            app.current_session_id.as_deref(),
            &app.discovered_locals,
        );
    }

    // File autocomplete dropdown (renders above input, attached to input border)
    if let Some(ref auto) = app.file_auto {
        let visible = auto.matches.len().min(8);
        if visible > 0 {
            let dropdown_height = visible as u16 + 2; // +2 for top/bottom border
            // Position so bottom border overlaps input top border (connected look)
            let dropdown_area = Rect {
                x: input_area.x,
                y: input_area.y.saturating_sub(dropdown_height - 1),
                width: input_area.width.min(60),
                height: dropdown_height,
            };

            let items: Vec<Line> = auto
                .matches
                .iter()
                .enumerate()
                .take(visible)
                .map(|(i, path)| {
                    let style = if i == auto.selected {
                        Style::default().fg(colors.text).bg(colors.bg_hover)
                    } else {
                        Style::default()
                            .fg(colors.text_secondary)
                            .bg(colors.bg_elevated)
                    };
                    Line::from(Span::styled(format!(" {path} "), style))
                })
                .collect();

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(Style::default().fg(colors.border_active))
                .title(" files ")
                .title_style(Style::default().fg(colors.text_dim))
                .style(Style::default().bg(colors.bg_elevated));
            let paragraph = Paragraph::new(items).block(block);
            frame.render_widget(ratatui::widgets::Clear, dropdown_area);
            frame.render_widget(paragraph, dropdown_area);
        }
    }
}

/// Render the paste confirmation dialog overlay.
fn render_paste_confirm(
    frame: &mut Frame,
    line_count: usize,
    char_count: usize,
    colors: &theme::Colors,
) {
    let area = frame.area();
    let width: u16 = 50;
    let height: u16 = 5;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" Paste Confirmation ")
        .border_style(Style::default().fg(colors.warning));

    let text = format!(
        "Paste {} lines ({} chars)?\n\n[y]es / [Enter]    [n]o / [Esc]",
        line_count, char_count
    );

    let paragraph = Paragraph::new(text)
        .block(block)
        .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
    frame.render_widget(ratatui::widgets::Clear, dialog_area);
    frame.render_widget(paragraph, dialog_area);
}

/// Render the Roundhouse provider picker dialog.
fn render_roundhouse_picker(
    frame: &mut Frame,
    picker: &crate::tui::dialog::RoundhousePickerState,
    app: &State,
    colors: &theme::Colors,
) {
    let area = frame.area();
    // 2 border + 1 primary + 1 blank + max(1, N secondaries) + 1 blank + 1 footer = max(1,N) + 6
    let list_rows = picker.secondaries.len().max(1) as u16;
    let content_lines = list_rows + 6;
    let width: u16 = 55;
    let height: u16 = content_lines.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" Roundhouse \u{2014} Add Models ")
        .border_style(Style::default().fg(colors.brand));

    let primary_label = if let Some(session) = &app.roundhouse_session {
        format!(
            "Primary: {}/{}",
            session.primary_provider, session.primary_model
        )
    } else {
        format!(
            "Primary: {}/{}",
            app.active_provider_name, app.active_model_name
        )
    };

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            primary_label,
            Style::default().fg(colors.text_secondary),
        )),
        Line::from(""),
    ];

    if picker.secondaries.is_empty() {
        lines.push(Line::from(Span::styled(
            "No secondaries added yet. Press 'a' to add.",
            Style::default().fg(colors.text_dim),
        )));
    } else {
        for (i, sec) in picker.secondaries.iter().enumerate() {
            let prefix = if i == picker.selected { "▸ " } else { "  " };
            let label = format!("{prefix}{}. {}/{}", i + 1, sec.display_name, sec.model);
            let style = if i == picker.selected {
                Style::default().fg(colors.brand).bg(colors.bg_hover)
            } else {
                Style::default().fg(colors.text)
            };
            lines.push(Line::from(Span::styled(label, style)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "a: add model | d: remove | Enter: start | Esc: cancel",
        Style::default().fg(colors.text_dim),
    )));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
    frame.render_widget(ratatui::widgets::Clear, dialog_area);
    frame.render_widget(paragraph, dialog_area);
}

fn render_circuits_list(
    frame: &mut Frame,
    list_state: &crate::tui::dialog::CircuitsListState,
    app: &State,
    colors: &theme::Colors,
) {
    let area = frame.area();
    let circuits = &app.circuit_manager.circuits;

    // Height: 2 border + 1 blank + max(1, N circuits) + 1 blank + 1 footer = N + 5 (min 6)
    let content_rows = circuits.len().max(1) as u16;
    let height: u16 = (content_rows + 5).min(area.height);
    let width: u16 = 72_u16.min(area.width);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .title(" Circuits ")
        .border_style(Style::default().fg(colors.brand));

    let mut lines: Vec<Line> = Vec::new();

    if circuits.is_empty() {
        lines.push(Line::from(Span::styled(
            "No active circuits. Use /circuit <interval> \"<prompt>\" to create one.",
            Style::default().fg(colors.text_secondary),
        )));
    } else {
        for (i, handle) in circuits.iter().enumerate() {
            let c = &handle.circuit;

            // Truncate prompt to fit
            let max_prompt = 28_usize;
            let prompt = if c.prompt.len() > max_prompt {
                format!("\"{}…\"", &c.prompt[..max_prompt.saturating_sub(1)])
            } else {
                format!("\"{}\"", c.prompt)
            };

            // Format interval
            let interval = if c.interval_secs >= 3600 {
                format!("{}h", c.interval_secs / 3600)
            } else if c.interval_secs >= 60 {
                format!("{}m", c.interval_secs / 60)
            } else {
                format!("{}s", c.interval_secs)
            };

            let status = match &c.status {
                crate::circuits::types::CircuitStatus::Active => "active",
                crate::circuits::types::CircuitStatus::Paused => "paused",
                crate::circuits::types::CircuitStatus::Error(_) => "error",
            };

            let runs = if c.run_count == 1 {
                "1 run".to_string()
            } else {
                format!("{} runs", c.run_count)
            };

            let row = format!(
                "  \u{25cf} {:<32} {:>4}   {:<8} {}",
                prompt, interval, status, runs
            );

            let style = if i == list_state.selected {
                Style::default().fg(colors.text).bg(colors.bg_hover)
            } else {
                Style::default().fg(colors.text)
            };
            lines.push(Line::from(Span::styled(row, style)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "d/Del: delete | \u{2191}\u{2193}: navigate | Esc: close",
        Style::default().fg(colors.text_dim),
    )));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
    frame.render_widget(ratatui::widgets::Clear, dialog_area);
    frame.render_widget(paragraph, dialog_area);
}

/// Render the local provider connect dialog.
fn render_local_connect(
    frame: &mut Frame,
    state: &crate::tui::dialog::LocalProviderConnectState,
    colors: &theme::Colors,
) {
    use crate::tui::dialog::LocalConnectPhase;

    let area = frame.area();

    match &state.phase {
        LocalConnectPhase::Address => {
            let width: u16 = 55;
            let height: u16 = if state.error.is_some() { 8 } else { 7 };
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            let y = area.y + (area.height.saturating_sub(height)) / 2;
            let dialog_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(format!(" Connect {} ", state.provider_name))
                .border_style(Style::default().fg(colors.brand));

            let mut lines: Vec<Line> = vec![
                Line::from(Span::styled(
                    "Server address:",
                    Style::default().fg(colors.text_secondary),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        format!(
                            " {} ",
                            if state.address.is_empty() {
                                " "
                            } else {
                                &state.address
                            }
                        ),
                        Style::default().fg(colors.text).bg(colors.bg_hover),
                    ),
                    Span::styled("\u{2588}", Style::default().fg(colors.brand)),
                ]),
            ];

            if let Some(err) = &state.error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    err.as_str(),
                    Style::default().fg(colors.error),
                )));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter: connect  |  Esc: cancel",
                Style::default().fg(colors.text_dim),
            )));

            let paragraph = Paragraph::new(lines)
                .block(block)
                .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
            frame.render_widget(ratatui::widgets::Clear, dialog_area);
            frame.render_widget(paragraph, dialog_area);
        }
        LocalConnectPhase::Probing => {
            let width: u16 = 45;
            let height: u16 = 5;
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            let y = area.y + (area.height.saturating_sub(height)) / 2;
            let dialog_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(format!(" Connect {} ", state.provider_name))
                .border_style(Style::default().fg(colors.brand));

            let text = format!("Connecting to {}...\n\nEsc: cancel", state.address);

            let paragraph = Paragraph::new(text)
                .block(block)
                .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
            frame.render_widget(ratatui::widgets::Clear, dialog_area);
            frame.render_widget(paragraph, dialog_area);
        }
        LocalConnectPhase::ModelSelect => {
            let visible_count = state.models.len().min(12);
            let width: u16 = 55;
            let height: u16 = visible_count as u16 + 4; // border + title line + footer
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            let y = area.y + (area.height.saturating_sub(height)) / 2;
            let dialog_area = Rect::new(x, y, width.min(area.width), height.min(area.height));

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(" Select Model ")
                .border_style(Style::default().fg(colors.brand));

            // Scroll window for long lists
            let scroll_start = if state.selected_model >= visible_count {
                state.selected_model - visible_count + 1
            } else {
                0
            };

            let mut lines: Vec<Line> = state
                .models
                .iter()
                .enumerate()
                .skip(scroll_start)
                .take(visible_count)
                .map(|(i, model)| {
                    let style = if i == state.selected_model {
                        Style::default().fg(colors.text).bg(colors.bg_hover)
                    } else {
                        Style::default().fg(colors.text_secondary)
                    };
                    let prefix = if i == state.selected_model {
                        "> "
                    } else {
                        "  "
                    };
                    Line::from(Span::styled(format!("{prefix}{model}"), style))
                })
                .collect();

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter: select  |  Esc: back",
                Style::default().fg(colors.text_dim),
            )));

            let paragraph = Paragraph::new(lines)
                .block(block)
                .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
            frame.render_widget(ratatui::widgets::Clear, dialog_area);
            frame.render_widget(paragraph, dialog_area);
        }
    }
}

fn render_migration_checklist(
    frame: &mut Frame,
    checklist: &crate::tui::dialog::MigrationChecklistState,
    colors: &theme::Colors,
) {
    use crate::tui::dialog::{MigrationItemKind, MigrationPhase};

    let area = frame.area();
    let title = format!(" Migrate from {} ", checklist.platform.label());

    match &checklist.phase {
        MigrationPhase::Checklist => {
            let content_lines = checklist.items.len() as u16 + 5;
            let width: u16 = 65_u16.min(area.width);
            let height: u16 = content_lines.min(area.height);
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            let y = area.y + (area.height.saturating_sub(height)) / 2;
            let dialog_area = Rect::new(x, y, width, height);

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(colors.brand));

            let mut lines: Vec<Line> = Vec::new();
            for (i, item) in checklist.items.iter().enumerate() {
                let checkbox = if item.toggled { "[x] " } else { "[ ] " };
                let label = format!("{}{}: {}", checkbox, item.label, item.description);
                let style = if i == checklist.selected {
                    Style::default().fg(colors.text).bg(colors.bg_hover)
                } else {
                    Style::default().fg(colors.text)
                };
                lines.push(Line::from(Span::styled(label, style)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Space: toggle | Enter: preview | Esc: cancel",
                Style::default().fg(colors.text_dim),
            )));

            let paragraph = Paragraph::new(lines)
                .block(block)
                .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
            frame.render_widget(ratatui::widgets::Clear, dialog_area);
            frame.render_widget(paragraph, dialog_area);
        }
        MigrationPhase::Preview => {
            let mcp_count = checklist
                .items
                .iter()
                .filter(|i| i.toggled && matches!(&i.kind, MigrationItemKind::McpServer { .. }))
                .count();
            let prompt_count = checklist
                .items
                .iter()
                .filter(|i| i.toggled && matches!(&i.kind, MigrationItemKind::SystemPrompt(_)))
                .count();
            let claude_md_count = checklist
                .items
                .iter()
                .filter(|i| i.toggled && matches!(&i.kind, MigrationItemKind::ClaudeMd(_)))
                .count();

            let mut preview_lines: Vec<String> = vec!["Will apply:".to_string(), String::new()];
            if mcp_count > 0 {
                preview_lines.push(format!("  + {} MCP server(s) to config", mcp_count));
            }
            if prompt_count > 0 {
                preview_lines.push("  + System prompt to CABOOSE.md".to_string());
            }
            if claude_md_count > 0 {
                preview_lines.push("  + CLAUDE.md content to CABOOSE.md".to_string());
            }

            let height: u16 = (preview_lines.len() as u16 + 5).min(area.height);
            let width: u16 = 55_u16.min(area.width);
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            let y = area.y + (area.height.saturating_sub(height)) / 2;
            let dialog_area = Rect::new(x, y, width, height);

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(colors.brand));

            let mut lines: Vec<Line> = preview_lines
                .iter()
                .map(|s| Line::from(s.as_str()))
                .collect();
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Enter: apply | Esc: back",
                Style::default().fg(colors.text_dim),
            )));

            let paragraph = Paragraph::new(lines)
                .block(block)
                .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
            frame.render_widget(ratatui::widgets::Clear, dialog_area);
            frame.render_widget(paragraph, dialog_area);
        }
        MigrationPhase::Done(summary) => {
            let height: u16 = 5_u16.min(area.height);
            let width: u16 = 55_u16.min(area.width);
            let x = area.x + (area.width.saturating_sub(width)) / 2;
            let y = area.y + (area.height.saturating_sub(height)) / 2;
            let dialog_area = Rect::new(x, y, width, height);

            let block = Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(" Migration Complete ")
                .border_style(Style::default().fg(colors.brand));

            let text = format!("{}\n\nPress any key to close", summary);
            let paragraph = Paragraph::new(text)
                .block(block)
                .style(Style::default().fg(colors.text).bg(colors.bg_elevated));
            frame.render_widget(ratatui::widgets::Clear, dialog_area);
            frame.render_widget(paragraph, dialog_area);
        }
        MigrationPhase::Applying => {
            // Brief spinner-like state (instant for now)
        }
    }
}

fn render_agent_stream_overlay(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    overlay_state: &crate::tui::dialog::AgentStreamOverlayState,
    state: &State,
    colors: &theme::Colors,
) {
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Paragraph};

    // Semi-transparent backdrop
    let backdrop = Block::default().style(Style::default().bg(ratatui::style::Color::Rgb(0, 0, 0)));
    frame.render_widget(backdrop, area);

    // Window: centered, 88% width
    let win_width = (area.width as f32 * 0.88) as u16;
    let win_height = area.height.saturating_sub(4);
    let x = (area.width.saturating_sub(win_width)) / 2;
    let y = (area.height.saturating_sub(win_height)) / 2;
    let win_area = ratatui::layout::Rect {
        x: area.x + x,
        y: area.y + y,
        width: win_width,
        height: win_height,
    };

    let Some(idx) = state.agent_stream_overlay else {
        return;
    };
    let Some(agent) = state.sub_agents.get(idx) else {
        return;
    };
    let agent_count = state.sub_agents.len();

    // Title
    let (dot, _) = crate::tui::sidebar::agent_status_display(&agent.state);
    let elapsed = crate::sub_agent::format_elapsed(agent.elapsed_secs());
    let title = format!(" {dot} {}   {} · {}", agent.task, elapsed, agent.branch);
    let nav_hint = if agent_count > 1 {
        "  \u{2191}\u{2193} scroll  tab next  esc close "
    } else {
        "  \u{2191}\u{2193} scroll  esc close "
    };

    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(colors.border))
        .title(Span::styled(title, Style::default().fg(colors.text)))
        .title_bottom(Span::styled(nav_hint, Style::default().fg(colors.text_dim)));
    frame.render_widget(block.clone(), win_area);
    let inner = block.inner(win_area);

    // Body area and footer area
    let body_height = inner.height.saturating_sub(1);
    let body_area = ratatui::layout::Rect {
        height: body_height,
        ..inner
    };
    let footer_area = ratatui::layout::Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };

    // Stream lines
    let stream_lines: Vec<Line> = agent
        .stream
        .iter()
        .map(|line| {
            let (style, prefix) = match line.kind {
                crate::sub_agent::StreamLineKind::Thinking => (
                    Style::default()
                        .fg(colors.text_dim)
                        .add_modifier(Modifier::ITALIC),
                    "",
                ),
                crate::sub_agent::StreamLineKind::ToolCall => {
                    (Style::default().fg(colors.info), "")
                }
                crate::sub_agent::StreamLineKind::ToolResult => {
                    (Style::default().fg(colors.success), "\u{2192} ")
                }
                crate::sub_agent::StreamLineKind::Text => (Style::default().fg(colors.text), ""),
                crate::sub_agent::StreamLineKind::Error => (Style::default().fg(colors.error), ""),
            };
            Line::from(Span::styled(format!("{prefix}{}", line.text), style))
        })
        .collect();

    let scroll = if overlay_state.follow {
        stream_lines.len().saturating_sub(body_height as usize) as u16
    } else {
        overlay_state.scroll_offset as u16
    };

    let body = Paragraph::new(stream_lines).scroll((scroll, 0));
    frame.render_widget(body, body_area);

    // Footer
    let footer_text = format!(
        " worktree: {}    ${:.3}",
        agent.worktree_path.display(),
        agent.cost_usd
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            footer_text,
            Style::default().fg(colors.text_dim),
        )),
        footer_area,
    );
}
