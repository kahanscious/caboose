//! Slash inline autocomplete — types, filtering, and rendering.

use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::provider::ModelInfo;
use crate::session::storage::SessionSearchResult;
use crate::tui::command::{Command, CommandRegistry};

/// What the dropdown is currently showing.
#[derive(Debug)]
pub enum DropdownMode {
    /// Default — shows slash commands and skills.
    Commands,
    /// /sessions — browsable session list.
    Sessions {
        results: Vec<SessionSearchResult>,
        confirm_delete: Option<usize>,
    },
    /// /model — model list from provider API.
    Models {
        models: Vec<(String, ModelInfo)>,
        error: Option<String>,
        /// Recently used models (newest first), shown at top of picker.
        recent: Vec<(String, ModelInfo)>,
    },
    /// /connect — provider catalog.
    Providers,
    /// /mcp — MCP server list with "Add new" option.
    McpServers {
        /// (name, status_label, tool_count, is_connected, is_preset, is_enabled, description)
        servers: Vec<(String, String, usize, bool, bool, bool, String)>,
    },
    /// Server action sub-menu after selecting a server from McpServers.
    McpServerActions {
        server_name: String,
        is_preset: bool,
    },
    /// /settings — settings toggles and preferences.
    Settings { items: Vec<SettingsItem> },
    /// /skills — interactive skill list with actions.
    Skills,
    /// /rewind — checkpoint list for file rewind.
    Checkpoints {
        /// (id, prompt_preview, age_label, file_count)
        items: Vec<(u32, String, String, usize)>,
    },
}

/// A single settings entry for the settings picker.
#[derive(Debug, Clone)]
pub struct SettingsItem {
    pub key: String,
    pub label: String,
    pub value: String,
    pub kind: SettingsKind,
}

/// What kind of control a settings item uses.
#[derive(Debug, Clone)]
pub enum SettingsKind {
    Toggle,
    Choice(Vec<String>),
}

/// State for the inline dropdown (slash autocomplete + picker modes).
#[derive(Debug)]
pub struct SlashAutoState {
    pub selected: usize,
    pub mode: DropdownMode,
    /// Filter text for picker modes (Commands mode uses input field instead).
    pub filter: String,
}

impl SlashAutoState {
    pub fn new() -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Commands,
            filter: String::new(),
        }
    }

    /// Create in picker mode with pre-loaded session data.
    pub fn with_sessions(results: Vec<SessionSearchResult>) -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Sessions {
                results,
                confirm_delete: None,
            },
            filter: String::new(),
        }
    }

    pub fn with_models(
        models: Vec<(String, ModelInfo)>,
        error: Option<String>,
        recent: Vec<(String, ModelInfo)>,
    ) -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Models {
                models,
                error,
                recent,
            },
            filter: String::new(),
        }
    }

    pub fn with_providers() -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Providers,
            filter: String::new(),
        }
    }

    pub fn with_mcp_servers(
        servers: Vec<(String, String, usize, bool, bool, bool, String)>,
    ) -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::McpServers { servers },
            filter: String::new(),
        }
    }

    pub fn with_mcp_server_actions(server_name: String, is_preset: bool) -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::McpServerActions {
                server_name,
                is_preset,
            },
            filter: String::new(),
        }
    }

    pub fn with_settings(items: Vec<SettingsItem>) -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Settings { items },
            filter: String::new(),
        }
    }

    pub fn with_skills() -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Skills,
            filter: String::new(),
        }
    }

    pub fn with_checkpoints(items: Vec<(u32, String, String, usize)>) -> Self {
        Self {
            selected: 0,
            mode: DropdownMode::Checkpoints { items },
            filter: String::new(),
        }
    }

    /// Whether we're in a picker mode (not Commands).
    pub fn is_picker(&self) -> bool {
        !matches!(self.mode, DropdownMode::Commands)
    }
}

/// A unified entry for the autocomplete dropdown.
pub enum SlashEntry<'a> {
    Command(&'a Command),
    Skill(&'a crate::skills::Skill),
}

impl<'a> SlashEntry<'a> {
    pub fn slash_name(&self) -> &str {
        match self {
            SlashEntry::Command(c) => c.slash.unwrap_or(""),
            SlashEntry::Skill(s) => &s.name,
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            SlashEntry::Command(c) => c.name,
            SlashEntry::Skill(s) => &s.description,
        }
    }
}

/// Extract the slash prefix from input (text after `/`, before any space).
/// Returns `None` if input doesn't start with `/` (after trimming leading whitespace).
pub fn slash_prefix(input: &str) -> Option<&str> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let after_slash = &trimmed[1..];
    // Take everything up to the first space (or all of it)
    Some(after_slash.split_whitespace().next().unwrap_or(after_slash))
}

/// Filter and return matching slash entries, split into (commands, skills).
/// Both lists are alphabetized by slash name. Prefix-matched against `prefix`.
pub fn filtered_entries<'a>(
    prefix: &str,
    registry: &'a CommandRegistry,
    skills: &'a [crate::skills::Skill],
) -> (Vec<SlashEntry<'a>>, Vec<SlashEntry<'a>>) {
    let prefix_lower = prefix.to_lowercase();

    let mut cmds: Vec<SlashEntry<'a>> = registry
        .slash_commands()
        .filter(|c| {
            c.slash
                .map(|s| s.to_lowercase().starts_with(&prefix_lower))
                .unwrap_or(false)
        })
        .map(SlashEntry::Command)
        .collect();
    cmds.sort_by(|a, b| a.slash_name().cmp(b.slash_name()));

    let mut skill_entries: Vec<SlashEntry<'a>> = skills
        .iter()
        .filter(|s| s.name.to_lowercase().starts_with(&prefix_lower))
        .map(SlashEntry::Skill)
        .collect();
    skill_entries.sort_by(|a, b| a.slash_name().cmp(b.slash_name()));

    (cmds, skill_entries)
}

