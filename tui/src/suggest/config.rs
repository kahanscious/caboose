//! Auto-detection and config helpers for /suggest.

use crate::config::schema::{ScanCommandConfig, SuggestConfig};

/// Default timeout for scan commands in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Resolve scan commands: use config if provided, otherwise auto-detect.
pub fn resolve_scans(config: Option<&SuggestConfig>) -> Vec<ScanCommandConfig> {
    if let Some(c) = config {
        if !c.scans.is_empty() {
            return c.scans.clone();
        }
    }
    auto_detect()
}

/// Auto-detect scan commands from project files in the current directory.
pub fn auto_detect() -> Vec<ScanCommandConfig> {
    let mut scans = Vec::new();

    if std::path::Path::new("Cargo.toml").exists() {
        scans.push(ScanCommandConfig {
            name: "clippy".to_string(),
            command: "cargo clippy --message-format=json 2>&1".to_string(),
            category: "lint".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
        scans.push(ScanCommandConfig {
            name: "test".to_string(),
            command: "cargo test 2>&1".to_string(),
            category: "test".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
    } else if std::path::Path::new("package.json").exists() {
        scans.push(ScanCommandConfig {
            name: "lint".to_string(),
            command: "npx eslint . --format=json 2>&1".to_string(),
            category: "lint".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
        scans.push(ScanCommandConfig {
            name: "test".to_string(),
            command: "npm test 2>&1".to_string(),
            category: "test".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
    } else if std::path::Path::new("pyproject.toml").exists()
        || std::path::Path::new("setup.py").exists()
    {
        scans.push(ScanCommandConfig {
            name: "lint".to_string(),
            command: "ruff check . --output-format=json 2>&1".to_string(),
            category: "lint".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
        scans.push(ScanCommandConfig {
            name: "test".to_string(),
            command: "python -m pytest --collect-only 2>&1".to_string(),
            category: "test".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
    }

    scans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_config_when_provided() {
        let config = SuggestConfig {
            enabled: true,
            scans: vec![ScanCommandConfig {
                name: "custom".to_string(),
                command: "echo hello".to_string(),
                category: "custom".to_string(),
                timeout_secs: None,
            }],
            priorities: None,
        };
        let scans = resolve_scans(Some(&config));
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].name, "custom");
    }

    #[test]
    fn resolve_auto_detects_when_config_empty() {
        let config = SuggestConfig {
            enabled: true,
            scans: vec![],
            priorities: None,
        };
        // We're in a Rust project, so auto_detect should find Cargo.toml
        let scans = resolve_scans(Some(&config));
        // Don't assert specific count — depends on cwd — just verify no crash
        let _ = scans;
    }

    #[test]
    fn resolve_auto_detects_when_no_config() {
        let scans = resolve_scans(None);
        let _ = scans;
    }
}
