//! Session export — write chat history as markdown.

use crate::app::ChatMessage;

/// Format a chat message list as markdown.
pub fn format_markdown(title: &str, messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    let date = chrono::Local::now().format("%Y-%m-%d");

    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!("*Exported {date} from Caboose*\n\n---\n\n"));

    for msg in messages {
        match msg {
            ChatMessage::User { content, .. } => {
                out.push_str(&format!("## User\n\n{content}\n\n---\n\n"));
            }
            ChatMessage::Assistant { content, .. } => {
                if !content.is_empty() {
                    out.push_str(&format!("## Assistant\n\n{content}\n\n"));
                }
            }
            ChatMessage::Tool(tool) => {
                let summary = tool_summary(&tool.name, &tool.args);
                out.push_str(&format!("> **{}** {}\n\n", tool.name, summary));
            }
            _ => {} // Skip System, Error, etc.
        }
    }

    out
}

/// Extract a short summary arg for a tool call.
fn tool_summary(name: &str, args: &serde_json::Value) -> String {
    let get_str = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(|s| format!("`{s}`"))
    };

    match name {
        "read_file" | "edit_file" | "write_file" => get_str("file_path").unwrap_or_default(),
        "run_command" => get_str("command").unwrap_or_default(),
        "glob" => get_str("pattern").unwrap_or_default(),
        "grep" => get_str("pattern").unwrap_or_default(),
        "fetch" => get_str("url").unwrap_or_default(),
        _ => String::new(),
    }
}

/// Slugify a title for use as a filename.
pub fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_simple() {
        assert_eq!(slugify("Fix auth middleware"), "fix-auth-middleware");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Hello, World! (test)"), "hello-world-test");
    }

    #[test]
    fn slugify_multiple_spaces() {
        assert_eq!(slugify("a   b   c"), "a-b-c");
    }

    #[test]
    fn tool_summary_read_file() {
        let args = serde_json::json!({"file_path": "src/main.rs"});
        assert_eq!(tool_summary("read_file", &args), "`src/main.rs`");
    }

    #[test]
    fn tool_summary_run_command() {
        let args = serde_json::json!({"command": "cargo test"});
        assert_eq!(tool_summary("run_command", &args), "`cargo test`");
    }

    #[test]
    fn tool_summary_unknown_tool() {
        let args = serde_json::json!({});
        assert_eq!(tool_summary("custom_tool", &args), "");
    }

    #[test]
    fn format_markdown_basic() {
        let messages = vec![
            ChatMessage::User {
                content: "Hello".to_string(),
                images: vec![],
            },
            ChatMessage::Assistant {
                content: "Hi there!".to_string(),
                thinking: None,
            },
        ];
        let md = format_markdown("Test Session", &messages);
        assert!(md.contains("# Test Session"));
        assert!(md.contains("## User"));
        assert!(md.contains("Hello"));
        assert!(md.contains("## Assistant"));
        assert!(md.contains("Hi there!"));
    }

    #[test]
    fn format_markdown_skips_system() {
        let messages = vec![
            ChatMessage::System {
                content: "internal info".to_string(),
            },
            ChatMessage::User {
                content: "Hello".to_string(),
                images: vec![],
            },
        ];
        let md = format_markdown("Test", &messages);
        assert!(!md.contains("internal info"));
        assert!(md.contains("Hello"));
    }
}
