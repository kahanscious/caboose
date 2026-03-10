//! Auto-detect project languages from config files.

use std::collections::HashMap;
use std::path::Path;

use crate::config::schema::LspServerConfig;

struct LangRule {
    language: &'static str,
    config_files: &'static [&'static str],
    command: &'static str,
    args: &'static [&'static str],
}

const RULES: &[LangRule] = &[
    LangRule {
        language: "typescript",
        config_files: &["tsconfig.json", "package.json"],
        command: "typescript-language-server",
        args: &["--stdio"],
    },
    LangRule {
        language: "rust",
        config_files: &["Cargo.toml"],
        command: "rust-analyzer",
        args: &[],
    },
    LangRule {
        language: "go",
        config_files: &["go.mod"],
        command: "gopls",
        args: &["serve"],
    },
    LangRule {
        language: "python",
        config_files: &["pyproject.toml", "setup.py", "requirements.txt"],
        command: "pylsp",
        args: &[],
    },
];

/// Scan `workspace_root` for known config files and return default server
/// configs for each detected language.
pub fn detect_languages(workspace_root: &Path) -> HashMap<String, LspServerConfig> {
    let mut result = HashMap::new();
    for rule in RULES {
        let found = rule
            .config_files
            .iter()
            .any(|f| workspace_root.join(f).exists());
        if found {
            result.insert(
                rule.language.to_string(),
                LspServerConfig {
                    command: rule.command.to_string(),
                    args: rule.args.iter().map(|s| s.to_string()).collect(),
                    env: HashMap::new(),
                    disabled: false,
                },
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_rust_from_cargo_toml() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        let detected = detect_languages(tmp.path());
        assert_eq!(detected.len(), 1);
        assert_eq!(detected["rust"].command, "rust-analyzer");
    }

    #[test]
    fn detects_typescript_from_tsconfig() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("tsconfig.json"), "{}").unwrap();
        let detected = detect_languages(tmp.path());
        assert!(detected.contains_key("typescript"));
        assert_eq!(detected["typescript"].args, vec!["--stdio"]);
    }

    #[test]
    fn detects_multiple_languages() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        std::fs::write(tmp.path().join("go.mod"), "").unwrap();
        let detected = detect_languages(tmp.path());
        assert_eq!(detected.len(), 2);
        assert!(detected.contains_key("rust"));
        assert!(detected.contains_key("go"));
    }

    #[test]
    fn empty_dir_detects_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let detected = detect_languages(tmp.path());
        assert!(detected.is_empty());
    }
}
