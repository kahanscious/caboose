//! Sidebar panel — context usage, files modified, MCP servers, tasks.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{FileStats, TaskOutline, TaskStatus};
use crate::mcp::ServerStatus;
use crate::sub_agent::{SubAgent, SubAgentState, format_elapsed};
use crate::tui::theme::Colors;

/// Per-state counts for the agents section header pills.
pub struct AgentCounts {
    pub running: usize,
    pub pending: usize,
    pub failed: usize,
}

/// Return the status dot character and a color key for a subagent state.
/// Color keys: "run" = info (blue), "done" = success (green), "fail" = error (red), "dim" = dim
pub fn agent_status_display(state: &SubAgentState) -> (&'static str, &'static str) {
    match state {
        SubAgentState::Running => ("\u{25CF}", "run"), // ●
        SubAgentState::Pending => ("\u{25CB}", "dim"), // ○
        SubAgentState::WaitingApproval { .. } => ("\u{25CF}", "warn"), // ● amber
        SubAgentState::Done => ("\u{2713}", "done"),   // ✓
        SubAgentState::Failed { .. } => ("\u{2717}", "fail"), // ✗
        SubAgentState::Conflict { .. } => ("\u{2717}", "fail"), // ✗
    }
}

/// Count running, pending, and failed agents.
pub fn agent_counts(agents: &[SubAgent]) -> AgentCounts {
    AgentCounts {
        running: agents
            .iter()
            .filter(|a| matches!(a.state, SubAgentState::Running))
            .count(),
        pending: agents
            .iter()
            .filter(|a| matches!(a.state, SubAgentState::Pending))
            .count(),
        failed: agents
            .iter()
            .filter(|a| {
                matches!(
                    a.state,
                    SubAgentState::Failed { .. } | SubAgentState::Conflict { .. }
                )
            })
            .count(),
    }
}

/// Render the agents section into `lines`.
/// Returns the line offset of the dismiss button within `lines` (if shown).
pub fn render_agents_section(
    lines: &mut Vec<Line<'static>>,
    agents: &[SubAgent],
    sidebar_width: u16,
    colors: &Colors,
    tick: u64,
) -> Option<usize> {
    let counts = agent_counts(agents);
    let mut dismiss_line_offset: Option<usize> = None;

    // Header: "  agents" + pills
    let mut header_spans: Vec<Span<'static>> = vec![Span::styled(
        "  agents",
        Style::default().fg(colors.text_secondary).bold(),
    )];
    if counts.running > 0 {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            format!("\u{25CF} {} running", counts.running),
            Style::default().fg(colors.info),
        ));
    }
    if counts.pending > 0 {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            format!("\u{25CB} {} pending", counts.pending),
            Style::default().fg(colors.text_dim),
        ));
    }
    if counts.failed > 0 {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            format!("\u{2717} {} failed", counts.failed),
            Style::default().fg(colors.error),
        ));
    }
    // Clickable dismiss when all agents are in terminal state
    if counts.running == 0 && counts.pending == 0 && !agents.is_empty() {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(
            "\u{25B8} clear",
            Style::default().fg(colors.text_muted),
        ));
        dismiss_line_offset = Some(lines.len());
    }
    lines.push(Line::from(header_spans));
    lines.push(Line::from(""));

    // Per-agent rows
    // Layout: "  {dot} {task_name}  {right_status}"
    // Total width available minus: 2 indent + 1 dot + 1 space + 2 trailing spaces + right_status
    for agent in agents {
        let (dot_char, color_key) = agent_status_display(&agent.state);
        // Blink running/waiting dots in sync (● ↔ ○ every ~10 ticks)
        let dot: &str = if matches!(color_key, "run" | "warn") {
            if (tick / 10).is_multiple_of(2) {
                "\u{25CF}"
            } else {
                "\u{25CB}"
            }
        } else {
            dot_char
        };
        let dot_color = match color_key {
            "run" => colors.info,
            "done" => colors.success,
            "fail" => colors.error,
            "warn" => colors.warning,
            _ => colors.text_dim,
        };

        let right_status: String = match &agent.state {
            SubAgentState::Running => format_elapsed(agent.elapsed_secs()),
            SubAgentState::Pending => "pending".to_string(),
            SubAgentState::WaitingApproval { tool_name } => format!("waiting: {tool_name}"),
            SubAgentState::Done => "done".to_string(),
            SubAgentState::Failed { .. } => "failed".to_string(),
            SubAgentState::Conflict { .. } => "conflict".to_string(),
        };

        // Compute max task name width: width - 2 indent - 1 dot - 1 space - 2 gap - right_status
        let fixed = 2 + 1 + 1 + 2 + right_status.chars().count();
        let max_task = (sidebar_width as usize).saturating_sub(fixed);
        let task_chars: usize = agent.task.chars().count();
        let display_task: String = if task_chars > max_task && max_task > 1 {
            let truncated: String = agent
                .task
                .chars()
                .take(max_task.saturating_sub(1))
                .collect();
            format!("{truncated}\u{2026}")
        } else {
            agent.task.clone()
        };

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(dot, Style::default().fg(dot_color)),
            Span::raw(" "),
            Span::styled(display_task, Style::default().fg(colors.text)),
            Span::raw("  "),
            Span::styled(right_status, Style::default().fg(colors.text_muted)),
        ]));
    }

    lines.push(Line::from(""));

    dismiss_line_offset
}