/// Total number of filtered entries (commands + skills).
pub fn total_filtered(
    prefix: &str,
    registry: &CommandRegistry,
    skills: &[crate::skills::Skill],
) -> usize {
    let (cmds, skill_entries) = filtered_entries(prefix, registry, skills);
    cmds.len() + skill_entries.len()
}

/// Result from slash autocomplete key handling.
pub enum SlashKeyResult {
    /// Key was consumed — don't process further.
    Consumed,
    /// Key should fall through to normal input handling.
    /// After normal handling, caller must call `state.update_slash_auto()`.
    Fallthrough,
}

/// Handle a key press while slash autocomplete is active.
/// `input` is the current input string; `selected` is the current selection index.
/// Returns the action to take and optionally the new input text (for completion).
pub fn handle_slash_key(
    key: KeyCode,
    input: &str,
    selected: usize,
    registry: &CommandRegistry,
    skills: &[crate::skills::Skill],
) -> (SlashKeyResult, Option<String>) {
    let prefix = slash_prefix(input).unwrap_or("");

    match key {
        KeyCode::Up => (SlashKeyResult::Consumed, None),
        KeyCode::Down => (SlashKeyResult::Consumed, None),
        KeyCode::Esc => (SlashKeyResult::Consumed, None),
        KeyCode::Tab | KeyCode::Enter => {
            // Find the entry at `selected` index and complete
            let (cmds, skill_entries) = filtered_entries(prefix, registry, skills);
            let all: Vec<&SlashEntry> = cmds.iter().chain(skill_entries.iter()).collect();
            if let Some(entry) = all.get(selected) {
                let completed = format!("/{}", entry.slash_name());
                (SlashKeyResult::Consumed, Some(completed))
            } else {
                (SlashKeyResult::Consumed, None)
            }
        }
        // Char and Backspace fall through to normal input handling
        _ => (SlashKeyResult::Fallthrough, None),
    }
}

/// Determine a human-readable source label for a skill.
fn skill_source_label(source: &crate::skills::SkillSource) -> &'static str {
    match source {
        crate::skills::SkillSource::Builtin => "built-in",
        crate::skills::SkillSource::File(path) => {
            let s = path.to_string_lossy();
            if s.contains(".caboose") {
                "user \u{00b7} project"
            } else {
                "user \u{00b7} global"
            }
        }
    }
}

/// Filter skills by prefix needle, returning indices into the full skills slice.
pub fn filter_skills(skills: &[crate::skills::Skill], needle: &str) -> Vec<usize> {
    let lower = needle.to_lowercase();
    skills
        .iter()
        .enumerate()
        .filter(|(_, s)| lower.is_empty() || s.name.to_lowercase().contains(&lower))
        .map(|(i, _)| i)
        .collect()
}

/// Total selectable skills after filtering.
pub fn filtered_skill_count(skills: &[crate::skills::Skill], filter: &str) -> usize {
    filter_skills(skills, filter).len()
}

/// Build skill items for the dropdown.
fn build_skill_items<'a>(
    skills: &[crate::skills::Skill],
    filter: &str,
    selected: usize,
    colors: &crate::tui::theme::Colors,
    width: u16,
) -> (Vec<ListItem<'a>>, Option<usize>) {
    let filtered = filter_skills(skills, filter);

    let mut items: Vec<ListItem> = Vec::new();
    let mut list_selected: Option<usize> = None;

    if filtered.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No matching skills",
            Style::default().fg(colors.text_muted),
        ))));
        return (items, None);
    }

    // Section header
    items.push(ListItem::new(Line::from(Span::styled(
        " Skills  (Enter=invoke  d=disable  Del=delete)",
        Style::default().fg(colors.text_dim).bold(),
    ))));

    for (sel_idx, &skill_idx) in filtered.iter().enumerate() {
        if sel_idx == selected {
            list_selected = Some(items.len());
        }

        let skill = &skills[skill_idx];
        let source_label = skill_source_label(&skill.source);
        let name_str = format!("  /{}", skill.name);
        let desc_str = &skill.description;
        let tag = format!("[{source_label}]");

        let avail = (width as usize).saturating_sub(6);
        // Layout: name  description  [source]
        // Cap description to 60 chars max to keep rows clean
        let desc_max = avail
            .saturating_sub(name_str.len())
            .saturating_sub(tag.len())
            .saturating_sub(4)
            .min(60);
        let desc_truncated: String = if desc_str.len() > desc_max {
            format!(
                "{}...",
                &desc_str[..desc_str.floor_char_boundary(desc_max.saturating_sub(3))]
            )
        } else {
            desc_str.to_string()
        };
        let pad1 = 2usize;
        let pad2 = avail
            .saturating_sub(name_str.len())
            .saturating_sub(pad1)
            .saturating_sub(desc_truncated.len())
            .saturating_sub(tag.len())
            .max(1);

        let base_style = if sel_idx == selected {
            Style::default().bg(colors.bg_hover).fg(colors.text)
        } else {
            Style::default().fg(colors.text)
        };
        let tag_style = if sel_idx == selected {
            base_style
        } else {
            Style::default().fg(colors.text_dim)
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(name_str, base_style),
            Span::styled(" ".repeat(pad1), base_style),
            Span::styled(
                desc_truncated,
                if sel_idx == selected {
                    base_style
                } else {
                    Style::default().fg(colors.text_muted)
                },
            ),
            Span::styled(" ".repeat(pad2), base_style),
            Span::styled(tag, tag_style),
        ])));
    }

    (items, list_selected)
}

