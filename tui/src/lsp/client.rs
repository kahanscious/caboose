//! LSP client — manages a single language server connection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context, Result};
use lsp_types::{
    ClientCapabilities,
    Diagnostic,
    DidOpenTextDocumentParams,
    DocumentSymbol,
    DocumentSymbolClientCapabilities,
    DocumentSymbolParams,
    // Navigation types
    GotoCapability,
    HoverClientCapabilities,
    InitializeParams,
    Location,
    Position,
    PublishDiagnosticsClientCapabilities,
    PublishDiagnosticsParams,
    ReferenceContext,
    ReferenceParams,
    SymbolInformation,
    TextDocumentClientCapabilities,
    TextDocumentIdentifier,
    TextDocumentItem,
    TextDocumentPositionParams,
    Uri,
    WorkspaceSymbolParams,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};

use super::types::*;

/// Convert a filesystem path to a `file://` URI suitable for LSP.
fn path_to_uri(path: &Path) -> Result<Uri> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // On Windows, paths like C:\foo become /C:/foo in file URIs.
    let path_str = abs.to_string_lossy().replace('\\', "/");
    let uri_str = if path_str.starts_with('/') {
        format!("file://{path_str}")
    } else {
        format!("file:///{path_str}")
    };
    uri_str
        .parse::<Uri>()
        .map_err(|e| anyhow::anyhow!("Invalid file URI '{}': {}", uri_str, e))
}

/// A connection to a single LSP server.
pub struct LspClient {
    language: String,
    state: ServerState,
    writer: Arc<Mutex<tokio::process::ChildStdin>>,
    next_id: AtomicI64,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<JsonRpcMessage>>>>,
    diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    open_files: Mutex<HashMap<PathBuf, i32>>,
    _reader_handle: tokio::task::JoinHandle<()>,
    _process: Child,
}

impl LspClient {
    /// Spawn a language server and perform the LSP initialize handshake.
    pub async fn start(
        language: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        workspace_root: &Path,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env.iter())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut process = cmd.spawn().with_context(|| {
            format!("Failed to spawn LSP server '{command}'. Is it installed and on PATH?")
        })?;

        let stdin = process.stdin.take().expect("stdin was piped");
        let stdout = process.stdout.take().expect("stdout was piped");

        let writer = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<JsonRpcMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let reader_pending = Arc::clone(&pending);
        let reader_diags = Arc::clone(&diagnostics);
        let reader_handle = tokio::spawn(async move {
            Self::reader_loop(stdout, reader_pending, reader_diags).await;
        });

