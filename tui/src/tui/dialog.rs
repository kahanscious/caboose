//! Dialog stack — explicit overlay management replacing Mode enum.

use crate::tui::file_browser::FileBrowserState;
use crate::tui::key_input::KeyInputState;
use crate::tui::mcp_input::McpServerInputState;

/// The base screen (always exactly one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Chat,
}

/// A dialog overlay that can be pushed onto the stack.
pub enum DialogKind {
    ApiKeyInput(KeyInputState),
    CommandPalette(CommandPaletteState),
    FileBrowser(FileBrowserState),
    LocalProviderConnect(LocalProviderConnectState),
    McpServerInput(McpServerInputState),
    PasteConfirm {
        text: String,
        line_count: usize,
        char_count: usize,
    },
    RoundhouseProviderPicker(RoundhousePickerState),
    CircuitsList(CircuitsListState),
    MigrationChecklist(MigrationChecklistState),
}

// Debug impl needed for Action derive
impl std::fmt::Debug for DialogKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKeyInput(_) => write!(f, "ApiKeyInput(...)"),
            Self::CommandPalette(_) => write!(f, "CommandPalette(...)"),
            Self::FileBrowser(_) => write!(f, "FileBrowser(...)"),
            Self::LocalProviderConnect(_) => write!(f, "LocalProviderConnect(...)"),
            Self::McpServerInput(_) => write!(f, "McpServerInput(...)"),
            Self::PasteConfirm {
                line_count,
                char_count,
                ..
            } => {
                write!(f, "PasteConfirm({line_count} lines, {char_count} chars)")
            }
            Self::RoundhouseProviderPicker(_) => write!(f, "RoundhouseProviderPicker(...)"),
            Self::CircuitsList(_) => write!(f, "CircuitsList(...)"),
            Self::MigrationChecklist(_) => write!(f, "MigrationChecklist(...)"),
        }
    }
}

/// State for the command palette overlay.
pub struct CommandPaletteState {
    pub filter: String,
    pub selected: usize,
}

impl CommandPaletteState {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            selected: 0,
        }
    }
}

/// Phase of the local provider connect flow.
pub enum LocalConnectPhase {
    /// Editing the server address.
    Address,
    /// Async probe in progress.
    Probing,
    /// Choose from discovered models.
    ModelSelect,
}

/// State for the local provider connect dialog.
pub struct LocalProviderConnectState {
    pub provider_id: String,
    pub provider_name: String,
    pub address: String,
    pub models: Vec<String>,
    pub selected_model: usize,
    pub phase: LocalConnectPhase,
    pub error: Option<String>,
    /// Receiver for async probe result.
    pub probe_rx: Option<tokio::sync::oneshot::Receiver<Result<Vec<String>, String>>>,
}

/// State for the Roundhouse provider picker dialog.
pub struct RoundhousePickerState {
    pub providers: Vec<RoundhouseProviderOption>,
    pub selected: usize,
}

/// A provider option shown in the Roundhouse picker.
pub struct RoundhouseProviderOption {
    pub id: String,
    pub display_name: String,
    pub model: String,
    pub toggled: bool,
}

/// State for the circuits list dialog.
pub struct CircuitsListState {
    pub selected: usize,
}

/// The dialog stack — a base screen plus zero or more overlays.
pub struct DialogStack {
    pub base: Screen,
    overlays: Vec<DialogKind>,
}

impl DialogStack {
    pub fn new(base: Screen) -> Self {
        Self {
            base,
            overlays: Vec::new(),
        }
    }

    /// Push a new overlay onto the stack.
    pub fn push(&mut self, dialog: DialogKind) {
        self.overlays.push(dialog);
    }

    /// Replace the top overlay (or push if stack is empty).
    #[allow(dead_code)]
    pub fn replace(&mut self, dialog: DialogKind) {
        if self.overlays.is_empty() {
            self.overlays.push(dialog);
        } else {
            let last = self.overlays.len() - 1;
            self.overlays[last] = dialog;
        }
    }

    /// Pop the top overlay. Returns it, or None if stack is empty.
    pub fn pop(&mut self) -> Option<DialogKind> {
        self.overlays.pop()
    }

    /// Clear all overlays, returning to just the base screen.
    pub fn clear(&mut self) {
        self.overlays.clear();
    }

    /// Get a reference to the top overlay, if any.
    pub fn top(&self) -> Option<&DialogKind> {
        self.overlays.last()
    }

