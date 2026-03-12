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
    WorkspaceList(WorkspaceListState),
    WorkspaceAdd(WorkspaceAddState),
    AgentStreamOverlay(AgentStreamOverlayState),
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
            Self::WorkspaceList(_) => write!(f, "WorkspaceList(...)"),
            Self::WorkspaceAdd(_) => write!(f, "WorkspaceAdd(...)"),
            Self::AgentStreamOverlay(_) => write!(f, "AgentStreamOverlay(...)"),
        }
    }
}

/// State for the agent stream overlay dialog.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AgentStreamOverlayState {
    /// Scroll offset for the stream log.
    pub scroll_offset: usize,
    /// Whether the view should follow new output automatically.
    pub follow: bool,
}

impl AgentStreamOverlayState {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            follow: true,
        }
    }
}

impl Default for AgentStreamOverlayState {
    fn default() -> Self {
        Self::new()
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
    pub secondaries: Vec<RoundhouseSecondary>,
    pub selected: usize,
}

/// A secondary model added to a Roundhouse session.
pub struct RoundhouseSecondary {
    pub provider_id: String,
    pub display_name: String,
    pub model: String,
}

/// State for the circuits list dialog.
pub struct CircuitsListState {
    pub selected: usize,
}

/// Phase of the workspace-add / workspace-edit flow.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceAddPhase {
    Path,
    Name,
    Mode,
    Permissions,
}

/// State for the workspace-add multi-step dialog (also used for editing).
#[derive(Debug, Clone)]
pub struct WorkspaceAddState {
    pub phase: WorkspaceAddPhase,
    /// Raw path string as the user types it.
    pub path_input: String,
    /// Fuzzy directory suggestions (populated async).
    pub path_matches: Vec<String>,
    /// Currently highlighted suggestion index.
    pub path_selected: usize,
    /// Workspace name (pre-filled from dirname after path is confirmed).
    pub name_input: String,
    /// Mode selection: 0 = Proactive, 1 = Explicit.
    pub mode_selected: usize,
    /// Access selection: 0 = ReadWrite, 1 = ReadOnly.
    pub permissions_selected: usize,
    /// When editing an existing workspace, the name being edited.
    pub editing_name: Option<String>,
    /// Inline validation error (cleared on new input).
    pub error: Option<String>,
}

impl Default for WorkspaceAddState {
    fn default() -> Self {
        Self {
            phase: WorkspaceAddPhase::Path,
            path_input: String::new(),
            path_matches: Vec::new(),
            path_selected: 0,
            name_input: String::new(),
            mode_selected: 0,
            permissions_selected: 0,
            editing_name: None,
            error: None,
        }
    }
}

impl WorkspaceAddState {
    /// Create state pre-loaded for editing an existing workspace (starts at Mode phase).
    pub fn for_edit(
        name: String,
        path: String,
        mode_selected: usize,
        permissions_selected: usize,
    ) -> Self {
        Self {
            phase: WorkspaceAddPhase::Mode,
            path_input: path,
            name_input: name.clone(),
            editing_name: Some(name),
            mode_selected,
            permissions_selected,
            ..Default::default()
        }
    }
}

/// State for the workspace-list dialog.
#[derive(Debug, Clone)]
pub struct WorkspaceListState {
    /// (name, config, is_available) — is_available checked at open time.
    pub workspaces: Vec<(String, crate::config::schema::WorkspaceConfig, bool)>,
    pub selected: usize,
}

impl WorkspaceListState {
    /// Clamp `selected` to valid index range (saturating to last entry).
    pub fn clamp_selected(&mut self) {
        let max = self.workspaces.len().saturating_sub(1);
        if self.selected > max {
            self.selected = max;
        }
    }
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

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut DialogKind> {
        self.overlays.iter_mut()
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
    McpServer {
        name: String,
        config: serde_json::Value,
    },
    SystemPrompt(String),
    ClaudeMd(std::path::PathBuf),
}

pub enum MigrationPhase {
    Checklist,
    Preview,
    #[allow(dead_code)]
    Applying,
    Done(String),
}

/// Build a migration checklist by scanning the given platform's config.
pub fn build_migration_checklist(
    platform: crate::migrate::SourcePlatform,
) -> MigrationChecklistState {
    let dirs = crate::migrate::detection::config_paths(&platform);
    let mut items = Vec::new();

    match &platform {
        crate::migrate::SourcePlatform::ClaudeCode => {
            let config = crate::migrate::claude_code::scan_claude_code(
                &dirs,
                Some(std::path::Path::new(".")),
            );
            for (name, server) in &config.mcp_servers {
                items.push(MigrationItem {
                    label: format!("MCP: {name}"),
                    description: "Import MCP server config".to_string(),
                    toggled: true,
                    kind: MigrationItemKind::McpServer {
                        name: name.clone(),
                        config: server.clone(),
                    },
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
                    kind: MigrationItemKind::McpServer {
                        name: name.clone(),
                        config: server.clone(),
                    },
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

    MigrationChecklistState {
        platform,
        items,
        selected: 0,
        phase: MigrationPhase::Checklist,
    }
}

#[cfg(test)]
mod stream_overlay_tests {
    use super::*;

    #[test]
    fn agent_stream_overlay_state_default() {
        let state = AgentStreamOverlayState { scroll_offset: 0, follow: true };
        assert!(state.follow);
        assert_eq!(state.scroll_offset, 0);
    }
}

#[cfg(test)]
mod workspace_dialog_tests {
    use super::*;

    #[test]
    fn workspace_add_state_default_phase_is_path() {
        let state = WorkspaceAddState::default();
        assert!(matches!(state.phase, WorkspaceAddPhase::Path));
    }

    #[test]
    fn workspace_add_state_default_inputs_are_empty() {
        let state = WorkspaceAddState::default();
        assert!(state.path_input.is_empty());
        assert!(state.name_input.is_empty());
        assert!(state.path_matches.is_empty());
        assert!(state.error.is_none());
    }

    #[test]
    fn workspace_list_state_selected_clamps() {
        let mut state = WorkspaceListState {
            workspaces: vec![],
            selected: 5,
        };
        state.clamp_selected();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn workspace_list_state_with_entries_clamps() {
        use crate::config::schema::{WorkspaceConfig, WorkspaceMode};
        let mut state = WorkspaceListState {
            workspaces: vec![
                ("a".to_string(), WorkspaceConfig { path: "/tmp/a".to_string(), mode: WorkspaceMode::Proactive, access: crate::config::schema::WorkspaceAccess::ReadWrite }, true),
                ("b".to_string(), WorkspaceConfig { path: "/tmp/b".to_string(), mode: WorkspaceMode::Explicit, access: crate::config::schema::WorkspaceAccess::ReadOnly }, false),
            ],
            selected: 10,
        };
        state.clamp_selected();
        assert_eq!(state.selected, 1);
    }
}