        let mut client = Self {
            language: language.to_string(),
            state: ServerState::Starting,
            writer,
            next_id: AtomicI64::new(1),
            pending,
            diagnostics,
            open_files: Mutex::new(HashMap::new()),
            _reader_handle: reader_handle,
            _process: process,
        };

        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client.initialize(workspace_root),
        )
        .await
        .with_context(|| format!("LSP server '{command}' timed out during initialization"))??;

        Ok(client)
    }

    /// Background task: read JSON-RPC messages from stdout and route them.
    async fn reader_loop(
        stdout: tokio::process::ChildStdout,
        pending: Arc<Mutex<HashMap<i64, oneshot::Sender<JsonRpcMessage>>>>,
        diagnostics: Arc<Mutex<HashMap<String, Vec<Diagnostic>>>>,
    ) {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut content_length: Option<usize> = None;
            let mut header_buf = String::new();
            loop {
                header_buf.clear();
                match reader.read_line(&mut header_buf).await {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(_) => return,
                }
                let line = header_buf.trim();
                if line.is_empty() {
                    break;
                }
                if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                    content_length = len_str.trim().parse().ok();
                }
            }

            let Some(len) = content_length else {
                continue;
            };

            let mut body = vec![0u8; len];
            if tokio::io::AsyncReadExt::read_exact(&mut reader, &mut body)
                .await
                .is_err()
            {
                return;
            }

            let Ok(msg) = serde_json::from_slice::<JsonRpcMessage>(&body) else {
                continue;
            };

            if msg.is_response() {
                if let Some(id) = msg.id.as_ref().and_then(|n| n.as_i64()) {
                    let mut map = pending.lock().await;
                    if let Some(sender) = map.remove(&id) {
                        let _ = sender.send(msg);
                    }
                }
            } else if msg.is_notification()
                && msg.method.as_deref() == Some("textDocument/publishDiagnostics")
                && let Some(params) = msg.params
                && let Ok(diag_params) = serde_json::from_value::<PublishDiagnosticsParams>(params)
            {
                let uri = diag_params.uri.as_str().to_string();
                let mut map = diagnostics.lock().await;
                map.insert(uri, diag_params.diagnostics);
            }
        }
    }

    /// Send LSP initialize request and initialized notification.
    #[allow(deprecated)] // root_uri is deprecated in lsp-types but still widely used
    async fn initialize(&mut self, workspace_root: &Path) -> Result<()> {
        self.state = ServerState::Initializing;

        let root_uri = path_to_uri(workspace_root)?;

        let params = InitializeParams {
            root_uri: Some(root_uri),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                        related_information: Some(true),
                        ..Default::default()
                    }),
                    definition: Some(GotoCapability::default()),
                    implementation: Some(GotoCapability::default()),
                    references: Some(lsp_types::DynamicRegistrationClientCapabilities::default()),
                    hover: Some(HoverClientCapabilities::default()),
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        hierarchical_document_symbol_support: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let _result = self
            .send_request("initialize", Some(serde_json::to_value(params)?))
            .await?;
        self.send_notification("initialized", Some(serde_json::json!({})))
            .await?;
        self.state = ServerState::Ready;
        Ok(())
    }

    /// Send a JSON-RPC request and wait for the response.
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<JsonRpcMessage> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest::new(id, method, params);
        let body = serde_json::to_vec(&req)?;
        let msg = encode_message(&body);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        {
            let mut w = self.writer.lock().await;
            w.write_all(&msg).await?;
            w.flush().await?;
        }

        let response = rx
            .await
            .context("LSP server disconnected while waiting for response")?;
        if let Some(err) = &response.error {
            anyhow::bail!("LSP error {}: {}", err.code, err.message);
        }
        Ok(response)
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification::new(method, params);
        let body = serde_json::to_vec(&notif)?;
        let msg = encode_message(&body);
        let mut w = self.writer.lock().await;
        w.write_all(&msg).await?;
        w.flush().await?;
        Ok(())
    }

    /// Open a file in the language server (textDocument/didOpen).
    pub async fn open_file(&self, path: &Path) -> Result<()> {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let uri = path_to_uri(path)?;

        let mut open = self.open_files.lock().await;
        let version = open.entry(path.to_path_buf()).or_insert(0);
        *version += 1;

        let item = TextDocumentItem {
            uri,
            language_id: self.language.clone(),
            version: *version,
            text: content,
        };

        self.send_notification(
            "textDocument/didOpen",
            Some(serde_json::to_value(DidOpenTextDocumentParams {
                text_document: item,
            })?),
        )
        .await
    }

    /// Notify the server that a file's content has changed (textDocument/didChange).
    /// Uses full document sync — sends the entire file content.
    /// The file must have been previously opened via `open_file`.
    pub async fn change_file(&self, path: &Path) -> Result<()> {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let uri = path_to_uri(path)?;

        let mut open = self.open_files.lock().await;
        let version = open
            .get_mut(path)
            .ok_or_else(|| anyhow::anyhow!("File not open: {}", path.display()))?;
        *version += 1;

        let params = lsp_types::DidChangeTextDocumentParams {
            text_document: lsp_types::VersionedTextDocumentIdentifier {
                uri,
                version: *version,
            },
            content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content,
            }],
        };

        self.send_notification(
            "textDocument/didChange",
            Some(serde_json::to_value(params)?),
        )
        .await
    }

    /// Notify the server about file content — opens if new, sends didChange if already open.
    pub async fn notify_file_changed(&self, path: &Path) -> Result<()> {
        let open = self.open_files.lock().await;
        let already_open = open.contains_key(path);
        drop(open);

        if already_open {
            self.change_file(path).await
        } else {
            self.open_file(path).await
        }
    }

    /// Go to definition of symbol at position. Returns locations.
    pub async fn goto_definition(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        self.notify_file_changed(path).await?;
        let uri = path_to_uri(path)?;
        let params = build_td_position(uri.as_str(), line, character);
        let resp = self
            .send_request(
                "textDocument/definition",
                Some(serde_json::to_value(params)?),
            )
            .await?;
        parse_location_response(resp.result)
    }

    /// Find all references to symbol at position.
    pub async fn find_references(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        self.notify_file_changed(path).await?;
        let uri = path_to_uri(path)?;
        let params = ReferenceParams {
            text_document_position: build_td_position(uri.as_str(), line, character),
            context: ReferenceContext {
                include_declaration: true,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let resp = self
            .send_request(
                "textDocument/references",
                Some(serde_json::to_value(params)?),
            )
            .await?;
        parse_location_response(resp.result)
    }

    /// Get hover info (type signature, docs) for symbol at position.
    pub async fn hover(&self, path: &Path, line: u32, character: u32) -> Result<Option<String>> {
        self.notify_file_changed(path).await?;
        let uri = path_to_uri(path)?;
        let params = build_td_position(uri.as_str(), line, character);
        let resp = self
            .send_request("textDocument/hover", Some(serde_json::to_value(params)?))
            .await?;
        Ok(parse_hover_response(resp.result))
    }

    /// Go to implementation of symbol at position.
    pub async fn goto_implementation(
        &self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        self.notify_file_changed(path).await?;
        let uri = path_to_uri(path)?;
        let params = build_td_position(uri.as_str(), line, character);
        let resp = self
            .send_request(
                "textDocument/implementation",
                Some(serde_json::to_value(params)?),
            )
            .await?;
        parse_location_response(resp.result)
    }

    /// List all symbols in a document.
    pub async fn document_symbol(&self, path: &Path) -> Result<Vec<DocumentSymbol>> {
        self.notify_file_changed(path).await?;
        let uri = path_to_uri(path)?;
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: uri.as_str().parse().expect("valid URI"),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let resp = self
            .send_request(
                "textDocument/documentSymbol",
                Some(serde_json::to_value(params)?),
            )
            .await?;
        parse_document_symbol_response(resp.result)
    }

    /// Search for symbols across the workspace.
    pub async fn workspace_symbol(&self, query: &str) -> Result<Vec<SymbolInformation>> {
        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let resp = self
            .send_request("workspace/symbol", Some(serde_json::to_value(params)?))
            .await?;
        parse_workspace_symbol_response(resp.result)
    }

    /// Get a snapshot of all cached diagnostics across all files this client has seen.
    pub async fn all_diagnostics(&self) -> HashMap<String, Vec<Diagnostic>> {
        self.diagnostics.lock().await.clone()
    }

    /// Get cached diagnostics for a file. Opens the file first if not already open.
    /// Waits briefly for diagnostics to arrive after opening.
    pub async fn get_diagnostics(&self, path: &Path) -> Result<Vec<Diagnostic>> {
        {
            let open = self.open_files.lock().await;
            if !open.contains_key(path) {
                drop(open);
                self.open_file(path).await?;
                // Wait for server to publish diagnostics
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }

        let uri = path_to_uri(path)?.as_str().to_string();

        let diags = self.diagnostics.lock().await;
        Ok(diags.get(&uri).cloned().unwrap_or_default())
    }

    /// Current server state.
    #[allow(dead_code)]
    pub fn state(&self) -> &ServerState {
        &self.state
    }

    /// Graceful shutdown: send shutdown request + exit notification.
    pub async fn shutdown(mut self) {
        self.state = ServerState::ShuttingDown;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let _ = self.send_request("shutdown", None).await;
            let _ = self.send_notification("exit", None).await;
        })
        .await;
        self.state = ServerState::Stopped;
    }
}