    /// Get a mutable reference to the top overlay, if any.
    pub fn top_mut(&mut self) -> Option<&mut DialogKind> {
        self.overlays.last_mut()
    }

    /// Whether any overlays are open.
    pub fn has_overlay(&self) -> bool {
        !self.overlays.is_empty()
    }

    /// How many overlays are on the stack.
    #[allow(dead_code)]
    pub fn depth(&self) -> usize {
        self.overlays.len()
    }
}

// ── Migration checklist types ──────────────────────────────────────────

pub struct MigrationChecklistState {
    pub platform: crate::migrate::SourcePlatform,
    pub items: Vec<MigrationItem>,
    pub selected: usize,
    pub phase: MigrationPhase,
}

pub struct MigrationItem {
    pub label: String,
    pub description: String,
    pub toggled: bool,
    pub kind: MigrationItemKind,
}

pub enum MigrationItemKind {
    McpServer { name: String, config: serde_json::Value },
    SystemPrompt(String),
    ClaudeMd(std::path::PathBuf),
}

pub enum MigrationPhase {
    Checklist,
    Preview,
    Applying,
    Done(String),
}

/// Build a migration checklist by scanning the given platform's config.
pub fn build_migration_checklist(platform: crate::migrate::SourcePlatform) -> MigrationChecklistState {
    let dirs = crate::migrate::detection::config_paths(&platform);
    let mut items = Vec::new();

    match &platform {
        crate::migrate::SourcePlatform::ClaudeCode => {
            let config = crate::migrate::claude_code::scan_claude_code(&dirs, Some(std::path::Path::new(".")));
            for (name, server) in &config.mcp_servers {
                items.push(MigrationItem {
                    label: format!("MCP: {name}"),
                    description: "Import MCP server config".to_string(),
                    toggled: true,
                    kind: MigrationItemKind::McpServer { name: name.clone(), config: server.clone() },
                });
            }
            if let Some(prompt) = &config.system_prompt {
                let preview: String = prompt.chars().take(60).collect();
                let suffix = if prompt.len() > 60 { "..." } else { "" };
                items.push(MigrationItem {
                    label: "System prompt".to_string(),
                    description: format!("{preview}{suffix}"),
                    toggled: true,
                    kind: MigrationItemKind::SystemPrompt(prompt.clone()),
                });
            }
            for path in &config.claude_md_paths {
                items.push(MigrationItem {
                    label: "CLAUDE.md \u{2192} CABOOSE.md".to_string(),
                    description: path.display().to_string(),
                    toggled: true,
                    kind: MigrationItemKind::ClaudeMd(path.clone()),
                });
            }
        }
        crate::migrate::SourcePlatform::OpenCode => {
            let config = crate::migrate::open_code::scan_open_code(&dirs);
            for (name, server) in &config.mcp_servers {
                items.push(MigrationItem {
                    label: format!("MCP: {name}"),
                    description: "Import MCP server config".to_string(),
                    toggled: true,
                    kind: MigrationItemKind::McpServer { name: name.clone(), config: server.clone() },
                });
            }
            if let Some(prompt) = &config.system_prompt {
                let preview: String = prompt.chars().take(60).collect();
                let suffix = if prompt.len() > 60 { "..." } else { "" };
                items.push(MigrationItem {
                    label: "Custom instructions".to_string(),
                    description: format!("{preview}{suffix}"),
                    toggled: true,
                    kind: MigrationItemKind::SystemPrompt(prompt.clone()),
                });
            }
        }
        crate::migrate::SourcePlatform::Codex => {
            let config = crate::migrate::codex::scan_codex(&dirs);
            if let Some(instructions) = &config.instructions {
                let preview: String = instructions.chars().take(60).collect();
                let suffix = if instructions.len() > 60 { "..." } else { "" };
                items.push(MigrationItem {
                    label: "Config instructions".to_string(),
                    description: format!("{preview}{suffix}"),
                    toggled: true,
                    kind: MigrationItemKind::SystemPrompt(instructions.clone()),
                });
            }
            if let Some(md) = &config.instructions_md {
                let preview: String = md.chars().take(60).collect();
                let suffix = if md.len() > 60 { "..." } else { "" };
                items.push(MigrationItem {
                    label: "Instructions file".to_string(),
                    description: format!("{preview}{suffix}"),
                    toggled: true,
                    kind: MigrationItemKind::SystemPrompt(md.clone()),
                });
            }
        }
    }

    MigrationChecklistState { platform, items, selected: 0, phase: MigrationPhase::Checklist }
}
