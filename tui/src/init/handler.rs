use std::fs;
use std::path::{Path, PathBuf};

use super::scanner::RepoContext;

/// Events emitted by the background init generation task.
pub enum InitEvent {
    /// A chunk of streamed text from the LLM.
    TextDelta(String),
    /// Generation finished successfully with token usage.
    Done {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// An error occurred during generation.
    Error(String),
}

/// System prompt instructing the LLM how to generate a CABOOSE.md file.
const GENERATION_PROMPT: &str = "\
You are a developer-documentation assistant. Given repository context \
(file tree, config files, README, and optionally an existing CABOOSE.md), \
generate a CABOOSE.md file that a coding AI agent can use to understand \
the project.

Write the following sections:
- **Project Overview** — what the project is and does (1-3 sentences)
- **Build & Dev Commands** — how to install, build, and run in development
- **Test Commands** — how to run the full test suite and individual tests
- **Code Style** — linting, formatting, naming conventions, import ordering
- **Architecture** — key directories, modules, and how they relate
- **Gotchas** — non-obvious footguns, workarounds, or env-specific issues

Rules:
- Start the file with `# CABOOSE.md`
- Keep the output under 200 lines
- Be factual — only state what the repository context supports
- Use fenced code blocks for commands
- Do not invent features or commands that are not evident in the context";

/// Extra instruction appended when an existing CABOOSE.md is being updated.
const MERGE_INSTRUCTION: &str = "\n\n\
Update it with new findings, preserve accurate existing content, \
remove outdated content.";

/// Build the full LLM prompt from a scanned `RepoContext`.
///
/// The prompt includes the generation instructions, optional merge
/// instructions (when an existing CABOOSE.md is present), and all
/// collected repository signals wrapped in XML-style tags.
pub fn build_prompt(ctx: &RepoContext) -> String {
    let mut prompt = String::from(GENERATION_PROMPT);

    // If there is an existing CABOOSE.md, append merge instructions
    if ctx.existing_caboose.is_some() {
        prompt.push_str(MERGE_INSTRUCTION);
    }

    prompt.push_str("\n\nHere is the repository context:\n\n");

    // File tree
    prompt.push_str("<file_tree>\n");
    prompt.push_str(&ctx.file_tree);
    prompt.push_str("\n</file_tree>\n\n");

    // Config files
    if !ctx.config_files.is_empty() {
        prompt.push_str("<config_files>\n");
        for (name, contents) in &ctx.config_files {
            prompt.push_str(&format!("--- {} ---\n{}\n", name, contents));
        }
        prompt.push_str("</config_files>\n\n");
    }

    // README (optional)
    if let Some(readme) = &ctx.readme {
        prompt.push_str("<readme>\n");
        prompt.push_str(readme);
        prompt.push_str("\n</readme>\n\n");
    }

    // Existing CABOOSE.md (optional)
    if let Some(existing) = &ctx.existing_caboose {
        prompt.push_str("<existing_caboose_md>\n");
        prompt.push_str(existing);
        prompt.push_str("\n</existing_caboose_md>\n\n");
    }

    prompt
}

/// Write a CABOOSE.md file to disk at the given root directory.
///
/// Returns the full path of the written file and its line count.
pub fn write_caboose_md(root: &Path, content: &str) -> std::io::Result<(PathBuf, usize)> {
    let path = root.join("CABOOSE.md");
    fs::write(&path, content)?;
    let line_count = content.lines().count();
    Ok((path, line_count))
}

/// Inject CABOOSE.md content into a system prompt.
/// Returns the prompt with CABOOSE.md content appended (truncated to 200 lines).
pub fn inject_caboose_md(mut prompt: String, content: Option<&str>) -> String {
    let Some(content) = content else {
        return prompt;
    };
    let truncated: String = content.lines().take(200).collect::<Vec<_>>().join("\n");
    prompt.push_str("\n\n## Project Instructions (CABOOSE.md)\n\n");
    prompt.push_str(&truncated);
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper to build a minimal RepoContext for testing.
    fn mock_context(
        file_tree: &str,
        config_files: Vec<(&str, &str)>,
        readme: Option<&str>,
        existing_caboose: Option<&str>,
    ) -> RepoContext {
        RepoContext {
            root: PathBuf::from("/tmp/mock"),
            file_tree: file_tree.to_string(),
            config_files: config_files
                .into_iter()
                .map(|(n, c)| (n.to_string(), c.to_string()))
                .collect(),
            readme: readme.map(|s| s.to_string()),
            existing_caboose: existing_caboose.map(|s| s.to_string()),
        }
    }

    #[test]
    fn prompt_includes_file_tree() {
        let ctx = mock_context("src/\nsrc/main.rs\nCargo.toml", vec![], None, None);
        let prompt = build_prompt(&ctx);

        assert!(
            prompt.contains("src/main.rs"),
            "Expected file tree entry src/main.rs in prompt"
        );
        assert!(
            prompt.contains("<file_tree>"),
            "Expected <file_tree> tag in prompt"
        );
        assert!(
            prompt.contains("</file_tree>"),
            "Expected </file_tree> tag in prompt"
        );
    }

    #[test]
    fn prompt_includes_config_contents() {
        let ctx = mock_context(
            "",
            vec![("Cargo.toml", "[package]\nname = \"demo\"")],
            None,
            None,
        );
        let prompt = build_prompt(&ctx);

        assert!(
            prompt.contains("--- Cargo.toml ---"),
            "Expected config file header"
        );
        assert!(
            prompt.contains("[package]"),
            "Expected config file content in prompt"
        );
        assert!(
            prompt.contains("<config_files>"),
            "Expected <config_files> tag in prompt"
        );
    }

    #[test]
    fn prompt_includes_readme() {
        let ctx = mock_context("", vec![], Some("# My Project\nA great project."), None);
        let prompt = build_prompt(&ctx);

        assert!(
            prompt.contains("# My Project"),
            "Expected README content in prompt"
        );
        assert!(
            prompt.contains("<readme>"),
            "Expected <readme> tag in prompt"
        );
        assert!(
            prompt.contains("</readme>"),
            "Expected </readme> tag in prompt"
        );
    }

    #[test]
    fn prompt_includes_existing_for_merge() {
        let ctx = mock_context("", vec![], None, Some("# CABOOSE.md\nOld content here."));
        let prompt = build_prompt(&ctx);

        assert!(
            prompt.contains("<existing_caboose_md>"),
            "Expected <existing_caboose_md> tag when existing CABOOSE.md is present"
        );
        assert!(
            prompt.contains("Old content here."),
            "Expected existing CABOOSE.md content in prompt"
        );
        assert!(
            prompt.contains("Update"),
            "Expected merge instruction word 'Update' in prompt"
        );
    }

    #[test]
    fn prompt_no_merge_when_no_existing() {
        let ctx = mock_context("", vec![], None, None);
        let prompt = build_prompt(&ctx);

        assert!(
            !prompt.contains("existing_caboose_md"),
            "Expected no <existing_caboose_md> tag when no existing CABOOSE.md"
        );
    }

    #[test]
    fn write_creates_file() {
        let tmp = TempDir::new().unwrap();
        let content = "# CABOOSE.md\nLine 2\nLine 3\n";

        let (path, line_count) = write_caboose_md(tmp.path(), content).unwrap();

        assert_eq!(path, tmp.path().join("CABOOSE.md"));
        assert_eq!(line_count, 3);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn write_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        let caboose_path = tmp.path().join("CABOOSE.md");
        fs::write(&caboose_path, "old content\n").unwrap();

        let new_content = "# CABOOSE.md\nNew content\n";
        let (path, line_count) = write_caboose_md(tmp.path(), new_content).unwrap();

        assert_eq!(path, caboose_path);
        assert_eq!(line_count, 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), new_content);
    }

    #[test]
    fn inject_caboose_md_into_prompt() {
        let base = "You are a helpful assistant.".to_string();
        let caboose_content = "# CABOOSE.md\n\n## Project Overview\nTest project.";
        let result = super::inject_caboose_md(base, Some(caboose_content));
        assert!(result.contains("## Project Instructions (CABOOSE.md)"));
        assert!(result.contains("Test project."));
    }

    #[test]
    fn inject_caboose_md_none() {
        let base = "You are a helpful assistant.".to_string();
        let result = super::inject_caboose_md(base.clone(), None);
        assert_eq!(result, base);
    }

    #[test]
    fn inject_caboose_md_truncates_at_200_lines() {
        let base = "Base prompt.".to_string();
        let long = (0..300)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = super::inject_caboose_md(base, Some(&long));
        let injected_section = result.split("## Project Instructions").nth(1).unwrap();
        let line_count = injected_section.trim().lines().count();
        assert!(line_count <= 202, "expected <=202 lines, got {line_count}");
    }
}
