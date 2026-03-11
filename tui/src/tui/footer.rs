//! Footer — track animation + status bar (no padding row).

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::agent::permission::Mode;
use crate::tui::theme::Colors;

/// Track pattern unit: double-rail with cross-ties.
///   ═══╪  repeating
const TRACK_PATTERN: [char; 4] = ['\u{2550}', '\u{2550}', '\u{2550}', '\u{256a}'];
//                                   ═          ═          ═          ╪

/// Accent color for the current mode.
fn mode_color(mode: Mode, colors: &Colors) -> Color {
    match mode {
        Mode::Plan => colors.info,
        Mode::Create => colors.brand,
        Mode::Chug => colors.warning,
    }
}

/// Generate the track pattern string for a given width.
fn track_string(width: usize) -> String {
    (0..width)
        .map(|i| TRACK_PATTERN[i % TRACK_PATTERN.len()])
        .collect()
}

/// Optional budget info for the status bar.
pub struct BudgetInfo {
    pub session_cost: f64,
    pub max_cost: f64,
}

/// Render the footer into the given area (1 track row + 1 status bar).
///
/// Track is always visible in the mode accent color. When the agent is active,
/// a bold typewriter pulse sweeps across left-to-right then resets.
/// `is_active` controls whether the bold pulse is shown.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    mode: Mode,
    caboose_pos: usize,
    is_active: bool,
    budget: Option<BudgetInfo>,
    update_available: Option<&str>,
) {
    if area.height < 2 {
        return;
    }
    let colors = Colors::default();
    let accent = mode_color(mode, &colors);
    let w = area.width as usize;

    // Padding: 1 row top, 1 row bottom when space allows.
    let pad_top: u16 = if area.height >= 4 { 1 } else { 0 };

    // Fill top padding with bg_primary
    if pad_top > 0 {
        let pad_row = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(Line::from("")), pad_row);
    }

    // --- Track row ---
    let track_row = Rect {
        x: area.x,
        y: area.y + pad_top,
        width: area.width,
        height: 1,
    };
    let dim_style = Style::default().fg(colors.text_dim).bg(colors.bg_primary);
    let lit_style = Style::default().fg(accent).bg(colors.bg_primary);

    let track = track_string(w);

    if is_active && w > 0 {
        // Typewriter pulse: accent sweeps left-to-right over dim track, then resets.
        let pos = caboose_pos % w;
        let byte_pos: usize = track
            .char_indices()
            .nth(pos)
            .map_or(track.len(), |(i, _)| i);
        let mut spans: Vec<Span> = Vec::new();
        if byte_pos > 0 {
            spans.push(Span::styled(&track[..byte_pos], lit_style));
        }
        if byte_pos < track.len() {
            spans.push(Span::styled(&track[byte_pos..], dim_style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), track_row);
    } else {
        // Idle: full dim track
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(&track, dim_style))),
            track_row,
        );
    }

    // --- Status bar ---
    let status_row = Rect {
        x: area.x,
        y: area.y + pad_top + 1,
        width: area.width,
        height: 1,
    };
    let status_style = Style::default().fg(colors.text_dim).bg(colors.bg_elevated);

    let cwd = std::env::current_dir()
        .map(|p| {
            if let Some(home) = dirs::home_dir()
                && let Ok(rest) = p.strip_prefix(&home)
            {
                return format!("~/{}", rest.display());
            }
            p.display().to_string()
        })
        .unwrap_or_default();

    let version = env!("CARGO_PKG_VERSION");
    let left = format!(" {cwd}");

    let (right_text, right_spans) = if let Some(new_ver) = update_available {
        let base = format!("caboose v{version} ");
        let update = format!("→ v{new_ver} available · run `caboose update` ");
        let full = format!("{base}{update}");
        let spans = vec![
            Span::styled(base, status_style),
            Span::styled(
                update,
                Style::default().fg(colors.info).bg(colors.bg_elevated),
            ),
        ];
        (full, spans)
    } else {
        let text = format!("caboose v{version} ");
        let spans = vec![Span::styled(text.clone(), status_style)];
        (text, spans)
    };

    // Budget indicator (shown when ≥80% of budget)
    let budget_text = budget.and_then(|b| {
        let pct = b.session_cost / b.max_cost;
        if pct >= 0.80 {
            Some(format!(" ${:.2} / ${:.2} ", b.session_cost, b.max_cost))
        } else {
            None
        }
    });

    let mut status_spans = vec![Span::styled(left.clone(), status_style)];

    if let Some(ref text) = budget_text {
        let budget_style = Style::default().fg(colors.warning).bg(colors.bg_elevated);
        let padding_left = w.saturating_sub(left.len() + text.len() + right_text.len()) / 2;
        let padding_right =
            w.saturating_sub(left.len() + text.len() + right_text.len() + padding_left);
        status_spans.push(Span::styled(" ".repeat(padding_left), status_style));
        status_spans.push(Span::styled(text.clone(), budget_style));
        status_spans.push(Span::styled(" ".repeat(padding_right), status_style));
    } else {
        let padding = w.saturating_sub(left.len() + right_text.len());
        status_spans.push(Span::styled(" ".repeat(padding), status_style));
    }

    status_spans.extend(right_spans);
    frame.render_widget(Paragraph::new(Line::from(status_spans)), status_row);
}
