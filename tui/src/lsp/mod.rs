//! LSP integration — language server lifecycle, diagnostics, and code intelligence.

pub mod client;
pub mod detection;
pub mod types;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use lsp_types::Diagnostic;

use crate::config::schema::LspConfig;
use client::LspClient;

/// Convert a file:// URI back to a filesystem path.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path_str = uri.strip_prefix("file://")?;
    Some(PathBuf::from(path_str))
}

/// Maps file extensions to language keys.
fn build_extension_map() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        (".ts", "typescript"),
        (".tsx", "typescript"),
        (".js", "typescript"),
        (".jsx", "typescript"),
        (".rs", "rust"),
        (".go", "go"),
        (".py", "python"),
    ])
}

/// Manages LSP server connections across languages.
pub struct LspManager {
    clients: HashMap<String, LspClient>,
    config: LspConfig,
    workspace_root: PathBuf,
    extension_map: HashMap<&'static str, &'static str>,
    broken_servers: HashSet<String>,
}

impl LspManager {
    /// Create a new manager. Merges auto-detected servers with user config.
    /// Does NOT start any servers — they start lazily on first request.
    pub fn new(workspace_root: PathBuf, user_config: Option<LspConfig>) -> Self {
        let mut detected = detection::detect_languages(&workspace_root);

        let config = if let Some(mut user) = user_config {
            // Auto-detected fill gaps — user config takes precedence
            for (name, cfg) in detected.drain() {
                user.servers.entry(name).or_insert(cfg);
            }
            user
        } else {
            LspConfig {
                enabled: true,
                servers: detected,
            }
        };

        Self {
            clients: HashMap::new(),
            config,
            workspace_root,
            extension_map: build_extension_map(),
            broken_servers: HashSet::new(),
        }
    }

    /// Resolve a file path to its language key via extension.
    pub fn language_for_path(&self, path: &Path) -> Option<&str> {
        let ext = path.extension()?.to_str()?;
        let dotted = format!(".{ext}");
        self.extension_map.get(dotted.as_str()).copied()
    }

    /// Ensure a client is running for the given path's language. Returns language key.
    /// Lazily starts the server if needed. Returns Err if broken, disabled, or no config.
    async fn ensure_client(&mut self, path: &Path) -> anyhow::Result<String> {
        if !self.config.enabled {
            anyhow::bail!("LSP is disabled in configuration");
        }

        let abs_path = self.resolve_path(path);

        let language = self
            .language_for_path(&abs_path)
            .ok_or_else(|| {
                let ext = abs_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("unknown");
                anyhow::anyhow!("No LSP server configured for .{ext} files")
            })?
            .to_string();

        if self.broken_servers.contains(&language) {
            anyhow::bail!(
                "LSP server for '{language}' previously failed to start and is disabled for this session"
            );
        }

        if !self.clients.contains_key(&language) {
            let server_config = self.config.servers.get(&language)
                .ok_or_else(|| anyhow::anyhow!(
                    "No LSP server configured for language '{language}'. \
                     Install a language server or add [lsp.servers.{language}] to .caboose/config.toml"
                ))?;

            if server_config.disabled {
                anyhow::bail!("LSP server for '{language}' is disabled in configuration");
            }

            match LspClient::start(
                &language,
                &server_config.command,
                &server_config.args,
                &server_config.env,
                &self.workspace_root,
            )
            .await
            {
                Ok(client) => {
                    self.clients.insert(language.clone(), client);
                    // Preload key project files so the server indexes them
                    let files_to_preload = preload_files(&language, &self.workspace_root);
                    if let Some(client) = self.clients.get(&language) {
                        for file in &files_to_preload {
                            let _ = client.open_file(file).await;
                        }
                    }
                }
                Err(e) => {
                    self.broken_servers.insert(language.clone());
                    return Err(e).with_context(|| {
                        format!(
                            "Failed to start LSP server for '{language}'. \
                         Is '{}' installed and on PATH? Server marked as broken for this session.",
                            server_config.command
                        )
                    });
                }
            }
        }

        Ok(language)
    }

