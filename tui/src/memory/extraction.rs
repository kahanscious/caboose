//! End-of-session memory extraction — sends observations to LLM for fact extraction.

use crate::memory::observations::Observation;

/// Minimum observations to trigger extraction (skip trivial sessions).
pub const MIN_OBSERVATIONS: u64 = 3;

/// Build the extraction prompt from observations and current memory content.
pub fn build_extraction_prompt(
    observations: &[Observation],
    current_project_memory: Option<&str>,
) -> String {
    let mut prompt = String::from(
        "Based on this session's tool activity, identify key facts worth remembering \
         for future sessions. Focus on: project structure, user preferences, architectural \
         decisions, recurring patterns, build/test commands.\n\n\
         Output ONLY new facts to add, one per line, prefixed with \"- \". \
         Do not repeat anything already in the memory file. \
         If nothing new is worth remembering, output exactly \"NO_NEW_MEMORIES\".\n\n",
    );

    if let Some(existing) = current_project_memory {
        prompt.push_str("Current project memories:\n```\n");
        prompt.push_str(existing);
        prompt.push_str("\n```\n\n");
    }

    prompt.push_str("Session observations:\n");
    for obs in observations {
        prompt.push_str(&format!(
            "- [{}] {} — {}\n",
            obs.kind, obs.target, obs.summary
        ));
    }

    prompt
}

/// Parse extraction response into memory lines to append.
/// Returns None if the LLM says NO_NEW_MEMORIES.
pub fn parse_extraction_response(response: &str) -> Option<Vec<String>> {
    let trimmed = response.trim();
    if trimmed == "NO_NEW_MEMORIES" || trimmed.is_empty() {
        return None;
    }
    let lines: Vec<String> = trimmed
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() { None } else { Some(lines) }
}

/// Append new memory lines to a MEMORY.md file.
pub fn append_to_memory_file(path: &std::path::Path, lines: &[String]) -> anyhow::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file)?; // blank line separator
    for line in lines {
        writeln!(file, "{}", line)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_no_new_memories() {
        assert!(parse_extraction_response("NO_NEW_MEMORIES").is_none());
        assert!(parse_extraction_response("  NO_NEW_MEMORIES  ").is_none());
        assert!(parse_extraction_response("").is_none());
    }

    #[test]
    fn parse_extracts_lines() {
        let response = "- Project uses Rust with tokio\n- Tests run with cargo test\n- User prefers short commits";
        let lines = parse_extraction_response(response).unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("Rust"));
    }

    #[test]
    fn build_prompt_includes_observations() {
        let obs = vec![Observation {
            id: 1,
            session_id: "s1".into(),
            kind: "read".into(),
            target: "src/main.rs".into(),
            summary: "Read src/main.rs".into(),
            created_at: "2026-03-01".into(),
        }];
        let prompt = build_extraction_prompt(&obs, Some("- existing fact"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("existing fact"));
    }

    #[test]
    fn append_to_memory_file_works() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("MEMORY.md");
        std::fs::write(&path, "# Memory\n").unwrap();

        append_to_memory_file(
            &path,
            &["- New fact one".to_string(), "- New fact two".to_string()],
        )
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Memory"));
        assert!(content.contains("New fact one"));
        assert!(content.contains("New fact two"));
    }

    #[test]
    fn full_extraction_flow() {
        let dir = tempfile::TempDir::new().unwrap();
        let memory_path = dir.path().join("MEMORY.md");
        std::fs::write(&memory_path, "# Memory\n- Existing fact\n").unwrap();

        // Simulate extraction response
        let response = "- Project uses Rust\n- Build with cargo build";
        let new_lines = parse_extraction_response(response).unwrap();
        append_to_memory_file(&memory_path, &new_lines).unwrap();

        let content = std::fs::read_to_string(&memory_path).unwrap();
        assert!(content.contains("Existing fact"));
        assert!(content.contains("Project uses Rust"));
        assert!(content.contains("Build with cargo build"));
    }
}
