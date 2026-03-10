//! Diagnostics tool — exposes LSP diagnostics to the agent.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use lsp_types::DiagnosticSeverity;

use crate::agent::tools::ToolResult;
use crate::lsp::LspManager;

fn severity_label(severity: Option<DiagnosticSeverity>) -> &'static str {
    match severity {
        Some(DiagnosticSeverity::ERROR) => "ERROR",
        Some(DiagnosticSeverity::WARNING) => "WARNING",
        Some(DiagnosticSeverity::INFORMATION) => "INFO",
        Some(DiagnosticSeverity::HINT) => "HINT",
        _ => "UNKNOWN",
    }
}

pub async fn execute(
    input: &serde_json::Value,
    lsp_manager: &mut LspManager,
) -> Result<ToolResult> {
    let path = input
        .get("path")
        .or_else(|| input.get("file_path"))
        .and_then(|v| v.as_str());

    match path {
        Some(p) => execute_single(p, lsp_manager).await,
        None => execute_workspace(lsp_manager).await,
    }
}

async fn execute_single(path: &str, lsp_manager: &mut LspManager) -> Result<ToolResult> {
    let file_path = std::path::Path::new(path);

    match lsp_manager.get_diagnostics(file_path).await {
        Ok(diagnostics) => {
            if diagnostics.is_empty() {
                return Ok(ToolResult {
                    tool_use_id: String::new(),
                    output: format!("{path}: no diagnostics (clean)"),
                    is_error: false,
                    tool_name: None,
                    file_path: None,
                    files_modified: vec![],
                    lines_added: 0,
                    lines_removed: 0,
                });
            }

            let count = diagnostics.len();
            let mut lines = vec![format!(
                "{path}: {count} diagnostic{}",
                if count == 1 { "" } else { "s" }
            )];
            lines.push(String::new());

            for diag in &diagnostics {
                let severity = severity_label(diag.severity);
                let line = diag.range.start.line + 1;
                let col = diag.range.start.character + 1;
                let code = diag
                    .code
                    .as_ref()
                    .map(|c| match c {
                        lsp_types::NumberOrString::Number(n) => format!(" ({n})"),
                        lsp_types::NumberOrString::String(s) => format!(" ({s})"),
                    })
                    .unwrap_or_default();

                lines.push(format!(
                    "{severity} line {line}:{col} -- {}{code}",
                    diag.message
                ));
            }

            Ok(ToolResult {
                tool_use_id: String::new(),
                output: lines.join("\n"),
                is_error: false,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            })
        }
        Err(e) => Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("{e}"),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        }),
    }
}

async fn execute_workspace(lsp_manager: &mut LspManager) -> Result<ToolResult> {
    let all_diags = lsp_manager.all_diagnostics().await;
    let output = format_workspace_output(&all_diags);
    Ok(ToolResult {
        tool_use_id: String::new(),
        output,
        is_error: false,
        tool_name: None,
        file_path: None,
        files_modified: vec![],
        lines_added: 0,
        lines_removed: 0,
    })
}

fn format_workspace_output(all_diags: &HashMap<PathBuf, Vec<lsp_types::Diagnostic>>) -> String {
    let non_empty: Vec<_> = all_diags.iter().filter(|(_, d)| !d.is_empty()).collect();

    if non_empty.is_empty() {
        return "No diagnostics across workspace (all clean).".to_string();
    }

    let mut lines = Vec::new();
    let total_diags: usize = non_empty.iter().map(|(_, d)| d.len()).sum();
    lines.push(format!(
        "Workspace: {} diagnostic{} across {} file{}",
        total_diags,
        if total_diags == 1 { "" } else { "s" },
        non_empty.len(),
        if non_empty.len() == 1 { "" } else { "s" },
    ));
    lines.push(String::new());

    let mut files: Vec<_> = non_empty;
    files.sort_by_key(|(path, _)| (*path).clone());

    let mut total_lines = 0;
    let mut remaining_diags = 0;
    let mut remaining_files = 0;

    for (path, diags) in &files {
        if total_lines >= 50 {
            remaining_files += 1;
            remaining_diags += diags.len();
            continue;
        }

        let count = diags.len();
        lines.push(format!(
            "{}: {count} diagnostic{}",
            path.display(),
            if count == 1 { "" } else { "s" }
        ));
        total_lines += 1;

        let mut sorted: Vec<_> = diags.iter().collect();
        sorted.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then(a.range.start.line.cmp(&b.range.start.line))
        });

        for diag in sorted {
            if total_lines >= 50 {
                remaining_diags += 1;
                continue;
            }
            let severity = severity_label(diag.severity);
            let line = diag.range.start.line + 1;
            let col = diag.range.start.character + 1;
            let code = diag
                .code
                .as_ref()
                .map(|c| match c {
                    lsp_types::NumberOrString::Number(n) => format!(" ({n})"),
                    lsp_types::NumberOrString::String(s) => format!(" ({s})"),
                })
                .unwrap_or_default();
            lines.push(format!(
                "  {severity} line {line}:{col} -- {}{code}",
                diag.message
            ));
            total_lines += 1;
        }
    }

    if remaining_diags > 0 {
        lines.push(format!(
            "... and {remaining_diags} more across {remaining_files} file{}",
            if remaining_files == 1 { "" } else { "s" }
        ));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_labels() {
        assert_eq!(severity_label(Some(DiagnosticSeverity::ERROR)), "ERROR");
        assert_eq!(severity_label(Some(DiagnosticSeverity::WARNING)), "WARNING");
        assert_eq!(
            severity_label(Some(DiagnosticSeverity::INFORMATION)),
            "INFO"
        );
        assert_eq!(severity_label(Some(DiagnosticSeverity::HINT)), "HINT");
        assert_eq!(severity_label(None), "UNKNOWN");
    }

    #[test]
    fn format_workspace_diagnostics_empty() {
        let output = format_workspace_output(&HashMap::new());
        assert!(output.contains("No diagnostics across workspace"));
    }

    #[test]
    fn format_workspace_diagnostics_with_entries() {
        use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
        let mut diags = HashMap::new();
        diags.insert(
            PathBuf::from("src/main.rs"),
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
                message: "test error".to_string(),
                ..Default::default()
            }],
        );
        let output = format_workspace_output(&diags);
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("ERROR"));
        assert!(output.contains("Workspace: 1 diagnostic across 1 file"));
    }

    #[test]
    fn format_workspace_diagnostics_multiple_files() {
        use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};
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
                message: "error a".to_string(),
                ..Default::default()
            }],
        );
        diags.insert(
            PathBuf::from("src/b.rs"),
            vec![Diagnostic {
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
                message: "warning b".to_string(),
                ..Default::default()
            }],
        );
        let output = format_workspace_output(&diags);
        assert!(output.contains("Workspace: 2 diagnostics across 2 files"));
        assert!(output.contains("src/a.rs"));
        assert!(output.contains("src/b.rs"));
    }
}
