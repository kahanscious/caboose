//! Chat message rendering — role-based styling with basic markdown detection.

use ratatui::prelude::*;

use crate::tui::theme::Colors;

/// Animated thinking phrases shared across the thinking block and status indicator.
pub const THINKING_PHRASES: &[&str] = &[
    "Thinking...",
    "Working...",
    "Caboosing...",
    "Chugging along...",
    "Choo chooing...",
];

/// Render a user message as styled Lines.
pub fn render_user_message(
    content: &str,
    images: &[(String, usize)],
    colors: &Colors,
    accent: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Role header: ● You
    lines.push(Line::from(vec![
        Span::styled("● ", Style::default().fg(accent)),
        Span::styled("You", Style::default().fg(colors.user_msg).bold()),
    ]));

    // Image attachment placeholders
    for (display_name, data_len) in images {
        lines.push(render_image_placeholder(display_name, *data_len, colors));
    }

    // Content lines (no indent — area padding handles alignment)
    for text_line in content.lines() {
        lines.push(Line::from(Span::styled(
            text_line.to_string(),
            Style::default().fg(colors.user_msg),
        )));
    }

    lines.push(Line::from("")); // blank separator
    lines
}

/// Render a single image attachment placeholder line.
pub fn render_image_placeholder(
    display_name: &str,
    data_len: usize,
    colors: &Colors,
) -> Line<'static> {
    let size = crate::attachment::format_size(data_len);
    Line::from(Span::styled(
        format!("[image: {display_name} ({size})]"),
        Style::default().fg(colors.text_dim).italic(),
    ))
}

/// Render an assistant message with basic markdown detection.
pub fn render_assistant_message(
    content: &str,
    colors: &Colors,
    accent: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Role header: ● Caboose (lighter grey dot)
    lines.push(Line::from(vec![
        Span::styled("● ", Style::default().fg(colors.text_dim)),
        Span::styled("Caboose", Style::default().fg(colors.text_secondary).bold()),
    ]));

    // Parse content with basic markdown detection
    let parsed = parse_markdown(content, colors, accent);
    lines.extend(parsed);

    lines.push(Line::from("")); // blank separator
    lines
}

const TRUNCATE_THRESHOLD: usize = 100;
const TRUNCATE_HEAD: usize = 20;
const TRUNCATE_TAIL: usize = 10;

/// Render an assistant message with optional truncation for long messages.
pub fn render_assistant_message_truncated(
    content: &str,
    colors: &Colors,
    expanded: bool,
    accent: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Role header (lighter grey dot)
    lines.push(Line::from(vec![
        Span::styled("● ", Style::default().fg(colors.text_dim)),
        Span::styled("Caboose", Style::default().fg(colors.text_secondary).bold()),
    ]));

    let parsed = parse_markdown(content, colors, accent);
    let total = parsed.len();

    if total <= TRUNCATE_THRESHOLD || expanded {
        lines.extend(parsed);
    } else {
        // Head
        lines.extend(parsed[..TRUNCATE_HEAD].to_vec());
        // Collapse indicator
        let hidden = total - TRUNCATE_HEAD - TRUNCATE_TAIL;
        lines.push(Line::from(Span::styled(
            format!("··· {hidden} lines hidden (click or press e to expand) ···"),
            Style::default().fg(colors.text_muted),
        )));
        // Tail
        lines.extend(parsed[total - TRUNCATE_TAIL..].to_vec());
    }

    lines.push(Line::from("")); // blank separator
    lines
}

/// Render a system message.
pub fn render_system_message(content: &str, colors: &Colors) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for text_line in content.lines() {
        lines.push(Line::from(Span::styled(
            text_line.to_string(),
            Style::default().fg(colors.system_msg),
        )));
    }
    lines.push(Line::from(""));
    lines
}

