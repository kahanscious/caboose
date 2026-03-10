//! MCP server input modal — multi-field form for adding a new MCP server.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::theme;

/// Which field is currently focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpField {
    Name,
    Command,
    Args,
}

impl McpField {
    /// Cycle to the next field.
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Command,
            Self::Command => Self::Args,
            Self::Args => Self::Name,
        }
    }

    /// Cycle to the previous field.
    pub fn prev(self) -> Self {
        match self {
            Self::Name => Self::Args,
            Self::Command => Self::Name,
            Self::Args => Self::Command,
        }
    }
}

/// State for the MCP server input modal.
#[derive(Debug)]
pub struct McpServerInputState {
    pub name: String,
    pub command: String,
    pub args: String,
    pub focused: McpField,
}

impl McpServerInputState {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            args: String::new(),
            focused: McpField::Name,
        }
    }

    /// Get a mutable reference to the currently focused field's text.
    pub fn focused_input_mut(&mut self) -> &mut String {
        match self.focused {
            McpField::Name => &mut self.name,
            McpField::Command => &mut self.command,
            McpField::Args => &mut self.args,
        }
    }
}

/// Render the MCP server input as a centered overlay.
pub fn render(frame: &mut Frame, state: &McpServerInputState) {
    let colors = theme::Colors::default();
    let area = frame.area();

    // Center a popup: 60% width, fixed height
    let popup_width = (area.width * 60 / 100).max(40).min(area.width - 4);
    let popup_height = 11u16.min(area.height - 2);
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Add MCP Server ")
        .title_alignment(Alignment::Left)
        .border_style(Style::default().fg(colors.border_active))
        .style(Style::default().bg(colors.bg_elevated));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Name label + input
            Constraint::Length(1), // blank
            Constraint::Length(1), // Command label + input
            Constraint::Length(1), // blank
            Constraint::Length(1), // Args label + input
            Constraint::Length(1), // blank
            Constraint::Length(1), // Hints
            Constraint::Min(0),
        ])
        .split(inner);

    render_field(
        frame,
        chunks[0],
        "Name:",
        &state.name,
        state.focused == McpField::Name,
        &colors,
    );
    render_field(
        frame,
        chunks[2],
        "Command:",
        &state.command,
        state.focused == McpField::Command,
        &colors,
    );
    render_field(
        frame,
        chunks[4],
        "Args:",
        &state.args,
        state.focused == McpField::Args,
        &colors,
    );

    // Hints
    let hints = Line::from(vec![
        Span::styled("  tab ", Style::default().fg(colors.text).bold()),
        Span::styled("next  ", Style::default().fg(colors.text_dim)),
        Span::styled("enter ", Style::default().fg(colors.text).bold()),
        Span::styled("submit  ", Style::default().fg(colors.text_dim)),
        Span::styled("esc ", Style::default().fg(colors.text).bold()),
        Span::styled("cancel", Style::default().fg(colors.text_dim)),
    ]);
    frame.render_widget(Paragraph::new(hints), chunks[6]);
}

/// Render a single labeled field.
fn render_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    colors: &theme::Colors,
) {
    let label_width = 10;
    let input_width = area.width.saturating_sub(label_width + 2);

    let label_area = Rect::new(area.x, area.y, label_width, 1);
    let input_area = Rect::new(area.x + label_width, area.y, input_width, 1);

    let label_style = if focused {
        Style::default().fg(colors.text).bold()
    } else {
        Style::default().fg(colors.text_secondary)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(format!("  {label:<9}"), label_style)),
        label_area,
    );

    let input_text = if value.is_empty() && !focused {
        Span::styled(
            "\u{2504}".repeat(input_width as usize),
            Style::default().fg(colors.text_muted),
        )
    } else if value.is_empty() && focused {
        Span::styled(
            format!(
                "\u{2588}{}",
                "\u{2504}".repeat(input_width.saturating_sub(1) as usize)
            ),
            Style::default().fg(colors.text_muted),
        )
    } else if focused {
        Span::styled(format!("{value}\u{2588}"), Style::default().fg(colors.text))
    } else {
        Span::styled(value.to_string(), Style::default().fg(colors.text))
    };

    let border_color = if focused {
        colors.border_active
    } else {
        colors.border
    };
    let input_block =
        Block::default().style(Style::default().bg(colors.bg_elevated).fg(border_color));
    frame.render_widget(
        Paragraph::new(Line::from(input_text)).block(input_block),
        input_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_starts_on_name() {
        let state = McpServerInputState::new();
        assert_eq!(state.focused, McpField::Name);
        assert!(state.name.is_empty());
        assert!(state.command.is_empty());
        assert!(state.args.is_empty());
    }

    #[test]
    fn field_cycling() {
        assert_eq!(McpField::Name.next(), McpField::Command);
        assert_eq!(McpField::Command.next(), McpField::Args);
        assert_eq!(McpField::Args.next(), McpField::Name);

        assert_eq!(McpField::Name.prev(), McpField::Args);
        assert_eq!(McpField::Args.prev(), McpField::Command);
        assert_eq!(McpField::Command.prev(), McpField::Name);
    }

    #[test]
    fn focused_input_mut_targets_correct_field() {
        let mut state = McpServerInputState::new();

        state.focused = McpField::Name;
        state.focused_input_mut().push_str("test");
        assert_eq!(state.name, "test");

        state.focused = McpField::Command;
        state.focused_input_mut().push_str("npx");
        assert_eq!(state.command, "npx");

        state.focused = McpField::Args;
        state.focused_input_mut().push_str("-y @github/mcp");
        assert_eq!(state.args, "-y @github/mcp");
    }
}