/// Build TextDocumentPositionParams from a URI string and 1-based line/char.
fn build_td_position(uri_str: &str, line: u32, character: u32) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: uri_str.parse().expect("valid URI"),
        },
        position: Position {
            line: line.saturating_sub(1),
            character: character.saturating_sub(1),
        },
    }
}

/// Parse a definition/implementation/references response into Vec<Location>.
/// LSP can return Location, Vec<Location>, or Vec<LocationLink>.
fn parse_location_response(result: Option<Value>) -> Result<Vec<Location>> {
    let Some(val) = result else {
        return Ok(vec![]);
    };
    if val.is_null() {
        return Ok(vec![]);
    }

    // Try as single Location first
    if let Ok(loc) = serde_json::from_value::<Location>(val.clone()) {
        return Ok(vec![loc]);
    }
    // Try as Vec<Location>
    if let Ok(locs) = serde_json::from_value::<Vec<Location>>(val.clone()) {
        return Ok(locs);
    }
    // Try as Vec<LocationLink> and convert
    if let Ok(links) = serde_json::from_value::<Vec<lsp_types::LocationLink>>(val) {
        let locs = links
            .into_iter()
            .map(|link| Location {
                uri: link.target_uri,
                range: link.target_selection_range,
            })
            .collect();
        return Ok(locs);
    }
    Ok(vec![])
}