/// Build session items for the dropdown.
fn build_session_items<'a>(
    filtered: &[crate::tui::session_picker::FilteredSession],
    selected: usize,
    confirm_delete: Option<usize>,
    current_session_id: Option<&str>,
    colors: &crate::tui::theme::Colors,
    width: u16,
) -> (Vec<ListItem<'a>>, Option<usize>) {
    use crate::tui::session_picker::{format_relative_time, truncate_at_word_boundary};

    let mut items: Vec<ListItem> = Vec::new();
    let mut list_selected: Option<usize> = None;

    if filtered.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No matching sessions",
            Style::default().fg(colors.text_muted),
        ))));
        return (items, None);
    }

    // Section header
    items.push(ListItem::new(Line::from(Span::styled(
        " Sessions",
        Style::default().fg(colors.text_dim).bold(),
    ))));

    for (sel_idx, entry) in filtered.iter().enumerate() {
        if sel_idx == selected {
            list_selected = Some(items.len());
        }

        let session = &entry.session;
        let is_active = current_session_id
            .map(|id| id == session.id)
            .unwrap_or(false);
        let is_confirming = confirm_delete == Some(sel_idx);

        let title_str = session
            .title
            .as_deref()
            .map(|t| truncate_at_word_boundary(t, 40))
            .unwrap_or_else(|| {
                if session.id.len() >= 8 {
                    session.id[..8].to_string()
                } else {
                    session.id.clone()
                }
            });

        let active_marker = if is_active { "\u{25CF} " } else { "  " };

        let suffix = if is_confirming {
            " [Delete? y/n]".to_string()
        } else {
            let time_ago = format_relative_time(session.updated_at);
            let turns = session.turn_count;
            format!("  {turns}t  {time_ago}")
        };

        let avail = (width as usize).saturating_sub(6);
        let label_left = format!("{active_marker}{title_str}");
        let pad = avail
            .saturating_sub(label_left.len())
            .saturating_sub(suffix.len())
            .max(1);
        let label = format!("{label_left}{}{suffix}", " ".repeat(pad));

        let style = if sel_idx == selected {
            if is_confirming {
                Style::default().bg(Color::Red).fg(Color::White).bold()
            } else {
                Style::default().bg(colors.bg_hover).fg(colors.text)
            }
        } else if is_active {
            Style::default().fg(colors.brand)
        } else {
            Style::default().fg(colors.text)
        };

        items.push(ListItem::new(Line::from(Span::styled(label, style))));

        // Snippet line for content matches
        if let Some(snippet) = &entry.matched_snippet {
            let snippet_text = format!("    \"{snippet}\"");
            let max_width = (width as usize).saturating_sub(6);
            let truncated = if snippet_text.len() > max_width {
                format!(
                    "{}...",
                    &snippet_text[..snippet_text.floor_char_boundary(max_width.saturating_sub(3))]
                )
            } else {
                snippet_text
            };
            items.push(ListItem::new(Line::from(Span::styled(
                truncated,
                Style::default().fg(colors.text_muted),
            ))));
        }
    }

    (items, list_selected)
}

/// Filter models by needle, returning references in order.
fn filter_models<'a>(
    models: &'a [(String, ModelInfo)],
    needle: &str,
) -> Vec<&'a (String, ModelInfo)> {
    models
        .iter()
        .filter(|(provider, info)| {
            if needle.is_empty() {
                return true;
            }
            provider.to_lowercase().contains(needle)
                || info.id.to_lowercase().contains(needle)
                || info.name.to_lowercase().contains(needle)
        })
        .collect()
}

/// Total selectable items in the model picker (recent + models, both filtered).
pub fn filtered_model_count(
    models: &[(String, ModelInfo)],
    recent: &[(String, ModelInfo)],
    filter: &str,
) -> usize {
    let needle = filter.to_lowercase();
    let rc = filter_models(recent, &needle).len();
    let mc = filter_models(models, &needle).len();
    rc + mc
}

/// Resolve `selected` index to (provider, model_id) from the combined recent+models list.
pub fn resolve_model_selection(
    models: &[(String, ModelInfo)],
    recent: &[(String, ModelInfo)],
    filter: &str,
    selected: usize,
) -> Option<(String, String)> {
    let needle = filter.to_lowercase();
    let recent_filtered = filter_models(recent, &needle);
    let models_filtered = filter_models(models, &needle);
    if selected < recent_filtered.len() {
        recent_filtered
            .get(selected)
            .map(|(p, info)| (p.clone(), info.id.clone()))
    } else {
        let idx = selected - recent_filtered.len();
        models_filtered
            .get(idx)
            .map(|(p, info)| (p.clone(), info.id.clone()))
    }
}

