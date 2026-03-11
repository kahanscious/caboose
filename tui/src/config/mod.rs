//! Configuration — global + project config loading, API key management.

pub mod auth;
pub mod keys;
pub mod prefs;
pub mod schema;

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Per-provider configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Default model for this provider
    pub model: Option<String>,
    /// Custom base URL (for self-hosted or proxy endpoints)
    pub base_url: Option<String>,
    /// Max tokens override
    pub max_tokens: Option<u32>,
}

/// Application configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Default provider (e.g. "anthropic", "openai")
    pub default_provider: Option<String>,
    /// Default model (e.g. "claude-sonnet-4-20250514")
    pub default_model: Option<String>,
    /// API keys (loaded from env, config file, or keyring)
    #[serde(default)]
    pub keys: keys::ApiKeys,
    /// Custom system prompt
    pub system_prompt: Option<String>,
    /// Per-provider configuration sections [providers.openai], etc.
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// Tools configuration (allow/deny commands, additional secrets)
    #[serde(default)]
    pub tools: Option<schema::ToolsConfig>,
    /// LSP servers configuration
    #[serde(default)]
    pub lsp: Option<schema::LspConfig>,
    /// MCP servers configuration
    #[serde(default)]
    pub mcp: Option<schema::McpConfig>,
    /// Memory system configuration
    #[serde(default)]
    pub memory: Option<schema::MemoryConfig>,
    /// Skills configuration
    #[serde(default)]
    pub skills: Option<schema::SkillsConfig>,
    /// Behavior configuration
    #[serde(default)]
    pub behavior: Option<schema::BehaviorConfig>,
    /// External services configuration (web search, etc.)
    #[serde(default)]
    pub services: Option<schema::ServicesConfig>,
    /// Default permission mode (plan, default, auto-edit, chug)
    pub permission_mode: Option<String>,
    /// Lifecycle hooks configuration
    #[serde(default)]
    pub hooks: Option<schema::HooksConfig>,
    /// Roundhouse (multi-LLM planning) configuration
    #[serde(default)]
    pub roundhouse: Option<schema::RoundhouseSchemaConfig>,
    /// Circuits (scheduled tasks) configuration
    #[serde(default)]
    pub circuits: Option<schema::CircuitsConfig>,
    /// SCM (GitHub/GitLab) integration configuration
    #[serde(default)]
    pub scm: Option<schema::ScmConfig>,
    /// Local LLM provider instances (Ollama, LM Studio, llama.cpp, custom)
    #[serde(default)]
    pub local_providers: HashMap<String, schema::LocalProviderConfig>,
}

impl Config {
    /// Load config from global + project files, with project overriding global.
    pub fn load() -> Result<Self> {
        let mut config = Config::default();

        // Load global config
        if let Some(global_path) = Self::global_config_path()
            && global_path.exists()
        {
            let content = std::fs::read_to_string(&global_path)?;
            let global: Config = toml::from_str(&content)?;
            config.merge(global);
        }

        // Load project config (.caboose/config.toml)
        let project_path = PathBuf::from(".caboose/config.toml");
        if project_path.exists() {
            let content = std::fs::read_to_string(&project_path)?;
            let project: Config = toml::from_str(&content)?;
            config.merge(project);
        }

        // Load API keys from environment (fills gaps only)
        config.keys.load_from_env();

        // Load API keys from auth.json (highest priority — overwrites)
        if let Some(auth_path) = auth::AuthStore::default_path() {
            let store = auth::AuthStore::new(auth_path);
            config.keys.load_from_auth(&store);
        }

        Ok(config)
    }