/// Parse a hover response into a string.
fn parse_hover_response(result: Option<Value>) -> Option<String> {
    let val = result?;
    if val.is_null() {
        return None;
    }
    let hover: lsp_types::Hover = serde_json::from_value(val).ok()?;
    match hover.contents {
        lsp_types::HoverContents::Scalar(s) => Some(markedstring_to_string(s)),
        lsp_types::HoverContents::Array(arr) => {
            let parts: Vec<String> = arr.into_iter().map(markedstring_to_string).collect();
            Some(parts.join("\n\n"))
        }
        lsp_types::HoverContents::Markup(mc) => Some(mc.value),
    }
}

#[allow(deprecated)] // MarkedString is deprecated but still returned by some servers
fn markedstring_to_string(ms: lsp_types::MarkedString) -> String {
    match ms {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(ls) => ls.value,
    }
}

/// Parse documentSymbol response. Servers may return DocumentSymbol[] or SymbolInformation[].
fn parse_document_symbol_response(result: Option<Value>) -> Result<Vec<DocumentSymbol>> {
    let Some(val) = result else {
        return Ok(vec![]);
    };
    if val.is_null() {
        return Ok(vec![]);
    }

    // Try as Vec<DocumentSymbol> (hierarchical)
    if let Ok(syms) = serde_json::from_value::<Vec<DocumentSymbol>>(val.clone()) {
        return Ok(syms);
    }
    // Fallback: Vec<SymbolInformation> (flat) — convert to DocumentSymbol
    #[allow(deprecated)]
    if let Ok(infos) = serde_json::from_value::<Vec<SymbolInformation>>(val) {
        let syms = infos
            .into_iter()
            .map(|si| DocumentSymbol {
                name: si.name,
                detail: None,
                kind: si.kind,
                range: si.location.range,
                selection_range: si.location.range,
                children: None,
                tags: None,
                deprecated: None,
            })
            .collect();
        return Ok(syms);
    }
    Ok(vec![])
}

/// Parse workspace/symbol response.
fn parse_workspace_symbol_response(result: Option<Value>) -> Result<Vec<SymbolInformation>> {
    let Some(val) = result else {
        return Ok(vec![]);
    };
    if val.is_null() {
        return Ok(vec![]);
    }
    Ok(serde_json::from_value::<Vec<SymbolInformation>>(val).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_uri_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.rs");
        let uri = path_to_uri(&file).unwrap();
        let s = uri.as_str();
        assert!(s.starts_with("file:///"), "expected file URI, got: {s}");
        assert!(s.ends_with("test.rs"), "expected test.rs in URI, got: {s}");
    }

    #[test]
    fn text_document_position_params_converts_1based_to_0based() {
        let params = build_td_position("file:///tmp/test.rs", 5, 10);
        let v: serde_json::Value = serde_json::to_value(params).unwrap();
        assert_eq!(v["textDocument"]["uri"], "file:///tmp/test.rs");
        assert_eq!(v["position"]["line"], 4);
        assert_eq!(v["position"]["character"], 9);
    }
}
