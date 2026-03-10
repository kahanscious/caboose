use std::path::PathBuf;

/// Items discoverable from a Codex installation
#[derive(Debug, Clone, Default)]
pub struct CodexConfig {
    pub config_path: Option<PathBuf>,
    pub instructions: Option<String>,
}

/// Scan Codex config directories
pub fn scan_codex(config_dirs: &[PathBuf]) -> CodexConfig {
    let mut result = CodexConfig::default();

    for dir in config_dirs {
        let config_file = dir.join("config.json");
        if config_file.exists() {
            result.config_path = Some(config_file.clone());
            // TODO: parse Codex config format when stable
        }

        let instructions_file = dir.join("instructions.md");
        if instructions_file.exists() {
            result.instructions = std::fs::read_to_string(&instructions_file).ok();
        }
    }

    result
}
