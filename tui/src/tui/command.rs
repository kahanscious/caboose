//! Centralized command registry — every action is a registered Command.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::app::State;

/// Command categories for grouping in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Navigation,
    Session,
    Tools,
    Provider,
}

impl Category {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Session => "Session",
            Self::Tools => "Tools",
            Self::Provider => "Provider",
        }
    }
}

/// A keybinding — key code plus required modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBind {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBind {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Format for display in command palette (e.g., "Ctrl+K").
    pub fn display(&self) -> String {
        let mut s = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            s.push_str("Ctrl+");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            s.push_str("Alt+");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            s.push_str("Shift+");
        }
        match self.code {
            KeyCode::Char(c) => {
                for ch in c.to_uppercase() {
                    s.push(ch);
                }
            }
            KeyCode::Enter => s.push_str("Enter"),
            KeyCode::Esc => s.push_str("Esc"),
            KeyCode::Tab => s.push_str("Tab"),
            KeyCode::Backspace => s.push_str("Backspace"),
            _ => s.push_str(&format!("{:?}", self.code)),
        }
        s
    }
}

/// Action returned by a command — tells the app loop what to do.
#[derive(Debug)]
pub enum Action {
    /// Command handled internally, no further action needed.
    None,
    /// Open an overlay dialog.
    PushDialog(super::dialog::DialogKind),
    /// Enter a dropdown picker mode.
    EnterPickerMode(crate::tui::slash_auto::SlashAutoState),
    /// Request application quit.
    Quit,
}

/// A registered command.
pub struct Command {
    pub id: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub keybind: Option<KeyBind>,
    pub slash: Option<&'static str>,
    /// Whether this command is available in the current state.
    pub available: fn(&State) -> bool,
    /// Execute the command. Returns an Action for the app loop.
    pub execute: fn(&mut State) -> Action,
}

/// The command registry — holds all registered commands.
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn register(&mut self, command: Command) {
        self.commands.push(command);
    }

    /// Find a command by its unique ID.
    pub fn find_by_id(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.id == id)
    }

    /// Find a command by slash name (e.g., "/model" matches "model").
    pub fn find_slash(&self, slash: &str) -> Option<&Command> {
        self.commands.iter().find(|c| c.slash == Some(slash))
    }

    /// Find a command by keybind.
    pub fn find_keybind(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<&Command> {
        self.commands.iter().find(|c| {
            c.keybind
                .map(|kb| kb.code == code && kb.modifiers == modifiers)
                .unwrap_or(false)
        })
    }

    /// Iterate over all commands that have a slash alias.
    pub fn slash_commands(&self) -> impl Iterator<Item = &Command> {
        self.commands.iter().filter(|c| c.slash.is_some())
    }

    /// All available commands, filtered by current state.
    pub fn available(&self, state: &State) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|c| (c.available)(state))
            .collect()
    }

    /// All commands grouped by category, filtered by availability.
    pub fn available_by_category(&self, state: &State) -> Vec<(Category, Vec<&Command>)> {
        let available = self.available(state);
        let categories = [
            Category::Provider,
            Category::Session,
            Category::Navigation,
            Category::Tools,
        ];
        categories
            .iter()
            .filter_map(|cat| {
                let cmds: Vec<_> = available
                    .iter()
                    .filter(|c| c.category == *cat)
                    .copied()
                    .collect();
                if cmds.is_empty() {
                    None
                } else {
                    Some((*cat, cmds))
                }
            })
            .collect()
    }
}

