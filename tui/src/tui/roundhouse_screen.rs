//! Dedicated Roundhouse screen — model viewer + navigator + gate bar.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app::State;
use crate::roundhouse::types::{PlannerStatus, RoundhousePhase};
use crate::tui::theme;

/// Render the full Roundhouse screen.
pub fn render(frame: &mut Frame, state: &State) {
    let colors = theme::Colors::default();
    let area = frame.area();

    let session = match state.roundhouse_session.as_ref() {
        Some(s) => s,
        None => {
            let text = Paragraph::new("No Roundhouse session active")
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(colors.roundhouse)),
                );
            frame.render_widget(text, area);
            return;
        }
    };

    // Determine if bottom bar is visible
    let show_bottom_bar = matches!(
        session.phase,
        RoundhousePhase::ReviewingPlans | RoundhousePhase::ReviewingCritiques
    ) || session.annotation_input.is_some();

    let bottom_height = if show_bottom_bar { 3 } else { 0 };

    // Split vertically: main area + optional bottom bar
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if show_bottom_bar {
            vec![Constraint::Min(1), Constraint::Length(bottom_height)]
        } else {
            vec![Constraint::Min(1)]
        })
        .split(area);

    let main_area = vertical[0];

    // Split main area horizontally: left 65%, right 35%
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(main_area);

    let left_area = horizontal[0];
    let right_area = horizontal[1];

    // --- Left panel: model viewer ---
    render_model_viewer(frame, session, &colors, left_area);

    // --- Right panel: navigator ---
    render_navigator(frame, session, &colors, right_area);

    // --- Bottom bar ---
    if show_bottom_bar {
        render_bottom_bar(frame, session, &colors, vertical[1]);
    }
}

/// Get the plan/critique status for a model by index.
fn model_plan_status(
    session: &crate::roundhouse::session::RoundhouseSession,
    index: usize,
) -> &PlannerStatus {
    if index == 0 {
        &session.primary_status
    } else {
        session
            .secondaries
            .get(index - 1)
            .map(|s| &s.status)
            .unwrap_or(&PlannerStatus::Pending)
    }
}

fn model_critique_status(
    session: &crate::roundhouse::session::RoundhouseSession,
    index: usize,
) -> &PlannerStatus {
    if index == 0 {
        &session.primary_critique_status
    } else {
        session
            .secondaries
            .get(index - 1)
            .map(|s| &s.critique_status)
            .unwrap_or(&PlannerStatus::Pending)
    }
}

fn status_for_phase(
    session: &crate::roundhouse::session::RoundhouseSession,
    index: usize,
) -> &PlannerStatus {
    match session.phase {
        RoundhousePhase::Critiquing | RoundhousePhase::ReviewingCritiques => {
            model_critique_status(session, index)
        }
        _ => model_plan_status(session, index),
    }
}

fn status_icon_and_style(status: &PlannerStatus, colors: &theme::Colors) -> (&'static str, Style) {
    match status {
        PlannerStatus::Pending => ("○", Style::default().fg(colors.text_dim)),
        PlannerStatus::Thinking => ("●", Style::default().fg(colors.roundhouse)),
        PlannerStatus::Streaming => ("●", Style::default().fg(colors.roundhouse)),
        PlannerStatus::UsingTool(_) => ("⚙", Style::default().fg(colors.warning)),
        PlannerStatus::Done => ("✓", Style::default().fg(colors.success)),
        PlannerStatus::Failed(_) => ("✗", Style::default().fg(colors.error)),
        PlannerStatus::TimedOut => ("⏱", Style::default().fg(colors.warning)),
    }
}

fn status_text(status: &PlannerStatus) -> &str {
    match status {
        PlannerStatus::Pending => "pending",
        PlannerStatus::Thinking => "thinking",
        PlannerStatus::Streaming => "streaming",
        PlannerStatus::UsingTool(_) => "using tool",
        PlannerStatus::Done => "done",
        PlannerStatus::Failed(_) => "failed",
        PlannerStatus::TimedOut => "timed out",
    }
}

fn render_model_viewer(
    frame: &mut Frame,
    session: &crate::roundhouse::session::RoundhouseSession,
    colors: &theme::Colors,
    area: Rect,
) {
    let selected = session.selected_model_index;
    let model_name = session.model_display_name(selected);
    let current_status = status_for_phase(session, selected);
    let title_text = format!(" {} — {} ", model_name, status_text(current_status));

    let content = match session.phase {
        RoundhousePhase::Critiquing | RoundhousePhase::ReviewingCritiques => {
            session.selected_critique_text()
        }
        RoundhousePhase::Synthesizing | RoundhousePhase::Complete => {
            &session.synthesis_streaming_text
        }
        _ => session.selected_model_text(),
    };

    let paragraph = Paragraph::new(content)
        .wrap(Wrap { trim: false })
        .scroll((session.viewer_scroll_offset, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors.roundhouse))
                .title(Span::styled(
                    title_text,
                    Style::default()
                        .fg(colors.roundhouse)
                        .add_modifier(Modifier::BOLD),
                )),
        );

    frame.render_widget(paragraph, area);
}

