//! LSP diagnostics hook — auto-injects diagnostics after file-modifying tools.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use lsp_types::{Diagnostic, DiagnosticSeverity};

use super::{HookContext, PostToolHook};
use crate::agent::tools::ToolResult;

/// Appends LSP diagnostics to tool results that modified files.
pub struct LspDiagnosticsHook;

impl PostToolHook for LspDiagnosticsHook {
    fn applies(&self, result: &ToolResult) -> bool {
        !result.is_error && !result.files_modified.is_empty()
    }

    fn run<'a>(
        &'a self,
        result: &'a mut ToolResult,
        ctx: &'a mut HookContext<'_>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let lsp = match ctx.lsp_manager.as_mut() {
                Some(lsp) => lsp,
                None => return Ok(()), // No LSP available — silent no-op
            };

            // 1. Notify LSP servers of file changes
            for path in &result.files_modified {
                let _ = lsp.notify_file_changed(path).await;
            }

            // 2. Wait for diagnostics to settle
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;

            // 3. Collect diagnostics for modified files
            let mut diags: HashMap<PathBuf, Vec<Diagnostic>> = HashMap::new();
            for path in &result.files_modified {
                if let Ok(file_diags) = lsp.get_diagnostics(path).await
                    && !file_diags.is_empty()
                {
                    diags.insert(path.clone(), file_diags);
                }
            }

            // 4. Collect diagnostics from up to 5 other affected files
            let all_cached = lsp.all_diagnostics().await;
            let mut affected_count = 0;
            for (path, file_diags) in &all_cached {
                if affected_count >= 5 {
                    break;
                }
                if !result.files_modified.contains(path) && !file_diags.is_empty() {
                    diags.insert(path.clone(), file_diags.clone());
                    affected_count += 1;
                }
            }

            // 5. Append formatted diagnostics to tool output
            result.output.push_str(&format_diagnostics_section(&diags));

            Ok(())
        })
    }
}

fn severity_label(severity: Option<DiagnosticSeverity>) -> &'static str {
    match severity {
        Some(DiagnosticSeverity::ERROR) => "ERROR",
        Some(DiagnosticSeverity::WARNING) => "WARNING",
        Some(DiagnosticSeverity::INFORMATION) => "INFO",
        Some(DiagnosticSeverity::HINT) => "HINT",
        _ => "UNKNOWN",
    }
}