/// Build model items for the dropdown (recent section + all models).
fn build_model_items<'a>(
    models: &[(String, ModelInfo)],
    recent: &[(String, ModelInfo)],
    filter: &str,
    selected: usize,
    error: Option<&str>,
    colors: &crate::tui::theme::Colors,
    width: u16,
) -> (Vec<ListItem<'a>>, Option<usize>) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut list_selected: Option<usize> = None;
    let mut logical_idx: usize = 0; // tracks position in the selectable items

    if let Some(err) = error {
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  Error: {err}"),
            Style::default().fg(Color::Red),
        ))));
        if models.is_empty() && recent.is_empty() {
            return (items, None);
        }
    }

    let needle = filter.to_lowercase();
    let recent_filtered = filter_models(recent, &needle);
    let models_filtered = filter_models(models, &needle);

    if recent_filtered.is_empty() && models_filtered.is_empty() && error.is_none() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No matching models",
            Style::default().fg(colors.text_muted),
        ))));
        return (items, None);
    }

    // Recent section
    if !recent_filtered.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " Recent",
            Style::default().fg(colors.text_dim).bold(),
        ))));
        for (provider, info) in &recent_filtered {
            if logical_idx == selected {
                list_selected = Some(items.len());
            }
            items.push(build_model_row(
                info,
                provider,
                logical_idx == selected,
                colors,
                width,
            ));
            logical_idx += 1;
        }
    }

    // All models section
    if !models_filtered.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " Models",
            Style::default().fg(colors.text_dim).bold(),
        ))));
        for (provider, info) in &models_filtered {
            if logical_idx == selected {
                list_selected = Some(items.len());
            }
            items.push(build_model_row(
                info,
                provider,
                logical_idx == selected,
                colors,
                width,
            ));
            logical_idx += 1;
        }
    }

    (items, list_selected)
}

/// Render a single model row.
fn build_model_row<'a>(
    info: &ModelInfo,
    provider: &str,
    is_selected: bool,
    colors: &crate::tui::theme::Colors,
    width: u16,
) -> ListItem<'a> {
    let label_left = format!("  {}", info.id);
    let label_right = provider.to_string();
    let avail = (width as usize).saturating_sub(6);
    let pad = avail
        .saturating_sub(label_left.len())
        .saturating_sub(label_right.len())
        .max(1);

    let style = if is_selected {
        Style::default().bg(colors.bg_hover).fg(colors.text)
    } else {
        Style::default().fg(colors.text)
    };

    ListItem::new(Line::from(vec![
        Span::styled(label_left, style),
        Span::styled(" ".repeat(pad), style),
        Span::styled(
            label_right,
            if is_selected {
                style
            } else {
                Style::default().fg(colors.text_dim)
            },
        ),
    ]))
}

/// Build provider items for the dropdown.
fn build_provider_items<'a>(
    filter: &str,
    selected: usize,
    colors: &crate::tui::theme::Colors,
    width: u16,
    discovered_locals: &[crate::provider::local::LocalServer],
) -> (Vec<ListItem<'a>>, Option<usize>) {
    use crate::provider::catalog;

    let needle = filter.to_lowercase();
    let filtered: Vec<&catalog::ProviderEntry> = catalog::CATALOG
        .iter()
        .filter(|e| {
            needle.is_empty()
                || e.display_name.to_lowercase().contains(&needle)
                || e.id.to_lowercase().contains(&needle)
        })
        .collect();

    let mut items: Vec<ListItem> = Vec::new();
    let mut list_selected: Option<usize> = None;
    let mut sel_idx = 0usize;

    let popular: Vec<_> = filtered.iter().filter(|e| e.popular).copied().collect();
    let other: Vec<_> = filtered.iter().filter(|e| !e.popular).copied().collect();

    // Helper: check if a catalog entry corresponds to a running local server.
    let is_running = |entry: &catalog::ProviderEntry| -> bool {
        use crate::provider::local::LocalServerType;
        let server_type = match entry.id {
            "ollama" => Some(LocalServerType::Ollama),
            "lmstudio" => Some(LocalServerType::LmStudio),
            "llamacpp" => Some(LocalServerType::LlamaCpp),
            _ => None,
        };
        server_type.is_some_and(|st| {
            discovered_locals
                .iter()
                .any(|s| s.server_type == st && s.available)
        })
    };

    if !popular.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " Popular",
            Style::default().fg(colors.text_dim).bold(),
        ))));
        for entry in &popular {
            if sel_idx == selected {
                list_selected = Some(items.len());
            }
            let label_left = format!("  {}", entry.display_name);
            let label_right = if is_running(entry) {
                format!("{} (running)", entry.description)
            } else {
                entry.description.to_string()
            };
            let avail = (width as usize).saturating_sub(6);
            let pad = avail
                .saturating_sub(label_left.len())
                .saturating_sub(label_right.len())
                .max(1);
            let style = if sel_idx == selected {
                Style::default().bg(colors.bg_hover).fg(colors.text)
            } else {
                Style::default().fg(colors.text)
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(label_left, style),
                Span::styled(" ".repeat(pad), style),
                Span::styled(
                    label_right,
                    if sel_idx == selected {
                        style
                    } else {
                        Style::default().fg(colors.text_dim)
                    },
                ),
            ])));
            sel_idx += 1;
        }
    }

    if !other.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " Other",
            Style::default().fg(colors.text_dim).bold(),
        ))));
        for entry in &other {
            if sel_idx == selected {
                list_selected = Some(items.len());
            }
            let label_left = format!("  {}", entry.display_name);
            let label_right = if is_running(entry) {
                format!("{} (running)", entry.description)
            } else {
                entry.description.to_string()
            };
            let avail = (width as usize).saturating_sub(6);
            let pad = avail
                .saturating_sub(label_left.len())
                .saturating_sub(label_right.len())
                .max(1);
            let style = if sel_idx == selected {
                Style::default().bg(colors.bg_hover).fg(colors.text)
            } else {
                Style::default().fg(colors.text)
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(label_left, style),
                Span::styled(" ".repeat(pad), style),
                Span::styled(
                    label_right,
                    if sel_idx == selected {
                        style
                    } else {
                        Style::default().fg(colors.text_dim)
                    },
                ),
            ])));
            sel_idx += 1;
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No matching providers",
            Style::default().fg(colors.text_muted),
        ))));
    }

    (items, list_selected)
}

