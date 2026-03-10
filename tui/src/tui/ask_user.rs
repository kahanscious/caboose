//! Ask-user tool — inline question widget for agent-initiated questions.

use serde::Deserialize;

/// A single question from the agent's ask_user tool call.
#[derive(Debug, Clone, Deserialize)]
pub struct AskUserQuestion {
    pub question: String,
    pub header: String,
    pub options: Vec<AskUserOption>,
    #[serde(default, rename = "multiSelect")]
    pub multi_select: bool,
}

/// An option within a question.
#[derive(Debug, Clone, Deserialize)]
pub struct AskUserOption {
    pub label: String,
    pub description: String,
}

/// Active ask-user session — tracks which question we're on and user selections.
#[derive(Debug)]
pub struct AskUserSession {
    /// The tool call ID to return results for.
    pub tool_call_id: String,
    /// All questions in this ask_user call.
    pub questions: Vec<AskUserQuestion>,
    /// Current question index (0-based).
    pub current_question: usize,
    /// Accumulated answers: question text → answer string.
    pub answers: Vec<(String, String)>,
    /// For multi-select: currently toggled option indices.
    pub toggled: std::collections::HashSet<usize>,
}

impl AskUserSession {
    pub fn new(tool_call_id: String, questions: Vec<AskUserQuestion>) -> Self {
        Self {
            tool_call_id,
            questions,
            current_question: 0,
            answers: Vec::new(),
            toggled: std::collections::HashSet::new(),
        }
    }

    /// Get the current question, if any remain.
    pub fn current(&self) -> Option<&AskUserQuestion> {
        self.questions.get(self.current_question)
    }

    /// Whether all questions have been answered.
    pub fn is_complete(&self) -> bool {
        self.current_question >= self.questions.len()
    }

    /// Format all answers in Claude Code style.
    pub fn format_answers(&self) -> String {
        if self.answers.is_empty() {
            return "User dismissed the question.".to_string();
        }
        let parts: Vec<String> = self
            .answers
            .iter()
            .map(|(q, a)| format!("\"{q}\"=\"{a}\""))
            .collect();
        format!("User has answered your questions: {}", parts.join(", "))
    }
}

/// Render an ask-user question as styled Lines for inline chat display.
#[allow(clippy::too_many_arguments)]
pub fn render_question(
    header: &str,
    question: &str,
    options: &[(String, String)],
    answer: Option<&str>,
    multi_select: bool,
    toggled: &std::collections::HashSet<usize>,
    colors: &crate::tui::theme::Colors,
    accent: ratatui::style::Color,
) -> Vec<ratatui::prelude::Line<'static>> {
    use ratatui::prelude::*;

    let mut lines = Vec::new();

    // Header line: ┌─ Ask User: Header ─────
    let title = if header.is_empty() {
        " Ask User ".to_string()
    } else {
        format!(" Ask User: {header} ")
    };
    lines.push(Line::from(Span::styled(
        format!("┌─{title}{}", "─".repeat(200)),
        Style::default().fg(accent),
    )));

    // Question text
    lines.push(Line::from(Span::styled(
        format!("│ {question}"),
        Style::default().fg(colors.text),
    )));
    lines.push(Line::from(Span::styled(
        "│".to_string(),
        Style::default().fg(accent),
    )));

    if let Some(ans) = answer {
        // Answered — show collapsed result
        lines.push(Line::from(vec![
            Span::styled("│  → ", Style::default().fg(accent)),
            Span::styled(ans.to_string(), Style::default().fg(colors.text).bold()),
        ]));
    } else {
        // Active — show options
        for (i, (label, desc)) in options.iter().enumerate() {
            let num = i + 1;
            let prefix = if multi_select {
                if toggled.contains(&i) { "☑" } else { "☐" }
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::styled(format!("│  [{num}] "), Style::default().fg(accent)),
                Span::styled(
                    if multi_select {
                        format!("{prefix} {label}")
                    } else {
                        label.clone()
                    },
                    Style::default().fg(colors.text).bold(),
                ),
                Span::styled(
                    format!(" — {desc}"),
                    Style::default().fg(colors.text_secondary),
                ),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "│".to_string(),
            Style::default().fg(accent),
        )));

        let hint = if multi_select {
            "│ Press 1-N to toggle, Enter to confirm, or type a custom answer"
        } else {
            "│ Press 1-N to select, or type a custom answer"
        };
        lines.push(Line::from(Span::styled(
            hint.to_string(),
            Style::default().fg(colors.text_dim),
        )));
    }

    // Footer
    lines.push(Line::from(Span::styled(
        format!("└{}", "─".repeat(200)),
        Style::default().fg(accent),
    )));
    lines.push(Line::from(""));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_questions() -> Vec<AskUserQuestion> {
        vec![
            AskUserQuestion {
                question: "Which DB?".to_string(),
                header: "Database".to_string(),
                options: vec![
                    AskUserOption {
                        label: "PostgreSQL".to_string(),
                        description: "Battle-tested".to_string(),
                    },
                    AskUserOption {
                        label: "SQLite".to_string(),
                        description: "Embedded".to_string(),
                    },
                ],
                multi_select: false,
            },
            AskUserQuestion {
                question: "Auth method?".to_string(),
                header: "Auth".to_string(),
                options: vec![
                    AskUserOption {
                        label: "JWT".to_string(),
                        description: "Stateless".to_string(),
                    },
                    AskUserOption {
                        label: "Session".to_string(),
                        description: "Stateful".to_string(),
                    },
                ],
                multi_select: false,
            },
        ]
    }

    #[test]
    fn new_session_starts_at_first_question() {
        let session = AskUserSession::new("tc1".into(), sample_questions());
        assert_eq!(session.current_question, 0);
        assert!(!session.is_complete());
        assert_eq!(session.current().unwrap().header, "Database");
    }

    #[test]
    fn format_answers_claude_code_style() {
        let mut session = AskUserSession::new("tc1".into(), sample_questions());
        session
            .answers
            .push(("Which DB?".into(), "PostgreSQL".into()));
        session.answers.push(("Auth method?".into(), "JWT".into()));
        session.current_question = 2;

        assert_eq!(
            session.format_answers(),
            r#"User has answered your questions: "Which DB?"="PostgreSQL", "Auth method?"="JWT""#
        );
    }

    #[test]
    fn format_answers_empty_is_dismissed() {
        let session = AskUserSession::new("tc1".into(), sample_questions());
        assert_eq!(session.format_answers(), "User dismissed the question.");
    }

    #[test]
    fn is_complete_when_all_answered() {
        let mut session = AskUserSession::new("tc1".into(), sample_questions());
        session.current_question = 2; // past both questions
        assert!(session.is_complete());
    }
}