/// Render the sidebar panel.
/// Returns the absolute screen row of the agents dismiss button, if one is shown.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    input_tokens: u32,
    output_tokens: u32,
    context_window: u32,
    turn_count: u32,
    tokens_per_sec: Option<f64>,
    mcp_servers: &[(String, ServerStatus, usize, bool)],
    model_id: &str,
    pricing: &crate::provider::pricing::PricingRegistry,
    modified_files: &std::collections::HashMap<String, FileStats>,
    task_outline: Option<&TaskOutline>,
    tick: u64,
    roundhouse_session: Option<&crate::roundhouse::RoundhouseSession>,
    active_watchers: &[crate::scm::watcher::Watcher],
    sub_agents: &[SubAgent],
) -> Option<u16> {
    let colors = Colors::default();

    // Sidebar block with left border only
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(colors.border))
        .style(Style::default().bg(colors.bg_primary));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // --- Context section ---
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Context",
        Style::default().fg(colors.text_secondary).bold(),
    )));
    lines.push(Line::from(""));

    // Progress bar
    let total_tokens = input_tokens + output_tokens;
    let bar_width = (inner.width as usize).saturating_sub(8);
    let pct = if context_window > 0 {
        (total_tokens as f64 / context_window as f64).min(1.0)
    } else {
        0.0
    };
    let filled = (pct * bar_width as f64) as usize;
    let empty = bar_width.saturating_sub(filled);
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {}", "\u{2588}".repeat(filled)),
            Style::default().fg(colors.brand),
        ),
        Span::styled(
            "\u{2591}".repeat(empty).to_string(),
            Style::default().fg(colors.text_muted),
        ),
        Span::styled(
            format!(" {:.0}%", pct * 100.0),
            Style::default().fg(colors.text_secondary),
        ),
    ]));

    // Token counts
    lines.push(Line::from(Span::styled(
        format!("  {total_tokens} / {context_window}"),
        Style::default().fg(colors.text_secondary),
    )));
    lines.push(Line::from(Span::styled(
        format!("  in:{input_tokens} out:{output_tokens}"),
        Style::default().fg(colors.text_secondary),
    )));

    // Cost estimate
    let cost_text = match pricing.estimate_cost(model_id, input_tokens, output_tokens) {
        Some(cost) => format!("  ${cost:.4}"),
        None => "  $--".to_string(),
    };
    lines.push(Line::from(Span::styled(
        cost_text,
        Style::default().fg(colors.text_secondary),
    )));

    // Per-million-token rates
    if let Some(p) = pricing.get(model_id) {
        lines.push(Line::from(Span::styled(
            format!(
                "  ${:.2}/M in · ${:.2}/M out",
                p.input_per_m, p.output_per_m
            ),
            Style::default().fg(colors.text_muted),
        )));
    }

    // Turn count
    lines.push(Line::from(Span::styled(
        format!("  Turn {turn_count}"),
        Style::default().fg(colors.text_secondary),
    )));

    // Tokens per second
    if let Some(tps) = tokens_per_sec {
        lines.push(Line::from(Span::styled(
            format!("  {tps:.1} tok/s"),
            Style::default().fg(colors.text_secondary),
        )));
    }

    // --- Separator ---
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  {}",
            "\u{2500}".repeat(inner.width.saturating_sub(4) as usize)
        ),
        Style::default().fg(colors.border),
    )));
    lines.push(Line::from(""));

    // --- Files Modified section ---
    render_files_modified(&mut lines, modified_files, inner.width, &colors);

    // --- MCP Servers section ---
    lines.push(Line::from(Span::styled(
        "  MCP Servers",
        Style::default().fg(colors.text_secondary).bold(),
    )));

    if mcp_servers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No servers",
            Style::default().fg(colors.text_dim),
        )));
    } else {
        for (name, status, _tool_count, _is_preset) in mcp_servers {
            let (dot, dot_color) = match status {
                ServerStatus::Connected => ("\u{25CF}", colors.success),
                ServerStatus::Connecting => ("\u{25CF}", colors.warning),
                ServerStatus::Error(_) => ("\u{25CF}", colors.error),
                ServerStatus::Disconnected => ("\u{25CB}", colors.text_dim),
            };

            let label = match status {
                ServerStatus::Connected => format!("{dot} {name}"),
                ServerStatus::Connecting => format!("{dot} {name}..."),
                ServerStatus::Error(_) => format!("{dot} {name} (error)"),
                ServerStatus::Disconnected => format!("{dot} {name}"),
            };

            lines.push(Line::from(Span::styled(
                format!("  {label}"),
                Style::default().fg(dot_color),
            )));
        }
    }

    // --- Roundhouse section ---
    if let Some(rh) = roundhouse_session {
        // Separator
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                "\u{2500}".repeat(inner.width.saturating_sub(4) as usize)
            ),
            Style::default().fg(colors.border),
        )));
        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            "  Roundhouse",
            Style::default().fg(colors.roundhouse).bold(),
        )));
        lines.push(Line::from(""));

        // Phase label with animated dots for active phases
        let phase_text = match rh.phase {
            crate::roundhouse::RoundhousePhase::SelectingProviders => {
                "Phase: selecting providers".to_string()
            }
            crate::roundhouse::RoundhousePhase::AwaitingPrompt => {
                "Phase: awaiting prompt".to_string()
            }
            crate::roundhouse::RoundhousePhase::Planning => {
                let dot_count = ((tick / 3) % 4) as usize;
                let dots = ".".repeat(dot_count);
                format!("Phase: planning{dots}")
            }
            crate::roundhouse::RoundhousePhase::Synthesizing => {
                let dot_count = ((tick / 3) % 4) as usize;
                let dots = ".".repeat(dot_count);
                format!("Phase: synthesizing{dots}")
            }
            crate::roundhouse::RoundhousePhase::Reviewing => "Phase: reviewing".to_string(),
            crate::roundhouse::RoundhousePhase::Executing => {
                let dot_count = ((tick / 3) % 4) as usize;
                let dots = ".".repeat(dot_count);
                format!("Phase: executing{dots}")
            }
            crate::roundhouse::RoundhousePhase::Complete => "Phase: complete".to_string(),
            crate::roundhouse::RoundhousePhase::Cancelled => "Phase: cancelled".to_string(),
        };
        lines.push(Line::from(Span::styled(
            format!("  {phase_text}"),
            Style::default().fg(colors.text_secondary),
        )));

        match rh.phase {
            crate::roundhouse::RoundhousePhase::Reviewing => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  Plan ready for review",
                    Style::default().fg(colors.success),
                )));
                if let Some(ref path) = rh.plan_file {
                    // Show relative path if possible
                    let display = std::env::current_dir()
                        .ok()
                        .and_then(|cwd| path.strip_prefix(&cwd).ok().map(|p| p.to_path_buf()))
                        .unwrap_or_else(|| path.clone());
                    lines.push(Line::from(Span::styled(
                        format!("  {}", display.display()),
                        Style::default().fg(colors.text_muted),
                    )));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  /roundhouse clear",
                    Style::default().fg(colors.text_dim),
                )));
            }
            crate::roundhouse::RoundhousePhase::Executing => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  Executing plan...",
                    Style::default().fg(colors.warning),
                )));
            }
            crate::roundhouse::RoundhousePhase::Complete => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  Plan executed",
                    Style::default().fg(colors.success),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  /roundhouse clear",
                    Style::default().fg(colors.text_dim),
                )));
            }
            _ => {
                // Show per-LLM status rows
                lines.push(Line::from(""));

                // Primary planner
                let (primary_icon, primary_status_text, primary_color) =
                    planner_status_parts(&rh.primary_status, &colors, tick, rh.primary_status_tick);
                let primary_name = truncate_provider_name(&rh.primary_provider, 10);
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{primary_icon} "),
                        Style::default().fg(primary_color),
                    ),
                    Span::styled(
                        format!("{primary_name:<10} "),
                        Style::default().fg(colors.text_secondary),
                    ),
                    Span::styled(primary_status_text, Style::default().fg(primary_color)),
                ]));

                // Secondary planners
                for secondary in &rh.secondaries {
                    let (icon, status_text, color) = planner_status_parts(
                        &secondary.status,
                        &colors,
                        tick,
                        secondary.status_tick,
                    );
                    let name = truncate_provider_name(&secondary.provider_name, 10);
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(format!("{icon} "), Style::default().fg(color)),
                        Span::styled(
                            format!("{name:<10} "),
                            Style::default().fg(colors.text_secondary),
                        ),
                        Span::styled(status_text, Style::default().fg(color)),
                    ]));
                }

                // Total cost
                if rh.total_cost > 0.0 {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        format!("  Total: ${:.4}", rh.total_cost),
                        Style::default().fg(colors.text_muted),
                    )));
                }
            }
        }
    }

    // --- Watchers section ---
    if !active_watchers.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Watchers",
            Style::default().fg(colors.text_secondary).bold(),
        )));
        lines.push(Line::from(""));
        for w in active_watchers {
            let icon = w.last_status.icon();
            lines.push(Line::from(Span::styled(
                format!("  {} PR #{}", icon, w.pr_number),
                Style::default().fg(colors.text_secondary),
            )));
        }
    }

    // --- Agents section (only when agents exist) ---
    let mut dismiss_row: Option<u16> = None;
    if !sub_agents.is_empty() {
        // Separator before agents section
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                "\u{2500}".repeat(inner.width.saturating_sub(4) as usize)
            ),
            Style::default().fg(colors.border),
        )));
        lines.push(Line::from(""));
        if let Some(offset) =
            render_agents_section(&mut lines, sub_agents, inner.width, &colors, tick)
        {
            dismiss_row = Some(inner.y + offset as u16);
        }
    }

    // Render fixed sections
    let fixed_height = lines.len() as u16;
    let fixed_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: fixed_height.min(inner.height),
    };
    frame.render_widget(Paragraph::new(lines), fixed_area);

    // --- Tasks section (scrollable, takes remaining space) ---
    if let Some(outline) = task_outline
        && !outline.tasks.is_empty()
    {
        let tasks_y = inner.y + fixed_height;
        let tasks_height = inner.height.saturating_sub(fixed_height);
        if tasks_height >= 3 {
            let tasks_area = Rect {
                x: inner.x,
                y: tasks_y,
                width: inner.width,
                height: tasks_height,
            };
            render_tasks(frame, tasks_area, outline, tick, &colors);
        }
    }

    dismiss_row
}