    /// Resolve path to absolute, applying workspace root.
    fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        }
    }

    /// Get diagnostics for a file. Lazily starts the LSP server if needed.
    pub async fn get_diagnostics(&mut self, path: &Path) -> anyhow::Result<Vec<Diagnostic>> {
        let language = self.ensure_client(path).await?;
        let abs_path = self.resolve_path(path);
        let client = self.clients.get(&language).unwrap();
        client.get_diagnostics(&abs_path).await
    }

    /// Go to definition of symbol at position (1-based line/char).
    pub async fn goto_definition(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> anyhow::Result<Vec<lsp_types::Location>> {
        let language = self.ensure_client(path).await?;
        let abs_path = self.resolve_path(path);
        let client = self.clients.get(&language).unwrap();
        client.goto_definition(&abs_path, line, character).await
    }

    /// Find all references to symbol at position (1-based line/char).
    pub async fn find_references(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> anyhow::Result<Vec<lsp_types::Location>> {
        let language = self.ensure_client(path).await?;
        let abs_path = self.resolve_path(path);
        let client = self.clients.get(&language).unwrap();
        client.find_references(&abs_path, line, character).await
    }

    /// Get hover info for symbol at position (1-based line/char).
    pub async fn hover(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> anyhow::Result<Option<String>> {
        let language = self.ensure_client(path).await?;
        let abs_path = self.resolve_path(path);
        let client = self.clients.get(&language).unwrap();
        client.hover(&abs_path, line, character).await
    }

    /// Go to implementation of symbol at position (1-based line/char).
    pub async fn goto_implementation(
        &mut self,
        path: &Path,
        line: u32,
        character: u32,
    ) -> anyhow::Result<Vec<lsp_types::Location>> {
        let language = self.ensure_client(path).await?;
        let abs_path = self.resolve_path(path);
        let client = self.clients.get(&language).unwrap();
        client.goto_implementation(&abs_path, line, character).await
    }

    /// List all symbols in a document.
    pub async fn document_symbol(
        &mut self,
        path: &Path,
    ) -> anyhow::Result<Vec<lsp_types::DocumentSymbol>> {
        let language = self.ensure_client(path).await?;
        let abs_path = self.resolve_path(path);
        let client = self.clients.get(&language).unwrap();
        client.document_symbol(&abs_path).await
    }

    /// Search for symbols across the workspace. Queries all running clients.
    pub async fn workspace_symbol(
        &mut self,
        query: &str,
    ) -> anyhow::Result<Vec<lsp_types::SymbolInformation>> {
        if !self.config.enabled {
            anyhow::bail!("LSP is disabled in configuration");
        }
        let mut all_results = Vec::new();
        for client in self.clients.values() {
            if let Ok(syms) = client.workspace_symbol(query).await {
                all_results.extend(syms);
            }
        }
        Ok(all_results)
    }

    /// Notify the appropriate LSP server that a file has changed on disk.
    /// Silently no-ops if LSP is disabled or no server is configured for the file type.
    pub async fn notify_file_changed(&mut self, path: &Path) -> anyhow::Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };

        let language = match self.language_for_path(&abs_path) {
            Some(l) => l.to_string(),
            None => return Ok(()), // No LSP for this file type — silent no-op
        };

        // Only notify if the server is already running (don't start servers just for notifications)
        if let Some(client) = self.clients.get(&language) {
            client.notify_file_changed(&abs_path).await?;
        }

        Ok(())
    }

    /// Collect all cached diagnostics across all running language servers.
    /// Returns a map from file path to diagnostics.
    pub async fn all_diagnostics(&self) -> HashMap<PathBuf, Vec<lsp_types::Diagnostic>> {
        let mut result = HashMap::new();
        for client in self.clients.values() {
            let client_diags = client.all_diagnostics().await;
            for (uri_str, diags) in client_diags {
                if !diags.is_empty()
                    && let Some(path) = uri_to_path(&uri_str)
                {
                    result.insert(path, diags);
                }
            }
        }
        result
    }

    /// Shut down all running LSP servers.
    pub async fn shutdown_all(self) {
        for (_lang, client) in self.clients {
            client.shutdown().await;
        }
    }
}

