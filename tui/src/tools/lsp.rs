//! LSP tool — semantic code navigation via language servers.

use std::path::{Path, PathBuf};

use anyhow::Result;
use lsp_types::{DocumentSymbol, Location, SymbolInformation, SymbolKind};

use crate::agent::tools::ToolResult;
use crate::lsp::LspManager;

pub async fn execute(
    input: &serde_json::Value,
    lsp_manager: &mut LspManager,
) -> Result<ToolResult> {
    let operation = input
        .get("operation")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing 'operation' parameter"))?;

    let path = input
        .get("path")
        .or_else(|| input.get("file_path"))
        .and_then(|v| v.as_str());

    let line = input.get("line").and_then(|v| v.as_u64()).map(|v| v as u32);

    let character = input
        .get("character")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let query = input.get("query").and_then(|v| v.as_str());

    let output = match operation {
        "goToDefinition" => {
            let (path, line, character) = require_position(path, line, character, operation)?;
            match lsp_manager
                .goto_definition(Path::new(path), line, character)
                .await
            {
                Ok(locs) => format_locations(&locs, "definition", 30),
                Err(e) => return Ok(error_result(e)),
            }
        }
        "findReferences" => {
            let (path, line, character) = require_position(path, line, character, operation)?;
            match lsp_manager
                .find_references(Path::new(path), line, character)
                .await
            {
                Ok(locs) => format_locations(&locs, "reference", 30),
                Err(e) => return Ok(error_result(e)),
            }
        }
        "hover" => {
            let (path, line, character) = require_position(path, line, character, operation)?;
            match lsp_manager.hover(Path::new(path), line, character).await {
                Ok(Some(text)) => text,
                Ok(None) => format!("No hover info at {path}:{line}:{character}"),
                Err(e) => return Ok(error_result(e)),
            }
        }
        "goToImplementation" => {
            let (path, line, character) = require_position(path, line, character, operation)?;
            match lsp_manager
                .goto_implementation(Path::new(path), line, character)
                .await
            {
                Ok(locs) => format_locations(&locs, "implementation", 30),
                Err(e) => return Ok(error_result(e)),
            }
        }
        "documentSymbol" => {
            let path =
                path.ok_or_else(|| anyhow::anyhow!("documentSymbol requires 'path' parameter"))?;
            match lsp_manager.document_symbol(Path::new(path)).await {
                Ok(syms) => {
                    if syms.is_empty() {
                        format!("No symbols in {path}")
                    } else {
                        let header = format!("Symbols in {path}:");
                        let body = format_document_symbols(path, &syms, 1);
                        format!("{header}\n{body}")
                    }
                }
                Err(e) => return Ok(error_result(e)),
            }
        }
        "workspaceSymbol" => {
            let query = query
                .ok_or_else(|| anyhow::anyhow!("workspaceSymbol requires 'query' parameter"))?;
            match lsp_manager.workspace_symbol(query).await {
                Ok(syms) => format_workspace_symbols(query, &syms, 15),
                Err(e) => return Ok(error_result(e)),
            }
        }
        _ => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: format!(
                    "Unknown LSP operation: '{operation}'. Supported: goToDefinition, findReferences, hover, goToImplementation, documentSymbol, workspaceSymbol"
                ),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

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

/// Validate that path, line, character are all present for position-based operations.
fn require_position<'a>(
    path: Option<&'a str>,
    line: Option<u32>,
    character: Option<u32>,
    operation: &str,
) -> Result<(&'a str, u32, u32)> {
    let path = path.ok_or_else(|| anyhow::anyhow!("{operation} requires 'path' parameter"))?;
    let line =
        line.ok_or_else(|| anyhow::anyhow!("{operation} requires 'line' parameter (1-based)"))?;
    let character = character
        .ok_or_else(|| anyhow::anyhow!("{operation} requires 'character' parameter (1-based)"))?;
    Ok((path, line, character))
}

fn error_result(e: anyhow::Error) -> ToolResult {
    ToolResult {
        tool_use_id: String::new(),
        output: format!("{e}"),
        is_error: true,
        tool_name: None,
        file_path: None,
        files_modified: vec![],
        lines_added: 0,
        lines_removed: 0,
    }
}

/// Read a single line from a file (1-based line number). Returns trimmed content.
fn read_line_snippet(path: &Path, line: u32) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let line_str = content.lines().nth((line.saturating_sub(1)) as usize)?;
    let trimmed = line_str.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Convert an LSP file URI to a relative path (relative to cwd).
fn uri_to_display_path(uri: &lsp_types::Uri) -> String {
    let uri_str = uri.as_str();
    if let Some(path_str) = uri_str.strip_prefix("file://") {
        if let Ok(cwd) = std::env::current_dir()
            && let Ok(rel) = PathBuf::from(path_str).strip_prefix(&cwd)
        {
            return rel.display().to_string();
        }
        path_str.to_string()
    } else {
        uri_str.to_string()
    }
}

fn format_locations(locations: &[Location], label: &str, cap: usize) -> String {
    if locations.is_empty() {
        return format!("No {label} found.");
    }

    let count = locations.len();
    let mut lines = vec![format!("{count} {label} found:")];

    for loc in locations.iter().take(cap) {
        let path = uri_to_display_path(&loc.uri);
        let line = loc.range.start.line + 1;
        let col = loc.range.start.character + 1;
        let abs_path = if let Some(p) = loc.uri.as_str().strip_prefix("file://") {
            PathBuf::from(p)
        } else {
            PathBuf::from(&path)
        };
        let snippet = read_line_snippet(&abs_path, line)
            .map(|s| format!(" \u{2014} {s}"))
            .unwrap_or_default();
        lines.push(format!("  {path}:{line}:{col}{snippet}"));
    }

    if count > cap {
        lines.push(format!("  ... and {} more", count - cap));
    }

    lines.join("\n")
}

fn symbol_kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE => "mod",
        SymbolKind::NAMESPACE => "namespace",
        SymbolKind::PACKAGE => "package",
        SymbolKind::CLASS => "class",
        SymbolKind::METHOD => "method",
        SymbolKind::PROPERTY => "property",
        SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "constructor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "interface",
        SymbolKind::FUNCTION => "fn",
        SymbolKind::VARIABLE => "var",
        SymbolKind::CONSTANT => "const",
        SymbolKind::STRING => "string",
        SymbolKind::NUMBER => "number",
        SymbolKind::BOOLEAN => "bool",
        SymbolKind::ARRAY => "array",
        SymbolKind::OBJECT => "object",
        SymbolKind::KEY => "key",
        SymbolKind::NULL => "null",
        SymbolKind::ENUM_MEMBER => "enum_member",
        SymbolKind::STRUCT => "struct",
        SymbolKind::EVENT => "event",
        SymbolKind::OPERATOR => "operator",
        SymbolKind::TYPE_PARAMETER => "type_param",
        _ => "symbol",
    }
}