/// Render an error message.
pub fn render_error_message(content: &str, colors: &Colors) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Error: ", Style::default().fg(colors.error).bold()),
        Span::styled(
            content.lines().next().unwrap_or("").to_string(),
            Style::default().fg(colors.error),
        ),
    ]));
    for text_line in content.lines().skip(1) {
        lines.push(Line::from(Span::styled(
            text_line.to_string(),
            Style::default().fg(colors.error),
        )));
    }
    lines.push(Line::from(""));
    lines
}

/// Render a structured provider error with category-specific styling.
pub fn render_provider_error(
    category: &crate::provider::error::ErrorCategory,
    _provider: &str,
    message: &str,
    hint: Option<&str>,
    colors: &Colors,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let border_color = if category.is_transient() {
        colors.warning
    } else {
        colors.error
    };
    let label = category.label();

    // Top border with label
    let label_part = format!("\u{250C} {label} ");
    let rule_len = 40usize.saturating_sub(label_part.len());
    lines.push(Line::from(vec![
        Span::styled(label_part, Style::default().fg(border_color).bold()),
        Span::styled(
            "\u{2500}".repeat(rule_len),
            Style::default().fg(border_color),
        ),
    ]));

    // Message lines
    for text_line in message.lines() {
        lines.push(Line::from(vec![
            Span::styled("\u{2502} ", Style::default().fg(border_color)),
            Span::styled(text_line.to_string(), Style::default().fg(colors.text)),
        ]));
    }

    // Hint
    if let Some(hint_text) = hint {
        lines.push(Line::from(Span::styled(
            "\u{2502}",
            Style::default().fg(border_color),
        )));
        lines.push(Line::from(vec![
            Span::styled("\u{2502} ", Style::default().fg(border_color)),
            Span::styled(
                format!("\u{2192} {hint_text}"),
                Style::default().fg(border_color).bold(),
            ),
        ]));
    }

    // Bottom border
    lines.push(Line::from(Span::styled(
        format!("\u{2514}{}", "\u{2500}".repeat(39)),
        Style::default().fg(border_color),
    )));

    lines.push(Line::from(""));
    lines
}

/// Render a queued user message — dimmed with a "queued" indicator.
#[allow(dead_code)]
pub fn render_queued_message(content: &str, colors: &Colors) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("◌ ", Style::default().fg(colors.text_dim)),
        Span::styled("You", Style::default().fg(colors.text_dim)),
        Span::styled(" (queued)", Style::default().fg(colors.text_dim).italic()),
    ]));

    for text_line in content.lines() {
        lines.push(Line::from(Span::styled(
            text_line.to_string(),
            Style::default().fg(colors.text_dim),
        )));
    }

    lines.push(Line::from(""));
    lines
}

/// Detect if a line is a table separator (e.g., |------|-----|)
fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|')
        && trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
}

fn is_table_separator_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| c == '-' || c == ':')
}

fn render_table(table_lines: &[&str], colors: &Colors) -> Vec<Line<'static>> {
    let mut result = Vec::new();

    let rows: Vec<Vec<String>> = table_lines
        .iter()
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();

    if rows.is_empty() {
        return result;
    }

    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; col_count];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count && !is_table_separator_cell(cell) {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    for (row_idx, row) in rows.iter().enumerate() {
        let is_sep = row.iter().all(|cell| is_table_separator_cell(cell));
        if is_sep {
            let sep: String = widths
                .iter()
                .map(|w| "\u{2500}".repeat(*w + 2))
                .collect::<Vec<_>>()
                .join("\u{253C}");
            result.push(Line::from(Span::styled(
                sep,
                Style::default().fg(colors.border),
            )));
            continue;
        }

        let mut spans = Vec::new();
        for (i, cell) in row.iter().enumerate() {
            let width = widths.get(i).copied().unwrap_or(cell.len());
            let padded = format!(" {:<width$} ", cell, width = width);
            let style = if row_idx == 0 {
                Style::default().fg(colors.text).bold()
            } else {
                Style::default().fg(colors.assistant_msg)
            };
            spans.push(Span::styled(padded, style));
            if i < row.len() - 1 {
                spans.push(Span::styled("\u{2502}", Style::default().fg(colors.border)));
            }
        }
        result.push(Line::from(spans));
    }

    result
}