    /// Merge another config into this one (other takes precedence).
    fn merge(&mut self, other: Config) {
        if other.default_provider.is_some() {
            self.default_provider = other.default_provider;
        }
        if other.default_model.is_some() {
            self.default_model = other.default_model;
        }
        if other.system_prompt.is_some() {
            self.system_prompt = other.system_prompt;
        }
        if let Some(other_tools) = other.tools {
            let self_tools = self.tools.get_or_insert_with(Default::default);
            if other_tools.allow_commands.is_some() {
                self_tools.allow_commands = other_tools.allow_commands;
            }
            if other_tools.deny_commands.is_some() {
                self_tools.deny_commands = other_tools.deny_commands;
            }
            if other_tools.additional_secret_names.is_some() {
                self_tools.additional_secret_names = other_tools.additional_secret_names;
            }
            if let Some(other_registry) = other_tools.registry {
                let self_registry = self_tools.registry.get_or_insert_with(Default::default);
                for (name, config) in other_registry {
                    self_registry.insert(name, config);
                }
            }
            if let Some(other_executable) = other_tools.executable {
                let self_executable = self_tools.executable.get_or_insert_with(Default::default);
                for (name, config) in other_executable {
                    self_executable.insert(name, config);
                }
            }
        }
        if let Some(other_lsp) = other.lsp {
            let self_lsp = self.lsp.get_or_insert_with(Default::default);
            if !other_lsp.enabled {
                self_lsp.enabled = false;
            }
            for (name, cfg) in other_lsp.servers {
                self_lsp.servers.insert(name, cfg);
            }
        }
        if let Some(other_mcp) = other.mcp {
            let self_mcp = self.mcp.get_or_insert_with(Default::default);
            for (name, cfg) in other_mcp.servers {
                self_mcp.servers.insert(name, cfg);
            }
        }
        if other.memory.is_some() {
            self.memory = other.memory;
        }
        if other.skills.is_some() {
            self.skills = other.skills;
        }
        if other.behavior.is_some() {
            self.behavior = other.behavior;
        }
        if other.services.is_some() {
            self.services = other.services;
        }
        if other.permission_mode.is_some() {
            self.permission_mode = other.permission_mode;
        }
        if let Some(other_hooks) = other.hooks {
            let self_hooks = self.hooks.get_or_insert_with(Default::default);
            self_hooks.session_start.extend(other_hooks.session_start);
            self_hooks.session_end.extend(other_hooks.session_end);
            self_hooks
                .user_prompt_submit
                .extend(other_hooks.user_prompt_submit);
            self_hooks.pre_tool_use.extend(other_hooks.pre_tool_use);
            self_hooks.post_tool_use.extend(other_hooks.post_tool_use);
            self_hooks
                .post_tool_use_failure
                .extend(other_hooks.post_tool_use_failure);
            self_hooks
                .permission_request
                .extend(other_hooks.permission_request);
            self_hooks.stop.extend(other_hooks.stop);
            self_hooks.pre_compact.extend(other_hooks.pre_compact);
            self_hooks.notification.extend(other_hooks.notification);
            self_hooks.subagent_start.extend(other_hooks.subagent_start);
            self_hooks.subagent_stop.extend(other_hooks.subagent_stop);
            self_hooks.setup.extend(other_hooks.setup);
        }
        if other.roundhouse.is_some() {
            self.roundhouse = other.roundhouse;
        }
        if other.circuits.is_some() {
            self.circuits = other.circuits;
        }
        if other.scm.is_some() {
            self.scm = other.scm;
        }
        for (name, local) in other.local_providers {
            self.local_providers.entry(name).or_insert(local);
        }
        self.keys.merge(other.keys);
        for (name, other_cfg) in other.providers {
            let entry = self.providers.entry(name).or_default();
            if other_cfg.model.is_some() {
                entry.model = other_cfg.model;
            }
            if other_cfg.base_url.is_some() {
                entry.base_url = other_cfg.base_url;
            }
            if other_cfg.max_tokens.is_some() {
                entry.max_tokens = other_cfg.max_tokens;
            }
        }
    }

    /// Global config directory path.
    fn global_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("caboose").join("config.toml"))
    }
}

/// Persist the skills.disabled list to the appropriate config file.
pub fn save_skills_disabled(disabled: &[String], project_config_exists: bool) {
    let path = if project_config_exists {
        PathBuf::from(".caboose/config.toml")
    } else {
        match dirs::config_dir() {
            Some(d) => d.join("caboose").join("config.toml"),
            None => return,
        }
    };

    // Read existing config, update skills.disabled, write back
    let mut config: toml::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = config.as_table_mut().unwrap();
    let skills = table
        .entry("skills")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    let disabled_val: Vec<toml::Value> = disabled
        .iter()
        .map(|s| toml::Value::String(s.clone()))
        .collect();
    skills.insert("disabled".into(), toml::Value::Array(disabled_val));

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, toml::to_string_pretty(&config).unwrap_or_default());
}