fn format_document_symbols(_path: &str, symbols: &[DocumentSymbol], indent: usize) -> String {
    let mut lines = Vec::new();
    for sym in symbols {
        let kind = symbol_kind_label(sym.kind);
        let line = sym.range.start.line + 1;
        let prefix = "  ".repeat(indent);
        lines.push(format!("{prefix}{kind} {} (line {line})", sym.name));
        if let Some(children) = &sym.children {
            lines.push(format_document_symbols_inner(children, indent + 1));
        }
    }
    lines.join("\n")
}

fn format_document_symbols_inner(symbols: &[DocumentSymbol], indent: usize) -> String {
    let mut lines = Vec::new();
    for sym in symbols {
        let kind = symbol_kind_label(sym.kind);
        let line = sym.range.start.line + 1;
        let prefix = "  ".repeat(indent);
        lines.push(format!("{prefix}{kind} {} (line {line})", sym.name));
        if let Some(children) = &sym.children {
            lines.push(format_document_symbols_inner(children, indent + 1));
        }
    }
    lines.join("\n")
}

fn format_workspace_symbols(query: &str, symbols: &[SymbolInformation], cap: usize) -> String {
    if symbols.is_empty() {
        return format!("No workspace symbols matching \"{query}\".");
    }

    let count = symbols.len();
    let mut lines = vec![format!("Workspace symbols matching \"{query}\":")];

    for sym in symbols.iter().take(cap) {
        let kind = symbol_kind_label(sym.kind);
        let path = uri_to_display_path(&sym.location.uri);
        let line = sym.location.range.start.line + 1;
        lines.push(format!("  {kind} {} \u{2014} {path}:{line}", sym.name));
    }

    if count > cap {
        lines.push(format!("  ... and {} more", count - cap));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    #[test]
    fn read_line_snippet_returns_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "  fn main() {\n    println!(\"hi\");\n  }\n").unwrap();
        assert_eq!(read_line_snippet(&file, 1), Some("fn main() {".to_string()));
        assert_eq!(
            read_line_snippet(&file, 2),
            Some("println!(\"hi\");".to_string())
        );
        assert_eq!(read_line_snippet(&file, 99), None);
    }

    #[test]
    fn format_locations_empty() {
        assert_eq!(
            format_locations(&[], "definition", 30),
            "No definition found."
        );
    }

    #[test]
    fn format_locations_single() {
        let loc = Location {
            uri: "file:///nonexistent/test.rs".parse().unwrap(),
            range: Range {
                start: Position {
                    line: 4,
                    character: 2,
                },
                end: Position {
                    line: 4,
                    character: 10,
                },
            },
        };
        let output = format_locations(&[loc], "definition", 30);
        assert!(output.contains("1 definition found:"));
        assert!(output.contains("5:3")); // 0-based → 1-based
    }

    #[test]
    fn format_locations_truncated() {
        let locs: Vec<Location> = (0..5)
            .map(|i| Location {
                uri: "file:///test.rs".parse().unwrap(),
                range: Range {
                    start: Position {
                        line: i,
                        character: 0,
                    },
                    end: Position {
                        line: i,
                        character: 5,
                    },
                },
            })
            .collect();
        let output = format_locations(&locs, "reference", 3);
        assert!(output.contains("5 reference found:"));
        assert!(output.contains("... and 2 more"));
    }

    #[test]
    fn symbol_kind_labels() {
        assert_eq!(symbol_kind_label(SymbolKind::FUNCTION), "fn");
        assert_eq!(symbol_kind_label(SymbolKind::STRUCT), "struct");
        assert_eq!(symbol_kind_label(SymbolKind::METHOD), "method");
    }

    #[test]
    #[allow(deprecated)]
    fn format_document_symbols_nested() {
        let symbols = vec![DocumentSymbol {
            name: "MyStruct".to_string(),
            detail: None,
            kind: SymbolKind::STRUCT,
            range: Range {
                start: Position {
                    line: 9,
                    character: 0,
                },
                end: Position {
                    line: 20,
                    character: 1,
                },
            },
            selection_range: Range {
                start: Position {
                    line: 9,
                    character: 0,
                },
                end: Position {
                    line: 9,
                    character: 8,
                },
            },
            children: Some(vec![DocumentSymbol {
                name: "new".to_string(),
                detail: None,
                kind: SymbolKind::FUNCTION,
                range: Range {
                    start: Position {
                        line: 11,
                        character: 4,
                    },
                    end: Position {
                        line: 15,
                        character: 5,
                    },
                },
                selection_range: Range {
                    start: Position {
                        line: 11,
                        character: 4,
                    },
                    end: Position {
                        line: 11,
                        character: 7,
                    },
                },
                children: None,
                tags: None,
                deprecated: None,
            }]),
            tags: None,
            deprecated: None,
        }];
        let output = format_document_symbols("test.rs", &symbols, 1);
        assert!(output.contains("  struct MyStruct (line 10)"));
        assert!(output.contains("    fn new (line 12)"));
    }

    #[test]
    fn format_workspace_symbols_empty() {
        let output = format_workspace_symbols("Foo", &[], 15);
        assert!(output.contains("No workspace symbols matching \"Foo\""));
    }
}
