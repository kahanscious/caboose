use anyhow::Result;
use std::path::{Path, PathBuf};

/// Write the synthesized plan to a temporary markdown file
pub fn write_plan_file(cwd: &Path, plan_content: &str, prompt_summary: &str) -> Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let slug: String = prompt_summary
        .chars()
        .take(30)
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let filename = format!("roundhouse-{timestamp}-{slug}.md");
    let path = cwd.join(&filename);
    std::fs::write(&path, plan_content)?;
    Ok(path)
}

/// Format individual plans into a reviewable document
pub fn format_plans_document(
    prompt: &str,
    individual_plans: &[(&str, &str)],
    synthesized_plan: &str,
    critiques: Option<&[(&str, &str)]>,
) -> String {
    let mut doc = String::new();
    doc.push_str("# Roundhouse Plan\n\n");
    doc.push_str(&format!("## Prompt\n\n{prompt}\n\n"));
    doc.push_str("---\n\n");
    doc.push_str(&format!("## Synthesized Plan\n\n{synthesized_plan}\n\n"));
    doc.push_str("---\n\n");
    doc.push_str("## Individual Plans\n\n");
    for (provider, plan) in individual_plans {
        doc.push_str(&format!("### {provider}\n\n{plan}\n\n"));
    }
    if let Some(crits) = critiques
        && !crits.is_empty()
    {
        doc.push_str("---\n\n");
        doc.push_str("## Critiques\n\n");
        for (provider, critique) in crits {
            doc.push_str(&format!("### {provider}\n\n{critique}\n\n"));
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_plans_document() {
        let doc = format_plans_document(
            "build auth",
            &[("openai", "Plan A"), ("gemini", "Plan B")],
            "Unified plan here",
            None,
        );
        assert!(doc.contains("# Roundhouse Plan"));
        assert!(doc.contains("build auth"));
        assert!(doc.contains("Unified plan here"));
        assert!(doc.contains("Plan A"));
        assert!(doc.contains("Plan B"));
    }

    #[test]
    fn test_format_plans_document_with_critiques() {
        let doc = format_plans_document(
            "build auth",
            &[("openai", "Plan A"), ("gemini", "Plan B")],
            "Unified plan here",
            Some(&[
                ("openai", "Critique of Plan A"),
                ("gemini", "Critique of Plan B"),
            ]),
        );
        assert!(doc.contains("## Critiques"));
        assert!(doc.contains("### openai"));
        assert!(doc.contains("Critique of Plan A"));
        assert!(doc.contains("### gemini"));
        assert!(doc.contains("Critique of Plan B"));
    }

    #[test]
    fn test_format_plans_document_without_critiques() {
        let doc = format_plans_document(
            "build auth",
            &[("openai", "Plan A")],
            "Unified plan here",
            None,
        );
        assert!(!doc.contains("## Critiques"));
        assert!(doc.contains("## Synthesized Plan"));
        assert!(doc.contains("## Individual Plans"));

        // Also verify empty slice produces no critiques section
        let doc2 = format_plans_document(
            "build auth",
            &[("openai", "Plan A")],
            "Unified plan here",
            Some(&[]),
        );
        assert!(!doc2.contains("## Critiques"));
    }
}
