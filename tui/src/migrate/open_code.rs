use std::path::PathBuf;

/// Items discoverable from an Open Code installation
#[derive(Debug, Clone, Default)]
pub struct OpenCodeConfig {
    pub config_path: Option<PathBuf>,
    pub mcp_servers: Vec<(String, serde_json::Value)>,
    pub system_prompt: Option<String>,
}

/// Scan Open Code config directories
pub fn scan_open_code(config_dirs: &[PathBuf]) -> OpenCodeConfig {
    let mut result = OpenCodeConfig::default();

    for dir in config_dirs {
        let config_file = dir.join("config.json");
        if config_file.exists() {
            result.config_path = Some(config_file.clone());
            // TODO: parse Open Code config format when stable
        }
    }

    result
}
