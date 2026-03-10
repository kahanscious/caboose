//! Syntax highlighting via syntect — converts code to styled ratatui Spans.

use ratatui::prelude::*;
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let ts = ThemeSet::load_defaults();
    ts.themes["base16-eighties.dark"].clone()
});

/// Map common LLM language names to syntect tokens.
fn normalize_lang(lang: &str) -> &str {
    match lang {
        "typescript" | "tsx" => "ts",
        "javascript" | "jsx" => "js",
        "bash" | "zsh" | "fish" => "sh",
        "dockerfile" => "Dockerfile",
        "makefile" => "Makefile",
        "yml" => "yaml",
        other => other,
    }
}

/// Highlight a code block, returning styled spans per line.
///
/// If the language is unknown, falls back to plain text (one span per line).
pub fn highlight_code(code: &str, lang: &str) -> Vec<Vec<Span<'static>>> {
    let resolved = normalize_lang(lang);
    let syntax = SYNTAX_SET
        .find_syntax_by_token(resolved)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(resolved))
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());

    let is_plain =
        syntax.name == "Plain Text" && !lang.is_empty() && lang != "txt" && lang != "text";

    if is_plain {
        // Unknown language — return single plain span per line
        return code
            .lines()
            .map(|line| vec![Span::raw(line.to_string())])
            .collect();
    }

    let mut highlighter = HighlightLines::new(syntax, &THEME);
    let mut result = Vec::new();

    for line in code.lines() {
        let regions = highlighter
            .highlight_line(line, &SYNTAX_SET)
            .unwrap_or_default();

        let spans: Vec<Span<'static>> = regions
            .into_iter()
            .map(|(style, text)| {
                let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                Span::styled(text.to_string(), Style::default().fg(fg))
            })
            .collect();

        result.push(spans);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_rust_code_produces_colored_spans() {
        let code = "let x = 42;";
        let result = highlight_code(code, "rust");
        assert_eq!(result.len(), 1, "single line of code");
        assert!(
            result[0].len() > 1,
            "should have multiple styled spans, got {}",
            result[0].len()
        );
    }

    #[test]
    fn highlight_unknown_lang_returns_plain_spans() {
        let code = "hello world";
        let result = highlight_code(code, "nosuchlanguage");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
    }

    #[test]
    fn highlight_multiline_code() {
        let code = "fn main() {\n    println!(\"hello\");\n}";
        let result = highlight_code(code, "rust");
        assert_eq!(result.len(), 3, "three lines of code");
    }

    #[test]
    fn highlight_empty_string() {
        let result = highlight_code("", "rust");
        assert!(result.is_empty() || (result.len() == 1 && result[0].is_empty()));
    }

    #[test]
    fn highlight_typescript_via_alias() {
        let code = "const x: number = 42;";
        // LLMs output "typescript" but syntect uses "ts" — our alias handles this
        let result = highlight_code(code, "typescript");
        assert_eq!(result.len(), 1);
        assert!(!result.is_empty());
    }

    #[test]
    fn highlight_bash_via_alias() {
        let code = "echo hello";
        let result = highlight_code(code, "bash");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn highlight_python() {
        let code = "def hello():\n    print('world')";
        let result = highlight_code(code, "python");
        assert_eq!(result.len(), 2);
    }
}