/// Persist a single MCP server's disabled state to project config.
///
/// If the server entry doesn't exist yet in config, creates it with the preset's
/// default config. If it does exist, only updates the `disabled` field.
pub fn save_mcp_server_toggle(name: &str, config: &schema::McpServerConfig) {
    let path = PathBuf::from(".caboose/config.toml");

    let mut doc: toml::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = doc.as_table_mut().unwrap();
    let mcp = table
        .entry("mcp")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();
    let servers = mcp
        .entry("servers")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    // Serialize the full server config and insert/replace
    if let Ok(val) = toml::Value::try_from(config) {
        servers.insert(name.to_string(), val);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, toml::to_string_pretty(&doc).unwrap_or_default());
}

/// Persist the behavior.max_session_cost to the global config file.
pub fn save_behavior_max_session_cost(max_cost: Option<f64>) {
    let path = match dirs::config_dir() {
        Some(d) => d.join("caboose").join("config.toml"),
        None => return,
    };

    let mut config: toml::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = config.as_table_mut().unwrap();
    let behavior = table
        .entry("behavior")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    match max_cost {
        Some(cost) => {
            behavior.insert("max_session_cost".into(), toml::Value::Float(cost));
        }
        None => {
            behavior.remove("max_session_cost");
        }
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, toml::to_string_pretty(&config).unwrap_or_default());
}