/// Build MCP server items for the dropdown.
fn build_mcp_items<'a>(
    servers: &[(String, String, usize, bool, bool, bool, String)],
    selected: usize,
    colors: &crate::tui::theme::Colors,
    width: u16,
) -> (Vec<ListItem<'a>>, Option<usize>) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut list_selected: Option<usize> = None;
    let mut sel_idx: usize = 0;

    // "Add new server" entry
    if sel_idx == selected {
        list_selected = Some(items.len());
    }
    let style = if sel_idx == selected {
        Style::default().bg(colors.bg_hover).fg(colors.success)
    } else {
        Style::default().fg(colors.success)
    };
    items.push(ListItem::new(Line::from(Span::styled(
        "  + Add new server",
        style,
    ))));
    sel_idx += 1;

    // Split into presets and custom
    let presets: Vec<_> = servers.iter().filter(|s| s.4).collect();
    let custom: Vec<_> = servers.iter().filter(|s| !s.4).collect();

    // --- Built-in section ---
    if !presets.is_empty() {
        let sep_width = (width as usize).saturating_sub(6);
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {}", "\u{2500}".repeat(sep_width)),
            Style::default().fg(colors.border),
        ))));
        items.push(ListItem::new(Line::from(Span::styled(
            "  Built-in",
            Style::default().fg(colors.text_secondary).bold(),
        ))));

        for (
            name,
            _status_label,
            _tool_count,
            _is_connected,
            _is_preset,
            is_enabled,
            description,
        ) in &presets
        {
            if sel_idx == selected {
                list_selected = Some(items.len());
            }

            let toggle = if *is_enabled { "[on] " } else { "[off]" };
            let toggle_color = if *is_enabled {
                colors.success
            } else {
                colors.text_muted
            };

            let avail = (width as usize).saturating_sub(6);
            let desc_trunc: String = if description.len() + name.len() + 8 > avail {
                let max = avail.saturating_sub(name.len() + 10);
                format!("{}...", &description[..max.min(description.len())])
            } else {
                description.to_string()
            };

            let base_style = if sel_idx == selected {
                Style::default().bg(colors.bg_hover).fg(colors.text)
            } else {
                Style::default().fg(colors.text)
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {toggle} "),
                    if sel_idx == selected {
                        base_style
                    } else {
                        Style::default().fg(toggle_color)
                    },
                ),
                Span::styled(name.to_string(), base_style),
                Span::styled(
                    format!("  {desc_trunc}"),
                    if sel_idx == selected {
                        base_style
                    } else {
                        Style::default().fg(colors.text_dim)
                    },
                ),
            ])));
            sel_idx += 1;
        }
    }

    // --- Custom section ---
    if !custom.is_empty() {
        let sep_width = (width as usize).saturating_sub(6);
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {}", "\u{2500}".repeat(sep_width)),
            Style::default().fg(colors.border),
        ))));
        items.push(ListItem::new(Line::from(Span::styled(
            "  Custom",
            Style::default().fg(colors.text_secondary).bold(),
        ))));

        for (name, status_label, tool_count, is_connected, _is_preset, _is_enabled, _description) in
            &custom
        {
            if sel_idx == selected {
                list_selected = Some(items.len());
            }

            let dot = if *is_connected {
                "\u{25CF}"
            } else {
                "\u{25CB}"
            };
            let dot_color = if *is_connected {
                colors.success
            } else {
                colors.text_dim
            };
            let suffix = if *is_connected {
                format!("({tool_count})")
            } else {
                status_label.to_string()
            };

            let label_left = format!("  {dot} {name}");
            let avail = (width as usize).saturating_sub(6);
            let pad = avail
                .saturating_sub(label_left.len())
                .saturating_sub(suffix.len())
                .max(1);

            let base_style = if sel_idx == selected {
                Style::default().bg(colors.bg_hover).fg(colors.text)
            } else {
                Style::default().fg(colors.text)
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    format!("  {dot} "),
                    if sel_idx == selected {
                        base_style
                    } else {
                        Style::default().fg(dot_color)
                    },
                ),
                Span::styled(name.to_string(), base_style),
                Span::styled(" ".repeat(pad), base_style),
                Span::styled(
                    suffix,
                    if sel_idx == selected {
                        base_style
                    } else {
                        Style::default().fg(colors.text_dim)
                    },
                ),
            ])));
            sel_idx += 1;
        }
    }

    (items, list_selected)
}