/// Render the tasks section with auto-scroll.
///
/// Auto-scrolls so the most recently completed task is at the top of the
/// visible area, with remaining tasks visible below.
fn render_tasks(frame: &mut Frame, area: Rect, outline: &TaskOutline, tick: u64, colors: &Colors) {
    // Count completed tasks (exclude cancelled from denominator)
    let done = outline
        .tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .count();
    let active = outline
        .tasks
        .iter()
        .filter(|t| !matches!(t.status, TaskStatus::Cancelled))
        .count();

    let mut lines: Vec<Line> = Vec::new();

    // Separator
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  {}",
            "\u{2500}".repeat(area.width.saturating_sub(4) as usize)
        ),
        Style::default().fg(colors.border),
    )));
    lines.push(Line::from(""));

    // Title
    lines.push(Line::from(Span::styled(
        format!("  Tasks ({done}/{active})"),
        Style::default().fg(colors.text_secondary).bold(),
    )));

    // Task rows
    for task in &outline.tasks {
        let (icon, icon_color, label) = match task.status {
            TaskStatus::Completed => ("\u{2713}", colors.success, &task.content),
            TaskStatus::InProgress => {
                let icon = if (tick / 10).is_multiple_of(2) {
                    "\u{25cf}"
                } else {
                    "\u{25cb}"
                };
                (icon, colors.warning, &task.active_form)
            }
            TaskStatus::Pending => ("\u{25cb}", colors.text_dim, &task.content),
            TaskStatus::Cancelled => ("\u{2717}", colors.error, &task.content),
        };

        // Truncate label to fit sidebar width (2 indent + icon + space + label)
        let max_label = (area.width as usize).saturating_sub(5);
        let display_label = if label.len() > max_label {
            format!("{}…", &label[..max_label.saturating_sub(1)])
        } else {
            label.to_string()
        };

        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(icon, Style::default().fg(icon_color)),
            Span::raw(" "),
            Span::styled(
                display_label,
                Style::default().fg(
                    if matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled) {
                        colors.text_dim
                    } else {
                        colors.text
                    },
                ),
            ),
        ]));
    }

    let total_lines = lines.len() as u16;

    // Auto-scroll: find the most recently completed task and scroll so it's
    // near the top of the visible area. The separator + title take 4 lines.
    let header_lines: u16 = 4;
    let scroll_offset = if total_lines > area.height {
        // Find the index of the last completed task
        let last_completed_idx = outline
            .tasks
            .iter()
            .rposition(|t| t.status == TaskStatus::Completed);

        match last_completed_idx {
            Some(idx) => {
                // Task lines start after header_lines; scroll so this task is at top
                let task_line = header_lines + idx as u16;
                let max_scroll = total_lines.saturating_sub(area.height);
                task_line.min(max_scroll)
            }
            None => 0,
        }
    } else {
        0
    };

    let tasks_paragraph = Paragraph::new(lines).scroll((scroll_offset, 0));
    frame.render_widget(tasks_paragraph, area);
}

