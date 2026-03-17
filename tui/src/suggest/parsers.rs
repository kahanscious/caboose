//! Scan output parsers — clippy JSON, cargo test, generic, TODO grep, git churn.

use crate::suggest::priority::{Category, Finding, Severity};

/// Parse cargo clippy --message-format=json output into findings.
pub fn parse_clippy_json(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if val.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(msg) = val.get("message") else {
            continue;
        };
        let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("warning");
        let text = msg
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        let location = msg
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|a| a.first())
            .and_then(|span| {
                let file = span.get("file_name")?.as_str()?;
                let line = span.get("line_start")?.as_u64()?;
                Some(format!("{file}:{line}"))
            });

        findings.push(Finding {
            category: Category::Lint,
            severity: match level {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                _ => Severity::Info,
            },
            summary: text.to_string(),
            location,
            count: 1,
        });
    }
    findings
}

/// Parse cargo test output for failures.
pub fn parse_cargo_test(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("test ") && line.ends_with("... FAILED") {
            let name = line
                .strip_prefix("test ")
                .and_then(|s| s.strip_suffix(" ... FAILED"))
                .unwrap_or(line);
            findings.push(Finding {
                category: Category::Test,
                severity: Severity::Error,
                summary: format!("FAIL {name}"),
                location: None,
                count: 1,
            });
        }
    }
    findings
}

/// Generic fallback parser — extract lines containing "error" or "warning".
pub fn parse_generic(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut error_count = 0usize;
    let mut warning_count = 0usize;
    for line in output.lines().take(2000) {
        let lower = line.to_lowercase();
        if lower.contains("error") {
            error_count += 1;
            if findings.len() < 5 {
                findings.push(Finding {
                    category: Category::Custom("generic".to_string()),
                    severity: Severity::Error,
                    summary: line.trim().to_string(),
                    location: None,
                    count: 1,
                });
            }
        } else if lower.contains("warning") {
            warning_count += 1;
            if findings.len() < 5 {
                findings.push(Finding {
                    category: Category::Custom("generic".to_string()),
                    severity: Severity::Warning,
                    summary: line.trim().to_string(),
                    location: None,
                    count: 1,
                });
            }
        }
    }
    let total = error_count + warning_count;
    if total > 5 {
        findings.push(Finding {
            category: Category::Custom("generic".to_string()),
            severity: Severity::Info,
            summary: format!(
                "{} more issues in output ({error_count} errors, {warning_count} warnings)",
                total - 5
            ),
            location: None,
            count: 1,
        });
    }
    findings
}

/// Parse grep output for TODO/FIXME/HACK markers.
/// Expects lines in format: "path/to/file.rs:42: // TODO: something"
pub fn parse_todo_grep(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (location, text) = if let Some((loc, rest)) = line.split_once(": ") {
            let marker = if rest.contains("FIXME") {
                "FIXME"
            } else if rest.contains("HACK") {
                "HACK"
            } else {
                "TODO"
            };
            let summary = rest
                .trim()
                .trim_start_matches("//")
                .trim_start_matches('#')
                .trim();
            (Some(loc.to_string()), format!("{marker}: {summary}"))
        } else {
            (None, line.to_string())
        };

        findings.push(Finding {
            category: Category::Todo,
            severity: if text.starts_with("FIXME") || text.starts_with("HACK") {
                Severity::Warning
            } else {
                Severity::Info
            },
            summary: text,
            location,
            count: 1,
        });
    }
    findings
}

/// Parse git log --name-only output into churn findings.
/// Input: raw file paths, one per line (may have blanks between commits).
pub fn parse_git_churn(output: &str) -> Vec<Finding> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked.truncate(10);

    ranked
        .into_iter()
        .map(|(file, count)| Finding {
            category: Category::Churn,
            severity: Severity::Info,
            summary: format!("{file} ({count} commits)"),
            location: Some(file),
            count,
        })
        .collect()
}

