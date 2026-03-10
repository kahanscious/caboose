//! Persistent TUI preferences — survives across sessions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::tui::theme::ThemeVariant;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentModel {
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TuiPrefs {
    #[serde(default = "default_true")]
    pub sidebar_visible: bool,
    #[serde(default)]
    pub tool_expand_default: bool,
    /// Last-used provider ID — auto-connect on restart.
    #[serde(default)]
    pub last_provider: Option<String>,
    /// Last-used model ID — auto-select on restart.
    #[serde(default)]
    pub last_model: Option<String>,
    /// Most recently used models (newest first, max 5).
    #[serde(default)]
    pub recent_models: Vec<RecentModel>,
    /// Active theme variant.
    #[serde(default)]
    pub theme: ThemeVariant,
}

fn default_true() -> bool {
    true
}

const MAX_RECENT_MODELS: usize = 5;

impl Default for TuiPrefs {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            tool_expand_default: false,
            last_provider: None,
            last_model: None,
            recent_models: Vec::new(),
            theme: ThemeVariant::default(),
        }
    }
}

impl TuiPrefs {
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(
                &path,
                serde_json::to_string_pretty(self).unwrap_or_default(),
            );
        }
    }

    /// Record a model as recently used (moves to front, deduplicates, caps at 5).
    pub fn push_recent_model(&mut self, provider: &str, model_id: &str) {
        let entry = RecentModel {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
        };
        self.recent_models.retain(|e| e != &entry);
        self.recent_models.insert(0, entry);
        self.recent_models.truncate(MAX_RECENT_MODELS);
    }

    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("caboose").join("tui_prefs.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let prefs = TuiPrefs::default();
        assert!(prefs.sidebar_visible);
        assert!(!prefs.tool_expand_default);
    }

    #[test]
    fn round_trip_serde() {
        let prefs = TuiPrefs {
            sidebar_visible: false,
            tool_expand_default: true,
            last_provider: Some("openrouter".into()),
            last_model: Some("anthropic/claude-sonnet-4.6".into()),
            recent_models: vec![
                RecentModel {
                    provider: "openrouter".into(),
                    model_id: "anthropic/claude-sonnet-4.6".into(),
                },
                RecentModel {
                    provider: "anthropic".into(),
                    model_id: "claude-opus-4".into(),
                },
            ],
            theme: ThemeVariant::SteamDome,
        };
        let json = serde_json::to_string(&prefs).unwrap();
        let loaded: TuiPrefs = serde_json::from_str(&json).unwrap();
        assert!(!loaded.sidebar_visible);
        assert!(loaded.tool_expand_default);
        assert_eq!(loaded.last_provider.as_deref(), Some("openrouter"));
        assert_eq!(
            loaded.last_model.as_deref(),
            Some("anthropic/claude-sonnet-4.6")
        );
        assert_eq!(loaded.recent_models.len(), 2);
        assert_eq!(
            loaded.recent_models[0].model_id,
            "anthropic/claude-sonnet-4.6"
        );
    }

    #[test]
    fn push_recent_model_deduplicates_and_caps() {
        let mut prefs = TuiPrefs::default();
        prefs.push_recent_model("openrouter", "model-a");
        prefs.push_recent_model("openrouter", "model-b");
        prefs.push_recent_model("anthropic", "model-c");
        prefs.push_recent_model("openrouter", "model-d");
        prefs.push_recent_model("openrouter", "model-e");
        assert_eq!(prefs.recent_models.len(), 5);

        // Push a 6th — oldest should drop off
        prefs.push_recent_model("google", "model-f");
        assert_eq!(prefs.recent_models.len(), 5);
        assert_eq!(prefs.recent_models[0].model_id, "model-f");
        assert!(!prefs.recent_models.iter().any(|m| m.model_id == "model-a"));

        // Push duplicate — moves to front, no growth
        prefs.push_recent_model("openrouter", "model-d");
        assert_eq!(prefs.recent_models.len(), 5);
        assert_eq!(prefs.recent_models[0].model_id, "model-d");
    }

    #[test]
    fn corrupted_json_returns_defaults() {
        let result: Result<TuiPrefs, _> = serde_json::from_str("not json!!!");
        assert!(result.is_err());
        let fallback = result.unwrap_or_default();
        assert!(fallback.sidebar_visible);
        assert!(!fallback.tool_expand_default);
        assert!(fallback.recent_models.is_empty());
    }
}