/// Build MCP server action items for the sub-menu.
fn build_mcp_action_items<'a>(
    server_name: &str,
    _is_preset: bool,
    selected: usize,
    colors: &crate::tui::theme::Colors,
) -> (Vec<ListItem<'a>>, Option<usize>) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut list_selected: Option<usize> = None;

    // Header
    items.push(ListItem::new(Line::from(Span::styled(
        format!(" {server_name}"),
        Style::default().fg(colors.text_dim).bold(),
    ))));

    let actions: Vec<&str> = vec!["Restart", "Remove"];

    for (i, action) in actions.iter().enumerate() {
        if i == selected {
            list_selected = Some(items.len());
        }
        let style = if i == selected {
            Style::default().bg(colors.bg_hover).fg(colors.text)
        } else {
            Style::default().fg(colors.text)
        };
        items.push(ListItem::new(Line::from(Span::styled(
            format!("  {action}"),
            style,
        ))));
    }

    (items, list_selected)
}

/// Maximum height for the dropdown (entries + headers).
const MAX_DROPDOWN_ROWS: u16 = 14;

/// Render the inline dropdown — slash autocomplete or picker mode.
/// `anchor` is the Rect of the input area — dropdown renders above it (chat)
/// or below a given line (home). `above` controls direction.
#[allow(clippy::too_many_arguments)]
pub fn render_slash_autocomplete(
    frame: &mut ratatui::Frame,
    anchor: Rect,
    state: &SlashAutoState,
    input: &str,
    registry: &CommandRegistry,
    skills: &[crate::skills::Skill],
    colors: &crate::tui::theme::Colors,
    above: bool,
    current_session_id: Option<&str>,
    discovered_locals: &[crate::provider::local::LocalServer],
) {
    let (items, selected_item_idx, title) = match &state.mode {
        DropdownMode::Commands => {
            let prefix = match slash_prefix(input) {
                Some(p) => p,
                None => return,
            };
            let (cmds, skill_entries) = filtered_entries(prefix, registry, skills);
            if cmds.is_empty() && skill_entries.is_empty() {
                return;
            }

            let mut items: Vec<ListItem> = Vec::new();
            let mut selectable_indices: Vec<usize> = Vec::new();
            let mut item_idx: usize = 0;

            if !cmds.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    " Commands",
                    Style::default().fg(colors.text_dim).bold(),
                ))));
                item_idx += 1;
                for entry in &cmds {
                    let line = format_entry_line(entry, anchor.width, colors);
                    items.push(ListItem::new(line));
                    selectable_indices.push(item_idx);
                    item_idx += 1;
                }
            }

            if !skill_entries.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    " Skills",
                    Style::default().fg(colors.text_dim).bold(),
                ))));
                item_idx += 1;
                for entry in &skill_entries {
                    let line = format_entry_line(entry, anchor.width, colors);
                    items.push(ListItem::new(line));
                    selectable_indices.push(item_idx);
                    item_idx += 1;
                }
            }

            let selected = selectable_indices.get(state.selected).copied();
            (items, selected, None)
        }
        DropdownMode::Sessions {
            results,
            confirm_delete,
        } => {
            let filtered =
                crate::tui::session_picker::filter_search_results(results, &state.filter);
            let (items, selected) = build_session_items(
                &filtered,
                state.selected,
                *confirm_delete,
                current_session_id,
                colors,
                anchor.width,
            );
            (items, selected, Some(" /sessions "))
        }
        DropdownMode::Models {
            models,
            error,
            recent,
        } => {
            let (items, selected) = build_model_items(
                models,
                recent,
                &state.filter,
                state.selected,
                error.as_deref(),
                colors,
                anchor.width,
            );
            (items, selected, Some(" /model "))
        }
        DropdownMode::Providers => {
            let (items, selected) = build_provider_items(
                &state.filter,
                state.selected,
                colors,
                anchor.width,
                discovered_locals,
            );
            (items, selected, Some(" /connect "))
        }
        DropdownMode::McpServers { servers } => {
            let (items, selected) = build_mcp_items(servers, state.selected, colors, anchor.width);
            (items, selected, Some(" /mcp "))
        }
        DropdownMode::McpServerActions {
            server_name,
            is_preset,
        } => {
            let (items, selected) =
                build_mcp_action_items(server_name, *is_preset, state.selected, colors);
            (items, selected, Some(" /mcp "))
        }
        DropdownMode::Settings { items } => {
            let list_items: Vec<ListItem> = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let (indicator, value_style) = match item.kind {
                        SettingsKind::Toggle => {
                            let ind = if item.value == "on" { "[on] " } else { "[off]" };
                            let vs = if item.value == "on" {
                                Style::default().fg(colors.success)
                            } else {
                                Style::default().fg(colors.text_muted)
                            };
                            (ind.to_string(), vs)
                        }
                        SettingsKind::Choice(_) => {
                            let ind = format!("[{}]", item.value);
                            (ind, Style::default().fg(colors.brand))
                        }
                    };
                    let style = if i == state.selected {
                        Style::default().fg(colors.text)
                    } else {
                        Style::default().fg(colors.text_muted)
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" {indicator} "), value_style),
                        Span::styled(&item.label, style),
                    ]))
                })
                .collect();
            let selected = if items.is_empty() {
                None
            } else {
                Some(state.selected)
            };
            (list_items, selected, Some(" /settings "))
        }
        DropdownMode::Skills => {
            let (items, selected) =
                build_skill_items(skills, &state.filter, state.selected, colors, anchor.width);
            (items, selected, Some(" /skills "))
        }
        DropdownMode::Checkpoints { items: cp_items } => {
            let list_items: Vec<ListItem> = cp_items
                .iter()
                .enumerate()
                .rev() // Show newest first
                .map(|(i, (id, preview, age, file_count))| {
                    let style = if i == state.selected {
                        Style::default().fg(colors.text)
                    } else {
                        Style::default().fg(colors.text_muted)
                    };
                    let files_label = if *file_count > 0 {
                        format!("{file_count} file(s)")
                    } else {
                        "no files".to_string()
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!(" [{id}] "), Style::default().fg(colors.brand)),
                        Span::styled(format!("{preview}  "), style),
                        Span::styled(
                            format!("({age}, {files_label})"),
                            Style::default().fg(colors.text_dim),
                        ),
                    ]))
                })
                .collect();
            let selected = if cp_items.is_empty() {
                None
            } else {
                Some(state.selected)
            };
            (list_items, selected, Some(" /rewind "))
        }
    };

    // Compute dropdown area, clamped to fit within the terminal
    let frame_area = frame.area();
    let total_rows = items.len() as u16 + 2; // +2 for top/bottom border
    let height = total_rows.min(MAX_DROPDOWN_ROWS);
    let width = anchor.width;

    let dropdown_area = if above {
        Rect::new(anchor.x, anchor.y.saturating_sub(height), width, height)
    } else {
        // Clamp so the dropdown doesn't extend past the terminal bottom
        let max_height = frame_area.height.saturating_sub(anchor.y);
        let clamped_height = height.min(max_height);
        if clamped_height < 3 {
            return; // Not enough room to render
        }
        Rect::new(anchor.x, anchor.y, width, clamped_height)
    };

    frame.render_widget(Clear, dropdown_area);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.border_active))
        .style(Style::default().bg(colors.bg_elevated));
    if let Some(t) = title {
        block = block.title(t);
    }

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(colors.bg_hover).fg(colors.text))
        .highlight_symbol("\u{25b8} "); // ▸

    let mut list_state = ratatui::widgets::ListState::default();
    list_state.select(selected_item_idx);
    frame.render_stateful_widget(list, dropdown_area, &mut list_state);
}