/// Build the default command registry with all built-in commands.
pub fn build_default_registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();

    registry.register(Command {
        id: "model.open",
        name: "Switch Model",
        category: Category::Provider,
        keybind: Some(KeyBind::new(KeyCode::Char('m'), KeyModifiers::CONTROL)),
        slash: Some("model"),
        available: |_| true,
        execute: |_state| Action::None, // Handled specially — needs async model loading
    });

    registry.register(Command {
        id: "provider.connect",
        name: "Connect Provider",
        category: Category::Provider,
        keybind: None,
        slash: Some("connect"),
        available: |_| true,
        execute: |_state| {
            Action::EnterPickerMode(crate::tui::slash_auto::SlashAutoState::with_providers())
        },
    });

    registry.register(Command {
        id: "session.compact",
        name: "Compact Context",
        category: Category::Session,
        keybind: None,
        slash: Some("compact"),
        available: |state| matches!(state.dialog_stack.base, super::dialog::Screen::Chat),
        execute: |_state| Action::None, // Handled specially — needs provider
    });

    registry.register(Command {
        id: "session.new",
        name: "New Session",
        category: Category::Session,
        keybind: Some(KeyBind::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
        slash: Some("new"),
        available: |_| true,
        execute: |state| {
            // Clear all chat state and return to home screen
            state.chat_messages.clear();
            state.input.clear();
            state.scroll_offset = 0;
            state.user_scrolled_up = false;
            state.session_title = None;
            state.session_title_source = None;
            state.current_session_id = None;
            state.modified_files.clear();
            state.file_baselines.clear();
            state.tool_counts.clear();
            state.focused_tool = None;
            state.pending_handoff = None;
            state.roundhouse_session = None;
            state.roundhouse_update_rx = None;
            state.roundhouse_synthesis_rx = None;
            state.roundhouse_model_add = false;
            state.agent.cancel();
            state.agent.conversation.messages.clear();
            state.agent.turn_count = 0;
            state.agent.session_allows.clear();
            state.agent.handoff_prompted = false;
            state.dialog_stack.base = super::dialog::Screen::Home;
            state.dialog_stack.clear();
            Action::None
        },
    });

    registry.register(Command {
        id: "session.list",
        name: "Session History",
        category: Category::Session,
        keybind: Some(KeyBind::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
        slash: Some("sessions"),
        available: |_| true,
        execute: |state| match state.sessions.list_with_content(50) {
            Ok(results) => Action::EnterPickerMode(
                crate::tui::slash_auto::SlashAutoState::with_sessions(results),
            ),
            Err(e) => {
                state.chat_messages.push(crate::app::ChatMessage::Error {
                    content: format!("Failed to load sessions: {e}"),
                });
                Action::None
            }
        },
    });

    registry.register(Command {
        id: "session.title",
        name: "Rename Session",
        category: Category::Session,
        keybind: None,
        slash: Some("title"),
        available: |state| state.current_session_id.is_some(),
        execute: |_state| Action::None, // Handled specially — needs arg parsing
    });

    registry.register(Command {
        id: "memory.list",
        name: "View Memories",
        category: Category::Session,
        keybind: None,
        slash: Some("memories"),
        available: |_| true,
        execute: |_state| Action::None, // Handled in app.rs — reads files
    });

    registry.register(Command {
        id: "memory.forget",
        name: "Forget Memory",
        category: Category::Session,
        keybind: None,
        slash: Some("forget"),
        available: |_| true,
        execute: |_state| Action::None, // Handled in app.rs — needs picker
    });

    registry.register(Command {
        id: "palette.open",
        name: "Command Palette",
        category: Category::Navigation,
        keybind: Some(KeyBind::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
        slash: None,
        available: |_| true,
        execute: |_state| {
            Action::PushDialog(super::dialog::DialogKind::CommandPalette(
                super::dialog::CommandPaletteState::new(),
            ))
        },
    });

    registry.register(Command {
        id: "sidebar.toggle",
        name: "Toggle Sidebar",
        category: Category::Navigation,
        keybind: Some(KeyBind::new(KeyCode::Char('b'), KeyModifiers::CONTROL)),
        slash: None,
        available: |state| matches!(state.dialog_stack.base, super::dialog::Screen::Chat),
        execute: |state| {
            state.sidebar_visible = !state.sidebar_visible;
            let mut prefs = crate::config::prefs::TuiPrefs::load();
            prefs.sidebar_visible = state.sidebar_visible;
            prefs.save();
            Action::None
        },
    });

    registry.register(Command {
        id: "terminal.toggle",
        name: "Toggle Terminal",
        category: Category::Navigation,
        keybind: None,
        slash: Some("terminal"),
        available: |_| true,
        execute: |state| {
            match &mut state.terminal_panel {
                Some(panel) => {
                    panel.visible = !panel.visible;
                    if !panel.visible {
                        state.terminal_focused = false;
                    }
                }
                None => {
                    let cwd =
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                    let cwd_str = cwd.to_string_lossy();
                    match crate::terminal::panel::TerminalPanel::new(80, 24, &cwd_str) {
                        Ok(mut panel) => {
                            panel.visible = true;
                            state.terminal_panel = Some(panel);
                        }
                        Err(e) => {
                            state.chat_messages.push(crate::app::ChatMessage::Error {
                                content: format!("Failed to spawn terminal: {e}"),
                            });
                        }
                    }
                }
            }
            Action::None
        },
    });

    registry.register(Command {
        id: "mcp.list",
        name: "MCP Servers",
        category: Category::Tools,
        keybind: None,
        slash: Some("mcp"),
        available: |_| true,
        execute: |_state| Action::None, // Handled specially — needs arg parsing
    });

    registry.register(Command {
        id: "checkpoint.rewind",
        name: "Rewind",
        category: Category::Navigation,
        keybind: None,
        slash: Some("rewind"),
        available: |_| true,
        execute: |_state| Action::None, // Handled in app.rs — needs checkpoint data
    });

    registry.register(Command {
        id: "settings.open",
        name: "Settings",
        category: Category::Navigation,
        keybind: None,
        slash: Some("settings"),
        available: |_| true,
        execute: |_state| Action::None, // Handled in app.rs — needs to build settings items
    });

    registry.register(Command {
        id: "skills.list",
        name: "List Skills",
        category: Category::Tools,
        keybind: None,
        slash: Some("skills"),
        available: |_| true,
        execute: |_state| {
            Action::EnterPickerMode(crate::tui::slash_auto::SlashAutoState::with_skills())
        },
    });

    registry.register(Command {
        id: "skills.create",
        name: "Create Skill",
        category: Category::Tools,
        keybind: None,
        slash: Some("create-skill"),
        available: |state| {
            // Block while agent is active or already creating a skill
            matches!(state.agent.state, crate::agent::AgentState::Idle)
                && state.skill_creation.is_none()
        },
        execute: |_state| {
            // Actual handling done in app.rs handle_create_skill_command()
            Action::None
        },
    });

    registry.register(Command {
        id: "init.run",
        name: "Init project with CABOOSE.md",
        category: Category::Tools,
        keybind: None,
        slash: Some("init"),
        available: |_| true,
        execute: |_state| Action::None, // Handled specially — needs async LLM call
    });

    registry.register(Command {
        id: "session.handoff",
        name: "Handoff Session",
        category: Category::Session,
        keybind: None,
        slash: Some("handoff"),
        available: |state| {
            // Only available on chat screen with messages
            matches!(state.dialog_stack.base, super::dialog::Screen::Chat)
                && !state.chat_messages.is_empty()
        },
        execute: |_state| Action::None, // Handled specially in app.rs — needs async
    });

    registry.register(Command {
        id: "roundhouse.start",
        name: "Roundhouse",
        category: Category::Tools,
        keybind: None,
        slash: Some("roundhouse"),
        available: |state| state.roundhouse_session.is_none(),
        execute: |state| {
            let primary_id = state.active_provider_name.clone();
            let primary_model = state.active_model_name.clone();

            state.roundhouse_session = Some(crate::roundhouse::RoundhouseSession::new(
                primary_id,
                primary_model,
            ));

            let picker = super::dialog::RoundhousePickerState {
                secondaries: vec![],
                selected: 0,
            };
            Action::PushDialog(super::dialog::DialogKind::RoundhouseProviderPicker(picker))
        },
    });

    registry.register(Command {
        id: "circuits.list",
        name: "Circuits",
        category: Category::Tools,
        keybind: None,
        slash: Some("circuits"),
        available: |_state| true,
        execute: |_state| {
            Action::PushDialog(super::dialog::DialogKind::CircuitsList(
                super::dialog::CircuitsListState { selected: 0 },
            ))
        },
    });

    registry.register(Command {
        id: "scm.watch",
        name: "Watch PR/MR",
        category: Category::Tools,
        keybind: None,
        slash: Some("watch"),
        available: |state| state.scm_provider != crate::scm::detection::ScmProvider::Unknown,
        execute: |_state| Action::None, // Handled in app.rs
    });

    registry.register(Command {
        id: "app.quit",
        name: "Quit",
        category: Category::Navigation,
        keybind: None,
        slash: Some("quit"),
        available: |_| true,
        execute: |_state| Action::Quit,
    });

    registry.register(Command {
        id: "workspace.list",
        name: "Workspaces",
        category: Category::Tools,
        keybind: None,
        slash: Some("workspace"),
        available: |_| true,
        execute: |_state| Action::None, // Handled in app.rs handle_workspace_command
    });

    registry.register(Command {
        id: "workspace.add",
        name: "Add Workspace",
        category: Category::Tools,
        keybind: None,
        slash: Some("workspace add"),
        available: |_| true,
        execute: |_state| Action::None, // Handled in app.rs handle_workspace_command
    });

    registry
}

#[cfg(test)]
mod workspace_command_tests {
    use super::*;

    #[test]
    fn workspace_command_registered() {
        let registry = build_default_registry();
        assert!(registry.find_slash("workspace").is_some());
    }

    #[test]
    fn workspace_add_command_registered() {
        let registry = build_default_registry();
        assert!(registry.find_slash("workspace add").is_some());
    }
}