/// Return (icon, status_text, color) for a PlannerStatus.
///
/// Active statuses use a typewriter reveal effect driven by `tick`.
fn planner_status_parts(
    status: &crate::roundhouse::PlannerStatus,
    colors: &Colors,
    tick: u64,
    status_tick: u64,
) -> (&'static str, String, Color) {
    match status {
        crate::roundhouse::PlannerStatus::Pending => {
            ("\u{25CB}", "pending".to_string(), colors.text_dim)
        }
        crate::roundhouse::PlannerStatus::Thinking => (
            "\u{25CF}",
            typewriter("thinking", tick, status_tick),
            colors.roundhouse,
        ),
        crate::roundhouse::PlannerStatus::Streaming => (
            "\u{25CF}",
            typewriter("streaming", tick, status_tick),
            colors.roundhouse,
        ),
        crate::roundhouse::PlannerStatus::UsingTool(name) => (
            "\u{25CF}",
            typewriter(name, tick, status_tick),
            colors.warning,
        ),
        crate::roundhouse::PlannerStatus::Done => ("\u{2713}", "done".to_string(), colors.success),
        crate::roundhouse::PlannerStatus::Failed(_) => {
            ("\u{2717}", "failed".to_string(), colors.error)
        }
        crate::roundhouse::PlannerStatus::TimedOut => {
            ("\u{2717}", "timed out".to_string(), colors.error)
        }
    }
}