/// Persist a local provider entry to the global config file.
///
/// Creates or replaces the `[local_providers.<name>]` section in the global
/// config, leaving all other sections untouched.
pub fn save_local_provider(name: &str, config: &schema::LocalProviderConfig) {
    let path = match dirs::config_dir() {
        Some(d) => d.join("caboose").join("config.toml"),
        None => return,
    };

    let mut doc: toml::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    let table = doc.as_table_mut().unwrap();
    let local_providers = table
        .entry("local_providers")
        .or_insert(toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .unwrap();

    if let Ok(val) = toml::Value::try_from(config) {
        local_providers.insert(name.to_string(), val);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, toml::to_string_pretty(&doc).unwrap_or_default());
}

/// Remove a custom MCP server entry from project config.
pub fn remove_mcp_server(name: &str) {
    let path = PathBuf::from(".caboose/config.toml");

    let mut doc: toml::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or(toml::Value::Table(toml::map::Map::new()))
    } else {
        return;
    };

    if let Some(servers) = doc
        .get_mut("mcp")
        .and_then(|m| m.get_mut("servers"))
        .and_then(|s| s.as_table_mut())
    {
        servers.remove(name);
    }

    let _ = std::fs::write(&path, toml::to_string_pretty(&doc).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_with_mcp_section() {
        let toml_str = r#"
[mcp.servers.test]
command = "echo"
args = ["hello"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let mcp = config.mcp.unwrap();
        assert_eq!(mcp.servers.len(), 1);
        assert_eq!(mcp.servers["test"].command, "echo");
    }

    #[test]
    fn config_without_mcp_section() {
        let toml_str = r#"
default_provider = "anthropic"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.mcp.is_none());
    }

    #[test]
    fn config_merge_mcp() {
        let mut base: Config = toml::from_str(
            r#"
[mcp.servers.base_server]
command = "base-cmd"
"#,
        )
        .unwrap();

        let overlay: Config = toml::from_str(
            r#"
[mcp.servers.overlay_server]
command = "overlay-cmd"
"#,
        )
        .unwrap();

        base.merge(overlay);
        let mcp = base.mcp.unwrap();
        // Project servers add to global — both should be present
        assert_eq!(mcp.servers.len(), 2);
        assert!(mcp.servers.contains_key("base_server"));
        assert!(mcp.servers.contains_key("overlay_server"));
    }

    #[test]
    fn config_merge_memory() {
        let mut base: Config = toml::from_str(
            r#"
[memory]
enabled = true
"#,
        )
        .unwrap();

        let overlay: Config = toml::from_str(
            r#"
[memory]
enabled = false
"#,
        )
        .unwrap();

        base.merge(overlay);
        assert!(!base.memory.unwrap().enabled);
    }

    #[test]
    fn config_merge_mcp_same_name_overrides() {
        let mut base: Config = toml::from_str(
            r#"
[mcp.servers.shared]
command = "global-cmd"
"#,
        )
        .unwrap();

        let overlay: Config = toml::from_str(
            r#"
[mcp.servers.shared]
command = "project-cmd"
"#,
        )
        .unwrap();

        base.merge(overlay);
        let mcp = base.mcp.unwrap();
        assert_eq!(mcp.servers.len(), 1);
        assert_eq!(mcp.servers["shared"].command, "project-cmd");
    }

    #[test]
    fn merge_overrides_default_provider_and_model() {
        let mut base = Config {
            default_provider: Some("anthropic".into()),
            default_model: Some("old-model".into()),
            ..Default::default()
        };
        let project = Config {
            default_provider: Some("openai".into()),
            default_model: Some("gpt-4o".into()),
            ..Default::default()
        };
        base.merge(project);
        assert_eq!(base.default_provider.as_deref(), Some("openai"));
        assert_eq!(base.default_model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn merge_preserves_base_when_other_is_none() {
        let mut base = Config {
            default_provider: Some("anthropic".into()),
            system_prompt: Some("be helpful".into()),
            ..Default::default()
        };
        let project = Config::default();
        base.merge(project);
        assert_eq!(base.default_provider.as_deref(), Some("anthropic"));
        assert_eq!(base.system_prompt.as_deref(), Some("be helpful"));
    }

    #[test]
    fn merge_permission_mode_from_project() {
        let mut base = Config::default();
        let project = Config {
            permission_mode: Some("chug".into()),
            ..Default::default()
        };
        base.merge(project);
        assert_eq!(base.permission_mode.as_deref(), Some("chug"));
    }

    #[test]
    fn merge_provider_config_partial_fields() {
        let mut base = Config::default();
        base.providers.insert(
            "openai".into(),
            ProviderConfig {
                model: Some("gpt-4o".into()),
                base_url: Some("https://custom.api".into()),
                max_tokens: Some(4096),
            },
        );
        let project = Config {
            providers: {
                let mut m = HashMap::new();
                m.insert(
                    "openai".into(),
                    ProviderConfig {
                        model: Some("gpt-4o-mini".into()),
                        base_url: None,
                        max_tokens: None,
                    },
                );
                m
            },
            ..Default::default()
        };
        base.merge(project);
        let openai = base.providers.get("openai").unwrap();
        assert_eq!(openai.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(openai.base_url.as_deref(), Some("https://custom.api"));
        assert_eq!(openai.max_tokens, Some(4096));
    }

    #[test]
    fn merge_tools_config_replaced_entirely() {
        let mut base = Config {
            tools: Some(schema::ToolsConfig {
                allow_commands: Some(vec!["ls".into()]),
                deny_commands: None,
                additional_secret_names: None,
                registry: None,
                executable: None,
            }),
            ..Default::default()
        };
        let project = Config {
            tools: Some(schema::ToolsConfig {
                allow_commands: Some(vec!["cat".into(), "grep".into()]),
                deny_commands: Some(vec!["rm".into()]),
                additional_secret_names: None,
                registry: None,
                executable: None,
            }),
            ..Default::default()
        };
        base.merge(project);
        let tools = base.tools.unwrap();
        assert_eq!(tools.allow_commands.unwrap(), vec!["cat", "grep"]);
        assert_eq!(tools.deny_commands.unwrap(), vec!["rm"]);
    }

    #[test]
    fn config_merge_tools_registry_per_tool() {
        let mut base = Config {
            tools: Some(schema::ToolsConfig {
                allow_commands: Some(vec!["ls".into()]),
                deny_commands: None,
                additional_secret_names: None,
                registry: Some({
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "global_test".into(),
                        schema::CliToolConfig {
                            command: "cargo test".into(),
                            description: "Global test".into(),
                            args: None,
                            permission: None,
                            output_format: None,
                            max_output_lines: None,
                        },
                    );
                    m
                }),
                executable: None,
            }),
            ..Default::default()
        };
        let project = Config {
            tools: Some(schema::ToolsConfig {
                allow_commands: Some(vec!["cat".into()]),
                deny_commands: Some(vec!["rm".into()]),
                additional_secret_names: None,
                registry: Some({
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "project_deploy".into(),
                        schema::CliToolConfig {
                            command: "make deploy".into(),
                            description: "Project deploy".into(),
                            args: None,
                            permission: None,
                            output_format: None,
                            max_output_lines: None,
                        },
                    );
                    m
                }),
                executable: None,
            }),
            ..Default::default()
        };
        base.merge(project);
        let tools = base.tools.unwrap();
        // allow/deny still replaced entirely
        assert_eq!(tools.allow_commands.unwrap(), vec!["cat"]);
        assert_eq!(tools.deny_commands.unwrap(), vec!["rm"]);
        // registry merged per-tool: both should be present
        let registry = tools.registry.unwrap();
        assert_eq!(registry.len(), 2);
        assert!(registry.contains_key("global_test"));
        assert!(registry.contains_key("project_deploy"));
    }

    #[test]
    fn config_merge_tools_registry_project_overrides_same_name() {
        let mut base = Config {
            tools: Some(schema::ToolsConfig {
                allow_commands: None,
                deny_commands: None,
                additional_secret_names: None,
                registry: Some({
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "test".into(),
                        schema::CliToolConfig {
                            command: "cargo test".into(),
                            description: "Global".into(),
                            args: None,
                            permission: None,
                            output_format: None,
                            max_output_lines: None,
                        },
                    );
                    m
                }),
                executable: None,
            }),
            ..Default::default()
        };
        let project = Config {
            tools: Some(schema::ToolsConfig {
                allow_commands: None,
                deny_commands: None,
                additional_secret_names: None,
                registry: Some({
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "test".into(),
                        schema::CliToolConfig {
                            command: "npm test".into(),
                            description: "Project".into(),
                            args: None,
                            permission: None,
                            output_format: None,
                            max_output_lines: None,
                        },
                    );
                    m
                }),
                executable: None,
            }),
            ..Default::default()
        };
        base.merge(project);
        let registry = base.tools.unwrap().registry.unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry["test"].command, "npm test");
    }

    #[test]
    fn config_merge_lsp() {
        let mut base: Config = toml::from_str(
            r#"
[lsp.servers.typescript]
command = "global-ts"
"#,
        )
        .unwrap();

        let overlay: Config = toml::from_str(
            r#"
[lsp.servers.typescript]
command = "project-ts"

[lsp.servers.rust]
command = "rust-analyzer"
"#,
        )
        .unwrap();

        base.merge(overlay);
        let lsp = base.lsp.unwrap();
        assert_eq!(lsp.servers.len(), 2);
        assert_eq!(lsp.servers["typescript"].command, "project-ts");
        assert_eq!(lsp.servers["rust"].command, "rust-analyzer");
    }

    #[test]
    fn save_mcp_server_toggle_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        // Write initial content with an existing section
        std::fs::write(&path, "default_provider = \"anthropic\"\n").unwrap();

        // Simulate save_mcp_server_toggle by writing to the temp path
        let config = schema::McpServerConfig {
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@upstash/context7-mcp@latest".to_string()],
            env: std::collections::HashMap::new(),
            disabled: false,
            removed: false,
        };

        let mut doc: toml::Value = {
            let content = std::fs::read_to_string(&path).unwrap();
            toml::from_str(&content).unwrap()
        };
        let table = doc.as_table_mut().unwrap();
        let mcp = table
            .entry("mcp")
            .or_insert(toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .unwrap();
        let servers = mcp
            .entry("servers")
            .or_insert(toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .unwrap();
        servers.insert(
            "context7".to_string(),
            toml::Value::try_from(&config).unwrap(),
        );
        std::fs::write(&path, toml::to_string_pretty(&doc).unwrap()).unwrap();

        // Read back and verify
        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(loaded.default_provider.as_deref(), Some("anthropic"));
        let mcp = loaded.mcp.unwrap();
        assert!(!mcp.servers["context7"].disabled);
        assert_eq!(mcp.servers["context7"].command, "npx");
    }

    #[test]
    fn config_with_services_section() {
        let toml_str = r#"
[services.web_search]
provider = "tavily"
api_key_env = "TAVILY_API_KEY"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let services = config.services.unwrap();
        assert_eq!(services.services["web_search"].provider, "tavily");
    }

    #[test]
    fn config_merge_cli_tool_registry() {
        let mut base: Config = toml::from_str(
            r#"
[tools.registry.test]
command = "cargo test"
description = "Run tests"
"#,
        )
        .unwrap();

        let overlay: Config = toml::from_str(
            r#"
[tools.registry.deploy]
command = "make deploy"
description = "Deploy"
"#,
        )
        .unwrap();

        base.merge(overlay);
        let registry = base.tools.unwrap().registry.unwrap();
        // Merge is per-tool, so both tools are present
        assert_eq!(registry.len(), 2);
        assert!(registry.contains_key("test"));
        assert!(registry.contains_key("deploy"));
    }

    #[test]
    fn config_merge_services() {
        let mut base: Config = toml::from_str(
            r#"
[services.web_search]
provider = "tavily"
api_key_env = "TAVILY_API_KEY"
"#,
        )
        .unwrap();

        let overlay: Config = toml::from_str(
            r#"
[services.web_search]
provider = "brave"
api_key_env = "BRAVE_API_KEY"
"#,
        )
        .unwrap();

        base.merge(overlay);
        let services = base.services.unwrap();
        assert_eq!(services.services["web_search"].provider, "brave");
    }

    #[test]
    fn save_and_load_skills_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        // Write some initial content
        std::fs::write(&path, "[skills]\nawareness = true\n").unwrap();

        // Simulate save_skills_disabled by writing to the temp path directly
        let disabled = vec!["debug".to_string(), "deploy".to_string()];
        let mut config: toml::Value = {
            let content = std::fs::read_to_string(&path).unwrap();
            toml::from_str(&content).unwrap()
        };
        let table = config.as_table_mut().unwrap();
        let skills = table
            .entry("skills")
            .or_insert(toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .unwrap();
        let disabled_val: Vec<toml::Value> = disabled
            .iter()
            .map(|s| toml::Value::String(s.clone()))
            .collect();
        skills.insert("disabled".into(), toml::Value::Array(disabled_val));
        std::fs::write(&path, toml::to_string_pretty(&config).unwrap()).unwrap();

        // Read back and verify
        let content = std::fs::read_to_string(&path).unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        let skills_cfg = loaded.skills.unwrap();
        assert_eq!(skills_cfg.disabled, vec!["debug", "deploy"]);
        assert!(skills_cfg.awareness); // preserved
    }

    #[test]
    fn full_config_to_registry_round_trip() {
        let toml_str = r#"
default_provider = "anthropic"

[tools]
allow_commands = ["cargo"]
deny_commands = ["rm"]

[tools.registry.test]
command = "cargo test"
description = "Run tests"

[tools.registry.deploy]
command = "make deploy"
description = "Deploy"
permission = "always_approve"

[tools.registry.deploy.args]
env = { type = "string", required = true }

[[hooks.PreToolUse]]
command = "echo check"
timeout = 5

[[hooks.SessionStart]]
command = "echo started"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();

        // Tools config
        let tools = config.tools.as_ref().unwrap();
        let registry = tools.registry.as_ref().unwrap();
        assert_eq!(registry.len(), 2);
        assert_eq!(registry["test"].command, "cargo test");
        assert_eq!(
            registry["deploy"].permission.as_deref(),
            Some("always_approve")
        );

        // Hooks config
        let hooks = config.hooks.as_ref().unwrap();
        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert_eq!(hooks.session_start.len(), 1);

        // ToolRegistry wiring
        let tool_reg = crate::tools::ToolRegistry::new(
            tools.registry.as_ref(),
            None,
            &crate::scm::detection::ScmProvider::Unknown,
        );
        let defs = tool_reg.definitions();
        assert!(defs.iter().any(|d| d.name == "cli_test"));
        assert!(defs.iter().any(|d| d.name == "cli_deploy"));

        // Deploy should have typed args
        let deploy_def = defs.iter().find(|d| d.name == "cli_deploy").unwrap();
        let props = deploy_def.input_schema.get("properties").unwrap();
        assert!(props.get("env").is_some());
    }

    #[test]
    fn merge_executable_tools_per_tool() {
        let mut global = Config::default();
        global.tools = Some(schema::ToolsConfig {
            executable: Some({
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "lint".into(),
                    schema::ExecutableToolConfig {
                        path: "global-lint.sh".into(),
                        timeout: None,
                        permission: None,
                        description: Some("Global lint".into()),
                        args: None,
                    },
                );
                m
            }),
            ..Default::default()
        });

        let project = Config {
            tools: Some(schema::ToolsConfig {
                executable: Some({
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "test".into(),
                        schema::ExecutableToolConfig {
                            path: "test.py".into(),
                            timeout: Some(120),
                            permission: None,
                            description: Some("Project test".into()),
                            args: None,
                        },
                    );
                    m
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        global.merge(project);
        let exec = global.tools.unwrap().executable.unwrap();
        assert_eq!(exec.len(), 2);
        assert!(exec.contains_key("lint"));
        assert!(exec.contains_key("test"));
    }
}
