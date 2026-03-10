//! Task outline renderer — collapsed one-liner in chat, full view in sidebar.

use ratatui::prelude::*;

use crate::app::{TaskOutline, TaskStatus};
use crate::tui::theme::Colors;

/// Render a task outline as a collapsed one-liner in the chat thread.
/// The full task list is displayed in the sidebar.
pub fn render(outline: &TaskOutline, colors: &Colors, _tick: u64) -> Vec<Line<'static>> {
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

    let all_done = done == active && active > 0;
    let icon = if all_done { "\u{2713}" } else { "\u{25cb}" };
    let icon_color = if all_done {
        colors.success
    } else {
        colors.text_dim
    };

    vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(icon, Style::default().fg(icon_color)),
            Span::styled(
                format!(" Tasks ({done}/{active})"),
                Style::default().fg(colors.text_dim),
            ),
        ]),
        Line::from(""),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Task;

    #[test]
    fn render_shows_collapsed_line() {
        let outline = TaskOutline {
            tasks: vec![
                Task {
                    content: "Read files".into(),
                    active_form: "Reading files".into(),
                    status: TaskStatus::Completed,
                },
                Task {
                    content: "Write code".into(),
                    active_form: "Writing code".into(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    content: "Run tests".into(),
                    active_form: "Running tests".into(),
                    status: TaskStatus::Pending,
                },
                Task {
                    content: "Skipped step".into(),
                    active_form: "Skipping step".into(),
                    status: TaskStatus::Cancelled,
                },
            ],
        };
        let colors = Colors::default();
        let lines = render(&outline, &colors, 0);
        // collapsed one-liner + spacer = 2 lines
        assert_eq!(lines.len(), 2);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("1/3"),
            "Expected '1/3' in collapsed line, got: {text}"
        );
    }

    #[test]
    fn render_excludes_cancelled_from_count() {
        let outline = TaskOutline {
            tasks: vec![
                Task {
                    content: "A".into(),
                    active_form: "A".into(),
                    status: TaskStatus::Completed,
                },
                Task {
                    content: "B".into(),
                    active_form: "B".into(),
                    status: TaskStatus::Cancelled,
                },
                Task {
                    content: "C".into(),
                    active_form: "C".into(),
                    status: TaskStatus::Pending,
                },
            ],
        };
        let colors = Colors::default();
        let lines = render(&outline, &colors, 0);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("1/2"), "Expected '1/2' in title, got: {text}");
    }

    #[test]
    fn render_completed_shows_checkmark() {
        let outline = TaskOutline {
            tasks: vec![
                Task {
                    content: "A".into(),
                    active_form: "A".into(),
                    status: TaskStatus::Completed,
                },
                Task {
                    content: "B".into(),
                    active_form: "B".into(),
                    status: TaskStatus::Completed,
                },
            ],
        };
        let colors = Colors::default();
        let lines = render(&outline, &colors, 0);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("2/2"), "Expected '2/2' in title, got: {text}");
        // Should show checkmark icon when all done
        assert!(
            text.contains("\u{2713}"),
            "Expected checkmark in completed state"
        );
    }
}