fn render_navigator(
    frame: &mut Frame,
    session: &crate::roundhouse::session::RoundhouseSession,
    colors: &theme::Colors,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 2 || inner.height < 2 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Phase name
    let phase_name = match session.phase {
        RoundhousePhase::SelectingProviders => "Selecting Providers",
        RoundhousePhase::AwaitingPrompt => "Awaiting Prompt",
        RoundhousePhase::Planning => "Planning",
        RoundhousePhase::ReviewingPlans => "Reviewing Plans",
        RoundhousePhase::Critiquing => "Critiquing",
        RoundhousePhase::ReviewingCritiques => "Reviewing Critiques",
        RoundhousePhase::Synthesizing => "Synthesizing",
        RoundhousePhase::Complete => "Complete",
        RoundhousePhase::Cancelled => "Cancelled",
    };
    lines.push(Line::from(Span::styled(
        format!("  {}", phase_name),
        Style::default()
            .fg(colors.roundhouse)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Model list
    let model_count = session.model_count();
    let max_name_width = inner.width.saturating_sub(7) as usize; // "  ▶ ● " = ~6 chars + space

    for i in 0..model_count {
        let is_selected = i == session.selected_model_index;
        let status = status_for_phase(session, i);
        let (icon, icon_style) = status_icon_and_style(status, colors);

        let mut name = session.model_display_name(i);
        if name.len() > max_name_width {
            name.truncate(max_name_width.saturating_sub(1));
            name.push('…');
        }

        let marker = if is_selected { "▶" } else { " " };
        let marker_style = if is_selected {
            Style::default().fg(colors.roundhouse)
        } else {
            Style::default()
        };

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(marker, marker_style),
            Span::raw(" "),
            Span::styled(icon, icon_style),
            Span::raw(" "),
            Span::styled(name, Style::default().fg(colors.text_secondary)),
        ]));
    }

    lines.push(Line::from(""));

    // Cost
    if session.total_cost > 0.0 {
        lines.push(Line::from(Span::styled(
            format!("  ${:.4}", session.total_cost),
            Style::default().fg(colors.text_secondary),
        )));
    }

    // Annotation count
    if !session.annotations.is_empty() {
        let count = session.annotations.len();
        let label = if count == 1 {
            "1 annotation".to_string()
        } else {
            format!("{} annotations", count)
        };
        lines.push(Line::from(Span::styled(
            format!("  {}", label),
            Style::default().fg(colors.text_secondary),
        )));
    }

    // Keybind hints at bottom — we'll fill remaining space then add hints
    // Calculate how many blank lines to push hints to bottom
    let content_lines = lines.len();
    let hint_lines = 2;
    let available = inner.height as usize;
    if available > content_lines + hint_lines {
        let padding = available - content_lines - hint_lines;
        for _ in 0..padding {
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(Span::styled(
        "  j/k  switch model",
        Style::default().fg(colors.text_dim),
    )));
    lines.push(Line::from(Span::styled(
        "  ↑/↓  scroll output",
        Style::default().fg(colors.text_dim),
    )));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn render_bottom_bar(
    frame: &mut Frame,
    session: &crate::roundhouse::session::RoundhouseSession,
    colors: &theme::Colors,
    area: Rect,
) {
    // If annotation input is active, show that
    if let Some(ref input) = session.annotation_input {
        let text = format!("  annotation: {}█", input);
        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(colors.roundhouse)),
        );
        frame.render_widget(paragraph, area);
        return;
    }

    // Gate action hints
    let hint_text = match session.phase {
        RoundhousePhase::ReviewingPlans => {
            if session.critique_enabled {
                "  [c] critique  [s] skip to synthesis  [a] annotate  [q] cancel"
            } else {
                "  [s] skip to synthesis  [a] annotate  [q] cancel"
            }
        }
        RoundhousePhase::ReviewingCritiques => "  [s] synthesize  [a] annotate  [q] cancel",
        _ => return,
    };

    let paragraph = Paragraph::new(hint_text).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(colors.border)),
    );
    frame.render_widget(paragraph, area);
}