/// Format diagnostics into the appended output section.
fn format_diagnostics_section(all_diags: &HashMap<PathBuf, Vec<Diagnostic>>) -> String {
    if all_diags.is_empty() || all_diags.values().all(|d| d.is_empty()) {
        return "\n--- diagnostics ---\nAll clean.".to_string();
    }

    let mut lines = vec![String::new(), "--- diagnostics ---".to_string()];

    // Sort files for deterministic output
    let mut files: Vec<_> = all_diags
        .iter()
        .filter(|(_, diags)| !diags.is_empty())
        .collect();
    files.sort_by_key(|(path, _)| (*path).clone());

    let mut total_lines = 0;
    let mut truncated_count = 0;
    let mut truncated_files = 0;

    for (path, diags) in &files {
        if total_lines >= 50 {
            truncated_files += 1;
            truncated_count += diags.len();
            continue;
        }

        let errors = diags
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .count();
        let warnings = diags
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::WARNING))
            .count();
        let others = diags.len() - errors - warnings;

        let summary = match (errors, warnings, others) {
            (e, 0, 0) => format!("{e} error{}", if e == 1 { "" } else { "s" }),
            (0, w, 0) => format!("{w} warning{}", if w == 1 { "" } else { "s" }),
            (e, w, 0) => format!(
                "{e} error{}, {w} warning{}",
                if e == 1 { "" } else { "s" },
                if w == 1 { "" } else { "s" }
            ),
            (e, w, o) => {
                let mut parts = Vec::new();
                if e > 0 {
                    parts.push(format!("{e} error{}", if e == 1 { "" } else { "s" }));
                }
                if w > 0 {
                    parts.push(format!("{w} warning{}", if w == 1 { "" } else { "s" }));
                }
                if o > 0 {
                    parts.push(format!("{o} other"));
                }
                parts.join(", ")
            }
        };

        lines.push(format!("{}: {summary}", path.display()));
        total_lines += 1;

        // Sort by severity (errors first) then by line number
        let mut sorted_diags: Vec<_> = diags.iter().collect();
        sorted_diags.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then(a.range.start.line.cmp(&b.range.start.line))
        });

        for diag in sorted_diags {
            if total_lines >= 50 {
                truncated_count += 1;
                continue;
            }
            let severity = severity_label(diag.severity);
            let line = diag.range.start.line + 1;
            let col = diag.range.start.character + 1;
            lines.push(format!(
                "  {severity} line {line}:{col} -- {}",
                diag.message
            ));
            total_lines += 1;
        }
    }

    if truncated_count > 0 {
        lines.push(format!(
            "... and {truncated_count} more diagnostic{} across {truncated_files} file{}",
            if truncated_count == 1 { "" } else { "s" },
            if truncated_files == 1 { "" } else { "s" },
        ));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    #[test]
    fn hook_applies_when_files_modified() {
        let hook = LspDiagnosticsHook;
        let result = ToolResult {
            tool_use_id: String::new(),
            output: "Wrote file".to_string(),
            is_error: false,
            tool_name: Some("write_file".to_string()),
            file_path: Some("/tmp/test.rs".to_string()),
            files_modified: vec![PathBuf::from("/tmp/test.rs")],
            lines_added: 0,
            lines_removed: 0,
        };
        assert!(hook.applies(&result));
    }

    #[test]
    fn hook_skips_when_no_files_modified() {
        let hook = LspDiagnosticsHook;
        let result = ToolResult {
            tool_use_id: String::new(),
            output: "Listed directory".to_string(),
            is_error: false,
            tool_name: Some("list_directory".to_string()),
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        };
        assert!(!hook.applies(&result));
    }

    #[test]
    fn hook_skips_on_error_results() {
        let hook = LspDiagnosticsHook;
        let result = ToolResult {
            tool_use_id: String::new(),
            output: "Error writing".to_string(),
            is_error: true,
            tool_name: Some("write_file".to_string()),
            file_path: Some("/tmp/test.rs".to_string()),
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        };
        assert!(!hook.applies(&result));
    }

    #[test]
    fn format_diagnostics_output_clean() {
        let output = format_diagnostics_section(&HashMap::new());
        assert_eq!(output, "\n--- diagnostics ---\nAll clean.");
    }

    #[test]
    fn format_diagnostics_output_with_errors() {
        let mut diags = HashMap::new();
        diags.insert(
            PathBuf::from("src/main.rs"),
            vec![Diagnostic {
                range: Range {
                    start: Position {
                        line: 41,
                        character: 9,
                    },
                    end: Position {
                        line: 41,
                        character: 15,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: "expected `usize`, found `String`".to_string(),
                ..Default::default()
            }],
        );
        let output = format_diagnostics_section(&diags);
        assert!(output.contains("--- diagnostics ---"));
        assert!(output.contains("src/main.rs: 1 error"));
        assert!(output.contains("ERROR line 42:10"));
    }

    #[test]
    fn format_diagnostics_multiple_files() {
        let mut diags = HashMap::new();
        diags.insert(
            PathBuf::from("src/a.rs"),
            vec![Diagnostic {
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 5,
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                message: "error in a".to_string(),
                ..Default::default()
            }],
        );
        diags.insert(
            PathBuf::from("src/b.rs"),
            vec![Diagnostic {
                range: Range {
                    start: Position {
                        line: 10,
                        character: 0,
                    },
                    end: Position {
                        line: 10,
                        character: 5,
                    },
                },
                severity: Some(DiagnosticSeverity::WARNING),
                message: "warning in b".to_string(),
                ..Default::default()
            }],
        );
        let output = format_diagnostics_section(&diags);
        assert!(output.contains("src/a.rs: 1 error"));
        assert!(output.contains("src/b.rs: 1 warning"));
    }

    #[test]
    fn format_diagnostics_mixed_severities() {
        let mut diags = HashMap::new();
        diags.insert(
            PathBuf::from("src/main.rs"),
            vec![
                Diagnostic {
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 5,
                        },
                    },
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: "an error".to_string(),
                    ..Default::default()
                },
                Diagnostic {
                    range: Range {
                        start: Position {
                            line: 5,
                            character: 0,
                        },
                        end: Position {
                            line: 5,
                            character: 5,
                        },
                    },
                    severity: Some(DiagnosticSeverity::WARNING),
                    message: "a warning".to_string(),
                    ..Default::default()
                },
            ],
        );
        let output = format_diagnostics_section(&diags);
        assert!(output.contains("1 error, 1 warning"));
    }
}