/// Looping typewriter effect: reveal `text` one character at a time, then
/// pause briefly, then restart. Loops continuously until the status changes.
fn typewriter(text: &str, tick: u64, status_tick: u64) -> String {
    let elapsed = tick.saturating_sub(status_tick);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if len == 0 {
        return String::new();
    }
    // Each cycle: reveal chars one per 2 ticks, then hold for 6 ticks
    let cycle_len = (len * 2 + 6) as u64;
    let pos = elapsed % cycle_len;
    let reveal = ((pos / 2) as usize).min(len);
    chars[..reveal].iter().collect()
}

/// Truncate or pad a provider name to at most `max` characters.
fn truncate_provider_name(name: &str, max: usize) -> String {
    let char_count = name.chars().count();
    if char_count <= max {
        name.to_string()
    } else {
        let truncated: String = name.chars().take(max.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Render the "Files Modified" sidebar section.
fn render_files_modified(
    lines: &mut Vec<Line>,
    modified_files: &std::collections::HashMap<String, FileStats>,
    sidebar_width: u16,
    colors: &Colors,
) {
    // Only show files that were actually written/edited (have additions or deletions)
    let mut write_files: Vec<(&String, &FileStats)> = modified_files
        .iter()
        .filter(|(_, stats)| stats.additions > 0 || stats.deletions > 0)
        .collect();

    lines.push(Line::from(Span::styled(
        "  Files Modified",
        Style::default().fg(colors.text_secondary).bold(),
    )));

    if write_files.is_empty() {
        lines.push(Line::from(Span::styled(
            "  None",
            Style::default().fg(colors.text_dim),
        )));
    } else {
        // Sort by path for stable display
        write_files.sort_by_key(|(path, _)| path.as_str());

        for (path, stats) in &write_files {
            // Show just the filename (or last 2 path components if space allows)
            let display_name = shorten_path(path, sidebar_width.saturating_sub(16) as usize);

            // Format: "  filename  +N -N"
            let added = format!("+{}", stats.additions);
            let removed = format!("-{}", stats.deletions);

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {display_name} "),
                    Style::default().fg(colors.text_secondary),
                ),
                Span::styled(added, Style::default().fg(colors.success)),
                Span::styled(" ", Style::default()),
                Span::styled(removed, Style::default().fg(colors.error)),
            ]));
        }

        // Summary line
        let total_added: usize = write_files.iter().map(|(_, s)| s.additions).sum();
        let total_removed: usize = write_files.iter().map(|(_, s)| s.deletions).sum();
        let file_count = write_files.len();
        lines.push(Line::from(Span::styled(
            format!("  {file_count} file(s) +{total_added} -{total_removed}"),
            Style::default().fg(colors.text_muted),
        )));
    }

    // Separator
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  {}",
            "\u{2500}".repeat(sidebar_width.saturating_sub(4) as usize)
        ),
        Style::default().fg(colors.border),
    )));
    lines.push(Line::from(""));
}