/// Dispatch to the appropriate parser based on scan category.
pub fn parse_scan_output(category: &str, output: &str) -> Vec<Finding> {
    match category {
        "lint" => parse_clippy_json(output),
        "test" => parse_cargo_test(output),
        "todo" => parse_todo_grep(output),
        "churn" => parse_git_churn(output),
        _ => parse_generic(output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- clippy JSON parser ---

    #[test]
    fn parse_clippy_warning() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused import: `std::io`","spans":[{"file_name":"src/app.rs","line_start":12}]}}"#;
        let findings = parse_clippy_json(json_line);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::Lint);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].summary.contains("unused import"));
        assert_eq!(findings[0].location.as_deref(), Some("src/app.rs:12"));
    }

    #[test]
    fn parse_clippy_error() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","spans":[{"file_name":"src/main.rs","line_start":5}]}}"#;
        let findings = parse_clippy_json(json_line);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
    }

    #[test]
    fn parse_clippy_skips_non_compiler_messages() {
        let json_line = r#"{"reason":"build-finished","success":true}"#;
        let findings = parse_clippy_json(json_line);
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_clippy_multiple_lines() {
        let output = format!(
            "{}\n{}\n{}",
            r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable","spans":[{"file_name":"src/a.rs","line_start":1}]}}"#,
            r#"{"reason":"build-finished","success":true}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","message":"dead code","spans":[{"file_name":"src/b.rs","line_start":2}]}}"#,
        );
        let findings = parse_clippy_json(&output);
        assert_eq!(findings.len(), 2);
    }

    // --- cargo test parser ---

    #[test]
    fn parse_cargo_test_failures() {
        let output =
            "test agent::tests::circuit_breaker ... ok\ntest tools::tests::shell_timeout ... FAILED\n";
        let findings = parse_cargo_test(output);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::Test);
        assert!(findings[0].summary.contains("shell_timeout"));
    }

    #[test]
    fn parse_cargo_test_all_pass() {
        let output = "test foo ... ok\ntest bar ... ok\n";
        let findings = parse_cargo_test(output);
        assert!(findings.is_empty());
    }

    // --- generic parser ---

    #[test]
    fn parse_generic_caps_at_five_findings() {
        let lines: Vec<String> = (0..20).map(|i| format!("error: problem {i}")).collect();
        let output = lines.join("\n");
        let findings = parse_generic(&output);
        // 5 individual + 1 overflow summary
        assert_eq!(findings.len(), 6);
        assert!(findings.last().unwrap().summary.contains("more issues"));
    }

    // --- TODO grep parser ---

    #[test]
    fn parse_todo_grep_extracts_markers() {
        let output =
            "src/app.rs:42: // TODO: handle edge case\nsrc/lib.rs:10: // FIXME: race condition\n";
        let findings = parse_todo_grep(output);
        assert_eq!(findings.len(), 2);
        assert!(findings[0].summary.starts_with("TODO"));
        assert!(findings[1].summary.starts_with("FIXME"));
        assert_eq!(findings[1].severity, Severity::Warning);
    }

    // --- git churn parser ---

    #[test]
    fn parse_git_churn_ranks_by_frequency() {
        let output = "src/app.rs\nsrc/lib.rs\nsrc/app.rs\nsrc/app.rs\nsrc/lib.rs\n";
        let findings = parse_git_churn(output);
        assert_eq!(findings[0].count, 3); // app.rs
        assert_eq!(findings[1].count, 2); // lib.rs
    }

    // --- dispatch ---

    #[test]
    fn dispatch_lint_uses_clippy_parser() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"warning","message":"test","spans":[]}}"#;
        let findings = parse_scan_output("lint", json_line);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::Lint);
    }

    #[test]
    fn dispatch_unknown_uses_generic() {
        let findings = parse_scan_output("something_else", "error: bad\n");
        assert_eq!(findings.len(), 1);
    }
}