/// Return key project files to preload for a language.
/// Only returns files that actually exist. Capped at 5.
fn preload_files(language: &str, workspace_root: &Path) -> Vec<PathBuf> {
    let candidates: Vec<PathBuf> = match language {
        "rust" => vec![
            workspace_root.join("src/main.rs"),
            workspace_root.join("src/lib.rs"),
        ],
        "typescript" => vec![
            workspace_root.join("src/index.ts"),
            workspace_root.join("src/index.tsx"),
            workspace_root.join("index.ts"),
        ],
        "go" => vec![
            workspace_root.join("main.go"),
            workspace_root.join("cmd/main.go"),
        ],
        "python" => vec![
            workspace_root.join("main.py"),
            workspace_root.join("src/__init__.py"),
        ],
        _ => vec![],
    };

    candidates
        .into_iter()
        .filter(|p| p.exists())
        .take(5)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::LspServerConfig;

    #[test]
    fn broken_servers_tracked() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        assert!(mgr.broken_servers.is_empty());
    }

    #[test]
    fn extension_map_resolves_typescript() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        assert_eq!(
            mgr.language_for_path(Path::new("foo.ts")),
            Some("typescript")
        );
        assert_eq!(
            mgr.language_for_path(Path::new("foo.tsx")),
            Some("typescript")
        );
        assert_eq!(
            mgr.language_for_path(Path::new("foo.js")),
            Some("typescript")
        );
        assert_eq!(
            mgr.language_for_path(Path::new("foo.jsx")),
            Some("typescript")
        );
    }

    #[test]
    fn extension_map_resolves_rust() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        assert_eq!(mgr.language_for_path(Path::new("main.rs")), Some("rust"));
    }

    #[test]
    fn extension_map_resolves_go() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        assert_eq!(mgr.language_for_path(Path::new("main.go")), Some("go"));
    }

    #[test]
    fn extension_map_resolves_python() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        assert_eq!(mgr.language_for_path(Path::new("app.py")), Some("python"));
    }

    #[test]
    fn unknown_extension_returns_none() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        assert_eq!(mgr.language_for_path(Path::new("file.xyz")), None);
    }

    #[test]
    fn user_config_overrides_autodetect() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();

        let user = LspConfig {
            enabled: true,
            servers: HashMap::from([(
                "rust".to_string(),
                LspServerConfig {
                    command: "my-custom-ra".to_string(),
                    args: vec!["--special".to_string()],
                    env: HashMap::new(),
                    disabled: false,
                },
            )]),
        };

        let mgr = LspManager::new(tmp.path().to_path_buf(), Some(user));
        assert_eq!(mgr.config.servers["rust"].command, "my-custom-ra");
    }

    #[test]
    fn uri_to_path_works() {
        let path = uri_to_path("file:///tmp/test.rs").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/test.rs"));
    }

    #[test]
    fn uri_to_path_returns_none_for_non_file() {
        assert!(uri_to_path("http://example.com").is_none());
    }

    #[tokio::test]
    async fn all_diagnostics_empty_when_no_clients() {
        let mgr = LspManager::new(PathBuf::from("/tmp"), None);
        let diags = mgr.all_diagnostics().await;
        assert!(diags.is_empty());
    }

    #[test]
    fn preload_files_returns_rust_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(src.join("lib.rs"), "").unwrap();

        let files = preload_files("rust", tmp.path());
        assert_eq!(files.len(), 2);
        assert!(files.contains(&src.join("main.rs")));
        assert!(files.contains(&src.join("lib.rs")));
    }

    #[test]
    fn preload_files_skips_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let files = preload_files("rust", tmp.path());
        assert!(files.is_empty());
    }

    #[test]
    fn preload_files_unknown_language_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let files = preload_files("haskell", tmp.path());
        assert!(files.is_empty());
    }

    #[test]
    fn autodetect_fills_gaps_in_user_config() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        std::fs::write(tmp.path().join("tsconfig.json"), "{}").unwrap();

        let user = LspConfig {
            enabled: true,
            servers: HashMap::from([(
                "rust".to_string(),
                LspServerConfig {
                    command: "my-ra".to_string(),
                    args: vec![],
                    env: HashMap::new(),
                    disabled: false,
                },
            )]),
        };

        let mgr = LspManager::new(tmp.path().to_path_buf(), Some(user));
        assert_eq!(mgr.config.servers["rust"].command, "my-ra");
        assert!(mgr.config.servers.contains_key("typescript"));
    }
}