/// Shorten a file path to fit within `max_width` characters.
/// Keeps the filename and as much of the parent path as possible.
fn shorten_path(path: &str, max_width: usize) -> String {
    if path.len() <= max_width {
        return path.to_string();
    }

    // Try to show last two components (parent/file)
    let parts: Vec<&str> = path.rsplit('/').collect();
    let short = if parts.len() >= 2 {
        format!("{}/{}", parts[1], parts[0])
    } else {
        parts[0].to_string()
    };

    if short.len() <= max_width {
        return short;
    }

    // Just filename
    let filename = parts[0];
    if filename.len() <= max_width {
        return filename.to_string();
    }

    // Truncate filename
    format!("{}…", &filename[..max_width.saturating_sub(1)])
}

#[cfg(test)]
mod agents_section_tests {
    use super::*;
    use crate::sub_agent::{SubAgent, SubAgentState, format_elapsed};

    #[test]
    fn agent_status_dot_running() {
        let (dot, _color_key) = agent_status_display(&SubAgentState::Running);
        assert_eq!(dot, "●");
    }

    #[test]
    fn agent_status_dot_pending() {
        let (dot, _) = agent_status_display(&SubAgentState::Pending);
        assert_eq!(dot, "○");
    }

    #[test]
    fn agent_status_dot_done() {
        let (dot, _) = agent_status_display(&SubAgentState::Done);
        assert_eq!(dot, "✓");
    }

    #[test]
    fn agent_status_dot_failed() {
        let (dot, _) = agent_status_display(&SubAgentState::Failed {
            message: "oops".into(),
        });
        assert_eq!(dot, "✗");
    }

    #[test]
    fn agent_status_dot_conflict() {
        let (dot, _) = agent_status_display(&SubAgentState::Conflict {
            report: "conflict".into(),
        });
        assert_eq!(dot, "✗");
    }

    #[test]
    fn agents_section_counts() {
        let agents = vec![
            make_agent(SubAgentState::Running),
            make_agent(SubAgentState::Running),
            make_agent(SubAgentState::Pending),
            make_agent(SubAgentState::Done),
        ];
        let counts = agent_counts(&agents);
        assert_eq!(counts.running, 2);
        assert_eq!(counts.pending, 1);
        assert_eq!(counts.failed, 0);
    }

    fn make_agent(state: SubAgentState) -> SubAgent {
        let mut a = SubAgent::new("task".into(), "branch".into(), std::path::PathBuf::new());
        a.state = state;
        a
    }

    // Suppress unused import warning when format_elapsed is not directly called in tests
    #[allow(dead_code)]
    fn _use_format_elapsed() -> String {
        format_elapsed(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_path_short_path_unchanged() {
        assert_eq!(shorten_path("src/main.rs", 20), "src/main.rs");
    }

    #[test]
    fn shorten_path_long_path_shows_parent_and_file() {
        assert_eq!(
            shorten_path("very/long/nested/path/file.rs", 15),
            "path/file.rs"
        );
    }

    #[test]
    fn shorten_path_very_narrow_truncates() {
        assert_eq!(shorten_path("some/really_long_filename.rs", 5), "real…");
    }
}
