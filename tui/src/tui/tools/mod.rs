//! Per-tool renderers — each tool type gets its own compact/expanded display.

pub mod bash;
pub mod fetch;
pub mod generic;
pub mod glob;
pub mod grep;
pub mod mcp;
pub mod read;
pub mod skill;
pub mod todo;
pub mod write;

use ratatui::prelude::*;

use crate::app::{ToolMessage, ToolStatus};
use crate::tui::theme::Colors;

/// Shared status icon for all tool renderers.
/// Uses smaller bullet (•) to distinguish from role header dots (●).
/// Includes trailing space so the icon span starts with "• " for turn margin skip.
pub fn status_icon<'a>(status: &ToolStatus, colors: &Colors, tick: u64) -> Span<'a> {
    match status {
        ToolStatus::Pending => Span::styled("\u{2022} ", Style::default().fg(colors.warning)),
        ToolStatus::Running => {
            let dim = (tick / 10) % 2 == 1;
            let color = if dim { colors.text_dim } else { colors.warning };
            Span::styled("\u{2022} ", Style::default().fg(color))
        }
        ToolStatus::Success => Span::styled("\u{2022} ", Style::default().fg(colors.success)),
        ToolStatus::Failed => Span::styled("\u{2022} ", Style::default().fg(colors.error)),
    }
}

/// Trait for per-tool-type rendering.
pub trait ToolRenderer: Send + Sync {
    /// Tool names this renderer handles (e.g., `["read_file", "list_directory"]`).
    fn handles(&self) -> &[&str];

    /// Return true if this renderer should handle the given tool name.
    /// Default implementation checks `handles()` for exact match.
    /// Override for pattern-based matching (e.g., MCP's `:` check).
    fn matches(&self, tool_name: &str) -> bool {
        self.handles().contains(&tool_name)
    }

    /// Render the tool message into styled lines (compact + optional expanded).
    fn render(
        &self,
        tool: &ToolMessage,
        colors: &Colors,
        tick: u64,
        diff_expanded: bool,
        diff_scroll: usize,
    ) -> Vec<Line<'static>>;
}

/// Registry of tool renderers with generic fallback.
pub struct ToolRendererRegistry {
    renderers: Vec<Box<dyn ToolRenderer>>,
}

impl ToolRendererRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            renderers: Vec::new(),
        };
        reg.register(Box::new(bash::BashRenderer));
        reg.register(Box::new(read::ReadRenderer));
        reg.register(Box::new(write::WriteRenderer));
        reg.register(Box::new(glob::GlobRenderer));
        reg.register(Box::new(grep::GrepRenderer));
        reg.register(Box::new(fetch::FetchRenderer));
        reg.register(Box::new(mcp::McpRenderer));
        reg
    }

    pub fn register(&mut self, renderer: Box<dyn ToolRenderer>) {
        self.renderers.push(renderer);
    }

    /// Find matching renderer and render, applying focus highlight if needed.
    pub fn render(
        &self,
        tool: &ToolMessage,
        colors: &Colors,
        focused: bool,
        tick: u64,
        diff_expanded: bool,
        diff_scroll: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = self
            .renderers
            .iter()
            .find(|r| r.matches(&tool.name))
            .map(|r| r.render(tool, colors, tick, diff_expanded, diff_scroll))
            .unwrap_or_else(|| generic::render(tool, colors, tick));

        if focused && let Some(first) = lines.first_mut() {
            let mut spans = vec![Span::styled(
                "\u{25b8} ",
                Style::default().fg(colors.border_active),
            )];
            spans.append(&mut first.spans);
            *first = Line::from(spans).style(Style::default().bg(colors.bg_hover));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ToolMessage, ToolStatus};

    fn make_tool(name: &str) -> ToolMessage {
        ToolMessage {
            name: name.to_string(),
            args: serde_json::Value::Null,
            output: None,
            status: ToolStatus::Success,
            expanded: false,
            file_path: None,
            diff_preview: None,
        }
    }

    #[test]
    fn registry_dispatches_bash() {
        let reg = ToolRendererRegistry::new();
        let tool = make_tool("run_command");
        let colors = Colors::default();
        let lines = reg.render(&tool, &colors, false, 0, false, 0);
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Bash"), "Expected 'Bash' in: {text}");
    }

    #[test]
    fn registry_dispatches_mcp() {
        let reg = ToolRendererRegistry::new();
        let tool = make_tool("server:some_tool");
        let colors = Colors::default();
        let lines = reg.render(&tool, &colors, false, 0, false, 0);
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("MCP"), "Expected 'MCP' in: {text}");
    }

    #[test]
    fn registry_falls_back_to_generic() {
        let reg = ToolRendererRegistry::new();
        let tool = make_tool("unknown_tool_xyz");
        let colors = Colors::default();
        let lines = reg.render(&tool, &colors, false, 0, false, 0);
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("unknown_tool_xyz"),
            "Expected tool name in: {text}"
        );
    }

    #[test]
    fn registry_applies_focus_highlight() {
        let reg = ToolRendererRegistry::new();
        let tool = make_tool("run_command");
        let colors = Colors::default();
        let lines = reg.render(&tool, &colors, true, 0, false, 0);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("\u{25b8}"),
            "Expected focus indicator in: {text}"
        );
    }

    #[test]
    fn skill_render_produces_output() {
        let colors = Colors::default();
        let lines = super::skill::render("brainstorm", "Design exploration", &colors);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Skill"), "Expected 'Skill' in: {text}");
        assert!(
            text.contains("brainstorm"),
            "Expected 'brainstorm' in: {text}"
        );
        assert!(
            text.contains("Design exploration"),
            "Expected description in: {text}"
        );
    }
}
