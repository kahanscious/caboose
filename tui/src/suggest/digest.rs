//! Digest formatting — compresses findings into compact markdown for LLM consumption.

use crate::suggest::priority::{Category, Finding, PriorityWeights, Severity};

/// Max findings to show per category section before "(N more)" summary.
const MAX_PER_SECTION: usize = 5;

/// The priority prompt appended after the digest.
pub const PRIORITY_PROMPT: &str = "\
Rank these findings into a prioritized action list. Use this framework:
1. Test failures — broken tests block everything
2. Lint errors — compiler-level issues
3. Lint warnings — code quality
4. FIXMEs/HACKs — known problems flagged by developers
5. TODOs — planned work
6. High-churn files with no recent test changes — likely test debt

For each suggestion, give: priority rank, what to fix, why, and estimated effort (small/medium/large).
Keep the list to 10 items max. Group related items.";

/// Format findings into a compact markdown digest.
pub fn build_digest(findings: &[Finding], weights: &PriorityWeights) -> String {
    let mut out = String::from("## Codebase scan results\n");

    if findings.is_empty() {
        out.push_str("\nNo issues found — codebase looks clean.\n");
        return out;
    }

    // Standard sections sorted by weight
    #[allow(clippy::type_complexity)]
    let mut sections: Vec<(u8, &str, Box<dyn Fn(&Finding) -> bool>)> = vec![
        (
            weights.test_failure,
            "Test failures",
            Box::new(|f: &Finding| f.category == Category::Test),
        ),
        (
            weights.lint_error,
            "Lint errors",
            Box::new(|f: &Finding| f.category == Category::Lint && f.severity == Severity::Error),
        ),
        (
            weights.lint_warning,
            "Lint warnings",
            Box::new(|f: &Finding| f.category == Category::Lint && f.severity != Severity::Error),
        ),
        (
            weights.todo,
            "TODOs/FIXMEs",
            Box::new(|f: &Finding| f.category == Category::Todo),
        ),
        (
            weights.recent_churn,
            "Recent churn",
            Box::new(|f: &Finding| f.category == Category::Churn),
        ),
    ];
    sections.sort_by_key(|(w, _, _)| *w);

    for (weight, label, predicate) in &sections {
        let matched: Vec<_> = findings.iter().filter(|f| predicate(f)).collect();
        out.push_str(&format!("\n### {label} (priority {weight})\n"));
        if matched.is_empty() {
            out.push_str("(none)\n");
            continue;
        }
        for (i, f) in matched.iter().enumerate() {
            if i >= MAX_PER_SECTION {
                let remaining = matched.len() - MAX_PER_SECTION;
                out.push_str(&format!("- {remaining} more\n"));
                break;
            }
            let count_suffix = if f.count > 1 {
                format!(" (x{})", f.count)
            } else {
                String::new()
            };
            let loc_suffix = f
                .location
                .as_deref()
                .map(|l| format!(" — {l}"))
                .unwrap_or_default();
            out.push_str(&format!("- {}{loc_suffix}{count_suffix}\n", f.summary));
        }
    }

    // Custom category findings not in standard sections
    let custom: Vec<_> = findings
        .iter()
        .filter(|f| matches!(f.category, Category::Custom(_)))
        .collect();
    if !custom.is_empty() {
        out.push_str("\n### Other findings\n");
        for (i, f) in custom.iter().enumerate() {
            if i >= MAX_PER_SECTION {
                let remaining = custom.len() - MAX_PER_SECTION;
                out.push_str(&format!("- {remaining} more\n"));
                break;
            }
            out.push_str(&format!("- {}\n", f.summary));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(cat: Category, sev: Severity, summary: &str, loc: Option<&str>) -> Finding {
        Finding {
            category: cat,
            severity: sev,
            summary: summary.to_string(),
            location: loc.map(|s| s.to_string()),
            count: 1,
        }
    }

    #[test]
    fn digest_groups_by_category() {
        let findings = vec![
            finding(
                Category::Test,
                Severity::Error,
                "FAIL test_foo",
                Some("src/lib.rs:10"),
            ),
            finding(
                Category::Lint,
                Severity::Warning,
                "unused import",
                Some("src/app.rs:5"),
            ),
        ];
        let digest = build_digest(&findings, &PriorityWeights::default());
        assert!(digest.contains("Test failures"));
        assert!(digest.contains("Lint warnings"));
        assert!(digest.contains("FAIL test_foo"));
        assert!(digest.contains("unused import"));
    }

    #[test]
    fn digest_shows_count_for_grouped_findings() {
        let findings = vec![Finding {
            category: Category::Lint,
            severity: Severity::Warning,
            summary: "unused import".to_string(),
            location: Some("src/app.rs:5".to_string()),
            count: 3,
        }];
        let digest = build_digest(&findings, &PriorityWeights::default());
        assert!(digest.contains("(x3)"));
    }

    #[test]
    fn digest_caps_per_section() {
        let findings: Vec<_> = (0..10)
            .map(|i| {
                finding(
                    Category::Todo,
                    Severity::Info,
                    &format!("TODO item {i}"),
                    None,
                )
            })
            .collect();
        let digest = build_digest(&findings, &PriorityWeights::default());
        assert!(digest.contains("more"));
    }

    #[test]
    fn empty_findings_produces_clean_message() {
        let digest = build_digest(&[], &PriorityWeights::default());
        assert!(digest.contains("Codebase scan results"));
        assert!(digest.contains("No issues found"));
    }

    #[test]
    fn digest_shows_location() {
        let findings = vec![finding(
            Category::Lint,
            Severity::Error,
            "type mismatch",
            Some("src/main.rs:42"),
        )];
        let digest = build_digest(&findings, &PriorityWeights::default());
        assert!(digest.contains("src/main.rs:42"));
    }
}