/// Markdown parser — headers, lists, code blocks, blockquotes, links, bold, inline code.
pub fn parse_markdown(
    content: &str,
    colors: &Colors,
    accent: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_lines: Vec<String> = Vec::new();
    let mut table_lines: Vec<&str> = Vec::new();

    for text_line in content.lines() {
        let trimmed = text_line.trim_start();

        // --- Code block toggle ---
        if let Some(stripped) = trimmed.strip_prefix("```") {
            if in_code_block {
                // End code block — highlight accumulated lines
                let highlighted =
                    crate::tui::highlight::highlight_code(&code_lines.join("\n"), &code_lang);
                for spans in highlighted {
                    let mut line_spans = vec![Span::styled(
                        "\u{2502} ".to_string(),
                        Style::default().fg(colors.code_border).bg(colors.code_bg),
                    )];
                    for span in spans {
                        line_spans.push(Span::styled(
                            span.content.to_string(),
                            span.style.bg(colors.code_bg),
                        ));
                    }
                    lines.push(Line::from(line_spans));
                }
                in_code_block = false;
                code_lang.clear();
                code_lines.clear();
            } else {
                // Start code block
                code_lang = stripped.trim().to_string();
                let label = if code_lang.is_empty() {
                    "\u{2502} ".to_string()
                } else {
                    format!("\u{2502} {}", code_lang)
                };
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(colors.code_border).bg(colors.code_bg),
                )));
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_lines.push(text_line.to_string());
            continue;
        }

        // --- Table accumulation ---
        if trimmed.contains('|')
            && (is_table_separator(trimmed) || trimmed.starts_with('|') || trimmed.ends_with('|'))
        {
            table_lines.push(text_line);
            continue;
        } else if !table_lines.is_empty() {
            lines.extend(render_table(&table_lines, colors));
            table_lines.clear();
        }

        // --- Horizontal rule ---
        {
            let non_space: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
            if non_space.len() >= 3
                && (non_space.chars().all(|c| c == '-')
                    || non_space.chars().all(|c| c == '*')
                    || non_space.chars().all(|c| c == '_'))
            {
                lines.push(Line::from(Span::styled(
                    "\u{2500}".repeat(40),
                    Style::default().fg(colors.horizontal_rule),
                )));
                continue;
            }
        }

        // --- Headers ---
        if let Some(text) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                text.to_string(),
                Style::default().fg(colors.header_h3).bold(),
            )));
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                text.to_string(),
                Style::default().fg(accent).bold(),
            )));
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                text.to_string(),
                Style::default().fg(accent).bold(),
            )));
            continue;
        }

        // --- Blockquotes ---
        if let Some(text) = trimmed.strip_prefix("> ") {
            let inner_spans = parse_inline_formatting(text, colors);
            let mut spans = vec![Span::styled(
                "\u{2502} ".to_string(),
                Style::default().fg(colors.blockquote_border),
            )];
            spans.extend(inner_spans);
            lines.push(Line::from(spans));
            continue;
        }

        // --- Unordered lists ---
        if (trimmed.starts_with("- ") || trimmed.starts_with("* ")) && trimmed.len() > 2 {
            let indent_len = text_line.len() - trimmed.len();
            let extra_indent = " ".repeat(indent_len);
            let text = &trimmed[2..];
            let inner_spans = parse_inline_formatting(text, colors);
            let mut spans = vec![
                Span::raw(extra_indent),
                Span::styled(
                    "\u{2022} ".to_string(),
                    Style::default().fg(colors.list_bullet),
                ),
            ];
            spans.extend(inner_spans);
            lines.push(Line::from(spans));
            continue;
        }

        // --- Ordered lists ---
        if let Some(dot_pos) = trimmed.find(". ") {
            let num_part = &trimmed[..dot_pos];
            if !num_part.is_empty()
                && num_part.len() <= 4
                && num_part.chars().all(|c| c.is_ascii_digit())
            {
                let text = &trimmed[dot_pos + 2..];
                let inner_spans = parse_inline_formatting(text, colors);
                let mut spans = vec![Span::styled(
                    format!("{num_part}. "),
                    Style::default().fg(colors.list_bullet),
                )];
                spans.extend(inner_spans);
                lines.push(Line::from(spans));
                continue;
            }
        }

        // --- Image path hint ---
        let image_extensions = [".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"];
        let lower = trimmed.to_lowercase();
        if image_extensions.iter().any(|ext| lower.contains(ext)) {
            let mut spans = parse_inline_formatting(text_line, colors);
            spans.push(Span::styled(
                " (image \u{2014} open externally)".to_string(),
                Style::default().fg(colors.text_muted),
            ));
            lines.push(Line::from(spans));
            continue;
        }

        // --- Regular line ---
        let spans = parse_inline_formatting(text_line, colors);
        lines.push(Line::from(spans));
    }

    // Flush any remaining table lines
    if !table_lines.is_empty() {
        lines.extend(render_table(&table_lines, colors));
    }

    // Handle unclosed code block
    if in_code_block {
        for code_line in &code_lines {
            lines.push(Line::from(Span::styled(
                format!("\u{2502} {code_line}"),
                Style::default().fg(colors.code_text).bg(colors.code_bg),
            )));
        }
    }

    lines
}