/// Format a single entry line: "  /name   Description"
fn format_entry_line<'a>(
    entry: &SlashEntry,
    width: u16,
    colors: &crate::tui::theme::Colors,
) -> Line<'a> {
    let slash_name = format!("/{}", entry.slash_name());
    let desc = entry.display_name();
    let available = (width as usize).saturating_sub(6); // borders + highlight symbol + padding
    let pad = available
        .saturating_sub(slash_name.len())
        .saturating_sub(desc.len())
        .max(1);

    Line::from(vec![
        Span::styled(slash_name, Style::default().fg(colors.text)),
        Span::raw(" ".repeat(pad)),
        Span::styled(desc.to_string(), Style::default().fg(colors.text_dim)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_prefix_extracts_after_slash() {
        assert_eq!(slash_prefix("/model"), Some("model"));
        assert_eq!(slash_prefix("/mo"), Some("mo"));
        assert_eq!(slash_prefix("/"), Some(""));
        assert_eq!(slash_prefix("  /model"), Some("model"));
    }

    #[test]
    fn slash_prefix_none_without_slash() {
        assert_eq!(slash_prefix("hello"), None);
        assert_eq!(slash_prefix(""), None);
        assert_eq!(slash_prefix("hello /model"), None);
    }

    #[test]
    fn slash_prefix_stops_at_space() {
        assert_eq!(slash_prefix("/model arg1"), Some("model"));
    }

    #[test]
    fn filtered_entries_prefix_matches() {
        let mut registry = CommandRegistry::new();
        registry.register(crate::tui::command::Command {
            id: "test.model",
            name: "Switch Model",
            category: crate::tui::command::Category::Provider,
            keybind: None,
            slash: Some("model"),
            available: |_| true,
            execute: |_| crate::tui::command::Action::None,
        });
        registry.register(crate::tui::command::Command {
            id: "test.connect",
            name: "Connect",
            category: crate::tui::command::Category::Provider,
            keybind: None,
            slash: Some("connect"),
            available: |_| true,
            execute: |_| crate::tui::command::Action::None,
        });

        let skills = vec![crate::skills::Skill {
            name: "mocha".into(),
            description: "Run Mocha tests".into(),
            template: String::new(),
            source: crate::skills::SkillSource::Builtin,
            response_format: crate::skills::ResponseFormat::Prose,
        }];

        // "mo" should match "model" command and "mocha" skill
        let (cmds, sk) = filtered_entries("mo", &registry, &skills);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].slash_name(), "model");
        assert_eq!(sk.len(), 1);
        assert_eq!(sk[0].slash_name(), "mocha");

        // "" matches everything
        let (cmds, sk) = filtered_entries("", &registry, &skills);
        assert_eq!(cmds.len(), 2);
        assert_eq!(sk.len(), 1);

        // "z" matches nothing
        let (cmds, sk) = filtered_entries("z", &registry, &skills);
        assert_eq!(cmds.len(), 0);
        assert_eq!(sk.len(), 0);
    }

    #[test]
    fn filtered_entries_alphabetized() {
        let mut registry = CommandRegistry::new();
        registry.register(crate::tui::command::Command {
            id: "test.quit",
            name: "Quit",
            category: crate::tui::command::Category::Navigation,
            keybind: None,
            slash: Some("quit"),
            available: |_| true,
            execute: |_| crate::tui::command::Action::None,
        });
        registry.register(crate::tui::command::Command {
            id: "test.compact",
            name: "Compact",
            category: crate::tui::command::Category::Session,
            keybind: None,
            slash: Some("compact"),
            available: |_| true,
            execute: |_| crate::tui::command::Action::None,
        });

        let (cmds, _) = filtered_entries("", &registry, &[]);
        assert_eq!(cmds[0].slash_name(), "compact");
        assert_eq!(cmds[1].slash_name(), "quit");
    }

    #[test]
    fn handle_slash_key_tab_completes() {
        let mut registry = CommandRegistry::new();
        registry.register(crate::tui::command::Command {
            id: "test.model",
            name: "Switch Model",
            category: crate::tui::command::Category::Provider,
            keybind: None,
            slash: Some("model"),
            available: |_| true,
            execute: |_| crate::tui::command::Action::None,
        });

        let (result, completion) = handle_slash_key(KeyCode::Tab, "/mo", 0, &registry, &[]);
        assert!(matches!(result, SlashKeyResult::Consumed));
        assert_eq!(completion, Some("/model".to_string()));
    }

    #[test]
    fn handle_slash_key_esc_consumed() {
        let registry = CommandRegistry::new();
        let (result, completion) = handle_slash_key(KeyCode::Esc, "/", 0, &registry, &[]);
        assert!(matches!(result, SlashKeyResult::Consumed));
        assert!(completion.is_none());
    }

    #[test]
    fn handle_slash_key_char_falls_through() {
        let registry = CommandRegistry::new();
        let (result, _) = handle_slash_key(KeyCode::Char('a'), "/", 0, &registry, &[]);
        assert!(matches!(result, SlashKeyResult::Fallthrough));
    }

    #[test]
    fn with_mcp_servers_initializes() {
        let servers = vec![
            (
                "github".to_string(),
                "connected".to_string(),
                3,
                true,
                false,
                true,
                String::new(),
            ),
            (
                "context7".to_string(),
                "disconnected".to_string(),
                0,
                false,
                true,
                false,
                "Docs".to_string(),
            ),
        ];
        let state = SlashAutoState::with_mcp_servers(servers);
        assert_eq!(state.selected, 0);
        assert!(matches!(state.mode, DropdownMode::McpServers { .. }));
        assert!(state.is_picker());
    }

    #[test]
    fn with_mcp_server_actions_initializes() {
        let state = SlashAutoState::with_mcp_server_actions("github".to_string(), false);
        assert_eq!(state.selected, 0);
        assert!(matches!(state.mode, DropdownMode::McpServerActions { .. }));
        assert!(state.is_picker());
    }

    #[test]
    fn skills_dropdown_is_picker() {
        let auto = SlashAutoState::with_skills();
        assert!(auto.is_picker());
        assert!(matches!(auto.mode, DropdownMode::Skills));
        assert_eq!(auto.selected, 0);
        assert!(auto.filter.is_empty());
    }

    #[test]
    fn filter_skills_empty_needle_returns_all() {
        let skills = vec![
            crate::skills::Skill {
                name: "alpha".into(),
                description: "".into(),
                template: "".into(),
                source: crate::skills::SkillSource::Builtin,
                response_format: crate::skills::types::ResponseFormat::Prose,
            },
            crate::skills::Skill {
                name: "beta".into(),
                description: "".into(),
                template: "".into(),
                source: crate::skills::SkillSource::Builtin,
                response_format: crate::skills::types::ResponseFormat::Prose,
            },
        ];
        let filtered = filter_skills(&skills, "");
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_skills_narrows_by_name() {
        let skills = vec![
            crate::skills::Skill {
                name: "deploy".into(),
                description: "".into(),
                template: "".into(),
                source: crate::skills::SkillSource::Builtin,
                response_format: crate::skills::types::ResponseFormat::Prose,
            },
            crate::skills::Skill {
                name: "debug".into(),
                description: "".into(),
                template: "".into(),
                source: crate::skills::SkillSource::Builtin,
                response_format: crate::skills::types::ResponseFormat::Prose,
            },
        ];
        let filtered = filter_skills(&skills, "dep");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], 0);
    }

    #[test]
    fn filtered_skill_count_matches_filter() {
        let skills = vec![crate::skills::Skill {
            name: "commit".into(),
            description: "".into(),
            template: "".into(),
            source: crate::skills::SkillSource::Builtin,
            response_format: crate::skills::types::ResponseFormat::Prose,
        }];
        assert_eq!(filtered_skill_count(&skills, ""), 1);
        assert_eq!(filtered_skill_count(&skills, "com"), 1);
        assert_eq!(filtered_skill_count(&skills, "xyz"), 0);
    }
}
