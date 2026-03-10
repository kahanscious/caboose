use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum ScmProvider {
    GitHub,
    GitLab,
    Unknown,
}

/// Detect SCM provider from git remotes in the given directory
pub fn detect_provider(cwd: &Path) -> ScmProvider {
    let output = Command::new("git")
        .args(["remote", "-v"])
        .current_dir(cwd)
        .output();

    match output {
        Ok(out) => {
            let remotes = String::from_utf8_lossy(&out.stdout);
            if remotes.contains("github.com") {
                ScmProvider::GitHub
            } else if remotes.contains("gitlab.com") || remotes.contains("gitlab") {
                ScmProvider::GitLab
            } else {
                ScmProvider::Unknown
            }
        }
        Err(_) => ScmProvider::Unknown,
    }
}

/// Check if the `gh` CLI is installed
pub fn has_gh_cli() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if the `glab` CLI is installed
pub fn has_glab_cli() -> bool {
    Command::new("glab")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_detect_provider_in_current_repo() {
        let cwd = env::current_dir().unwrap();
        let provider = detect_provider(&cwd);
        assert_eq!(provider, ScmProvider::GitHub);
    }

    #[test]
    fn test_detect_provider_nonexistent_dir() {
        let provider = detect_provider(Path::new("/nonexistent/path"));
        assert_eq!(provider, ScmProvider::Unknown);
    }
}
