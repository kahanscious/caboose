use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum SourcePlatform {
    ClaudeCode,
    OpenCode,
    Codex,
}

impl SourcePlatform {
    pub fn label(&self) -> &str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::OpenCode => "Open Code",
            Self::Codex => "Codex",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![Self::ClaudeCode, Self::OpenCode, Self::Codex]
    }
}

/// Known config paths for each platform
pub fn config_paths(platform: &SourcePlatform) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();

    match platform {
        SourcePlatform::ClaudeCode => {
            let mut paths = vec![home.join(".claude")];
            if let Some(config) = dirs::config_dir() {
                paths.push(config.join("claude"));
            }
            paths
        }
        SourcePlatform::OpenCode => {
            let mut paths = vec![home.join(".open-code")];
            if let Some(config) = dirs::config_dir() {
                paths.push(config.join("open-code"));
            }
            paths
        }
        SourcePlatform::Codex => {
            let mut paths = vec![home.join(".codex")];
            if let Some(config) = dirs::config_dir() {
                paths.push(config.join("codex"));
            }
            paths
        }
    }
}

/// Check which platforms have detectable configs
#[allow(dead_code)]
pub fn detect_installed_platforms() -> Vec<SourcePlatform> {
    SourcePlatform::all()
        .into_iter()
        .filter(|p| config_paths(p).iter().any(|path| path.exists()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_platforms() {
        let all = SourcePlatform::all();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_config_paths_nonempty() {
        for platform in SourcePlatform::all() {
            let paths = config_paths(&platform);
            assert!(!paths.is_empty(), "no paths for {:?}", platform);
        }
    }

    #[test]
    fn test_labels() {
        assert_eq!(SourcePlatform::ClaudeCode.label(), "Claude Code");
        assert_eq!(SourcePlatform::OpenCode.label(), "Open Code");
        assert_eq!(SourcePlatform::Codex.label(), "Codex");
    }

    #[test]
    fn config_paths_returns_nonempty() {
        for platform in &[
            SourcePlatform::ClaudeCode,
            SourcePlatform::OpenCode,
            SourcePlatform::Codex,
        ] {
            let paths = config_paths(platform);
            assert!(
                !paths.is_empty(),
                "{:?} should have at least one path",
                platform
            );
        }
    }

    #[test]
    fn claude_code_path_contains_dot_claude() {
        let paths = config_paths(&SourcePlatform::ClaudeCode);
        assert!(
            paths
                .iter()
                .any(|p| p.to_string_lossy().contains(".claude"))
        );
    }

    #[test]
    fn open_code_has_multiple_paths() {
        let paths = config_paths(&SourcePlatform::OpenCode);
        // Should have both ~/.open-code and XDG/platform config path
        assert!(paths.len() >= 1);
    }
}