/// Parse inline markdown formatting: **bold**, `code`.
fn parse_inline_formatting(text: &str, colors: &Colors) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    let mut chars = text.chars().peekable();
    let mut current = String::new();
    let base_style = Style::default().fg(colors.assistant_msg);

    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                // Flush current text
                if !current.is_empty() {
                    spans.push(Span::styled(current.clone(), base_style));
                    current.clear();
                }
                chars.next(); // consume second *
                // Collect bold text until **
                let mut bold_text = String::new();
                let mut found_close = false;
                while let Some(bch) = chars.next() {
                    if bch == '*' && chars.peek() == Some(&'*') {
                        chars.next();
                        found_close = true;
                        break;
                    }
                    bold_text.push(bch);
                }
                if found_close {
                    spans.push(Span::styled(
                        bold_text,
                        base_style.add_modifier(Modifier::BOLD),
                    ));
                } else {
                    // No closing ** — render as-is
                    current.push_str("**");
                    current.push_str(&bold_text);
                }
            }
            '`' if chars.peek() != Some(&'`') => {
                // Flush current text
                if !current.is_empty() {
                    spans.push(Span::styled(current.clone(), base_style));
                    current.clear();
                }
                // Collect inline code until `
                let mut code_text = String::new();
                let mut found_close = false;
                for cch in chars.by_ref() {
                    if cch == '`' {
                        found_close = true;
                        break;
                    }
                    code_text.push(cch);
                }
                if found_close {
                    spans.push(Span::styled(
                        code_text,
                        Style::default().fg(colors.code_text).bg(colors.code_bg),
                    ));
                } else {
                    current.push('`');
                    current.push_str(&code_text);
                }
            }
            '[' => {
                // Flush current text
                if !current.is_empty() {
                    spans.push(Span::styled(current.clone(), base_style));
                    current.clear();
                }
                // Collect link text until ]
                let mut link_text = String::new();
                let mut found_close = false;
                for lch in chars.by_ref() {
                    if lch == ']' {
                        found_close = true;
                        break;
                    }
                    link_text.push(lch);
                }
                if found_close && chars.peek() == Some(&'(') {
                    chars.next(); // consume (
                    let mut url = String::new();
                    let mut found_paren = false;
                    for uch in chars.by_ref() {
                        if uch == ')' {
                            found_paren = true;
                            break;
                        }
                        url.push(uch);
                    }
                    if found_paren {
                        spans.push(Span::styled(
                            link_text,
                            Style::default()
                                .fg(colors.link_text)
                                .add_modifier(Modifier::UNDERLINED),
                        ));
                    } else {
                        // Malformed — render as-is
                        current.push('[');
                        current.push_str(&link_text);
                        current.push_str("](");
                        current.push_str(&url);
                    }
                } else if found_close {
                    current.push('[');
                    current.push_str(&link_text);
                    current.push(']');
                } else {
                    current.push('[');
                    current.push_str(&link_text);
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Flush remaining text
    if !current.is_empty() {
        spans.push(Span::styled(current, base_style));
    }

    if spans.len() == 1 {
        // Only the indent — add empty content
        spans.push(Span::raw(String::new()));
    }

    spans
}

/// Render a thinking block, either collapsed or expanded.
///
/// Collapsed: `▶ Thinking...` (typewriter-animated, text_muted, italic)
/// Expanded:  `▼ thinking` header + `│ content` lines (text_dim, plain text)
pub fn render_thinking_block(
    thinking: &str,
    collapsed: bool,
    colors: &Colors,
    tick: u64,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if collapsed {
        const PHRASE_TICKS: u64 = 50;
        let phrase_idx = (tick / PHRASE_TICKS) as usize;
        let chars_visible = ((tick % PHRASE_TICKS) / 2 + 1) as usize;
        let phrase = THINKING_PHRASES[phrase_idx % THINKING_PHRASES.len()];
        let visible: String = phrase.chars().take(chars_visible).collect();

        lines.push(Line::from(vec![
            Span::styled(
                "\u{25B6} ", // ▶
                Style::default().fg(colors.text_muted),
            ),
            Span::styled(visible, Style::default().fg(colors.text_muted).italic()),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(
                "\u{25BC} ", // ▼
                Style::default().fg(colors.text_dim),
            ),
            Span::styled(
                "thinking".to_string(),
                Style::default().fg(colors.text_dim).italic(),
            ),
        ]));

        for text_line in thinking.lines() {
            lines.push(Line::from(vec![
                Span::styled(
                    "\u{2502} ", // │
                    Style::default().fg(colors.text_muted),
                ),
                Span::styled(text_line.to_string(), Style::default().fg(colors.text_dim)),
            ]));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Colors;

    fn colors() -> Colors {
        Colors::default()
    }

    #[test]
    fn parse_h1_header() {
        let lines = parse_markdown("# Hello World", &colors(), Color::Blue);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Hello World"));
    }

    #[test]
    fn parse_h2_header() {
        let lines = parse_markdown("## Section Title", &colors(), Color::Blue);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Section Title"));
    }

    #[test]
    fn parse_unordered_list() {
        let lines = parse_markdown("- Item one\n- Item two", &colors(), Color::Blue);
        assert_eq!(lines.len(), 2);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("\u{2022}") && text.contains("Item one"));
    }

    #[test]
    fn parse_ordered_list() {
        let lines = parse_markdown("1. First\n2. Second", &colors(), Color::Blue);
        assert_eq!(lines.len(), 2);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("1.") && text.contains("First"));
    }

    #[test]
    fn parse_link() {
        let lines = parse_markdown(
            "See [docs](https://example.com) here",
            &colors(),
            Color::Blue,
        );
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("docs"));
        // Should NOT contain the raw URL in display
        assert!(!text.contains("https://example.com"));
    }

    #[test]
    fn parse_horizontal_rule() {
        let lines = parse_markdown("---", &colors(), Color::Blue);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("\u{2500}"));
    }

    #[test]
    fn parse_blockquote() {
        let lines = parse_markdown("> This is quoted", &colors(), Color::Blue);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("\u{2502}") && text.contains("This is quoted"));
    }

    #[test]
    fn code_block_with_syntax_highlighting() {
        let lines = parse_markdown("```rust\nlet x = 42;\n```", &colors(), Color::Blue);
        assert!(
            lines.len() >= 2,
            "should have lang label + highlighted code line"
        );
    }

    #[test]
    fn bold_text_preserved() {
        let lines = parse_markdown("This is **bold** text", &colors(), Color::Blue);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("bold"));
    }

    #[test]
    fn inline_code_preserved() {
        let lines = parse_markdown("Use `foo()` here", &colors(), Color::Blue);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("foo()"));
    }

    #[test]
    fn unclosed_code_block_renders() {
        let lines = parse_markdown("```python\nprint('hello')", &colors(), Color::Blue);
        // Should render something (not panic)
        assert!(lines.len() >= 2);
    }

    #[test]
    fn parse_table() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
        let lines = parse_markdown(md, &colors(), Color::Blue);
        // Should produce 4 lines (header, separator, 2 data rows)
        assert!(
            lines.len() >= 3,
            "expected at least 3 lines, got {}",
            lines.len()
        );
        let header_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            header_text.contains("Name"),
            "header should contain column names"
        );
    }

    #[test]
    fn detect_image_path() {
        let lines = parse_markdown(
            "Here is screenshot.png in the output",
            &colors(),
            Color::Blue,
        );
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("image") || text.contains("externally"),
            "should hint about image, got: {text}"
        );
    }

    #[test]
    fn long_message_truncated() {
        let content: String = (0..150)
            .map(|i| format!("Line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = render_assistant_message_truncated(&content, &colors(), false, Color::Blue);
        assert!(
            lines.len() < 140,
            "expected truncation, got {} lines",
            lines.len()
        );
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            all_text.contains("lines hidden"),
            "should show hidden count"
        );
    }

    #[test]
    fn short_message_not_truncated() {
        let content = "Short message\nJust two lines";
        let lines = render_assistant_message_truncated(content, &colors(), false, Color::Blue);
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(!all_text.contains("lines hidden"));
    }

    #[test]
    fn expanded_message_shows_all() {
        let content: String = (0..150)
            .map(|i| format!("Line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = render_assistant_message_truncated(&content, &colors(), true, Color::Blue);
        assert!(
            lines.len() > 140,
            "expanded should show all, got {} lines",
            lines.len()
        );
    }

    #[test]
    fn render_provider_error_auth() {
        let lines = render_provider_error(
            &crate::provider::error::ErrorCategory::Auth,
            "anthropic",
            "Invalid API key provided",
            Some("Run /connect anthropic to update your key"),
            &colors(),
        );
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            all_text.contains("Auth Error"),
            "should show category label"
        );
        assert!(all_text.contains("Invalid API key"), "should show message");
        assert!(all_text.contains("/connect"), "should show hint");
    }

    #[test]
    fn render_provider_error_network_uses_warning_color() {
        let c = colors();
        let lines = render_provider_error(
            &crate::provider::error::ErrorCategory::Network,
            "openai",
            "Connection refused",
            Some("Check your internet connection"),
            &c,
        );
        let top_line = &lines[0];
        assert_eq!(top_line.spans[0].style.fg, Some(c.warning));
    }

    #[test]
    fn render_provider_error_no_hint() {
        let lines = render_provider_error(
            &crate::provider::error::ErrorCategory::Unknown,
            "test",
            "Something broke",
            None,
            &colors(),
        );
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(!all_text.contains("\u{2192}"), "should not show hint arrow");
    }

    #[test]
    fn render_thinking_collapsed() {
        let lines = render_thinking_block("some thinking content", true, &colors(), 0);
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("\u{25B6}")); // ▶ arrow
    }

    #[test]
    fn render_thinking_expanded() {
        let thinking = "Line one\nLine two\nLine three";
        let lines = render_thinking_block(thinking, false, &colors(), 0);
        // Should have: header line + 3 content lines
        assert_eq!(lines.len(), 4);
        let header: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("\u{25BC}")); // ▼ arrow
        let line1: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(line1.contains("\u{2502}")); // │ border
        assert!(line1.contains("Line one"));
    }

    #[test]
    fn render_thinking_empty_content() {
        let lines = render_thinking_block("", false, &colors(), 0);
        // Empty thinking should still show the header
        assert_eq!(lines.len(), 1);
    }
}
