use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
pub fn slug(task: &str) -> String {
    let s: String = task
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive dashes
    let mut result = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_dash {
                result.push(c);
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    let result = result.trim_matches('-').to_string();
    // Truncate, then trim again in case truncation lands on a dash
    let truncated: String = result.chars().take(40).collect();
    truncated.trim_matches('-').to_string()
}

#[allow(dead_code)]
pub fn unique_slug(task: &str, existing: &[String]) -> String {
    let base = slug(task);
    if !existing.contains(&base) {
        return base;
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("{base}-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[allow(dead_code)]
pub fn branch_name(slug: &str) -> String {
    format!("agent/{slug}")
}

#[allow(dead_code)]
pub fn worktree_path(slug: &str) -> PathBuf {
    PathBuf::from(format!(".worktrees/agent-{slug}"))
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error(".worktrees/ is not in .gitignore — add it before running /execute")]
    NotIgnored,
    #[error("git command failed: {0}")]
    GitFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Verify `.worktrees/` is listed in `.gitignore`.
#[allow(dead_code)]
pub fn check_worktrees_ignored() -> Result<(), WorktreeError> {
    let status = Command::new("git")
        .args(["check-ignore", "-q", ".worktrees"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(WorktreeError::NotIgnored)
    }
}

/// Create a worktree at `path` with new branch `branch` from current HEAD.
#[allow(dead_code)]
pub fn create_worktree(path: &Path, branch: &str) -> Result<(), WorktreeError> {
    let out = Command::new("git")
        .args(["worktree", "add", &path.to_string_lossy(), "-b", branch])
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&out.stderr).into(),
        ))
    }
}

/// Merge `branch` into the current branch with `--no-ff`.
/// On conflict, aborts the merge and returns Err.
#[allow(dead_code)]
pub fn merge_branch(branch: &str) -> Result<(), WorktreeError> {
    let out = Command::new("git")
        .args(["merge", "--no-ff", branch])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let _ = Command::new("git").args(["merge", "--abort"]).status();
    Err(WorktreeError::GitFailed(
        String::from_utf8_lossy(&out.stderr).into(),
    ))
}

/// Remove a worktree and (best-effort) delete its branch.
#[allow(dead_code)]
pub fn remove_worktree(path: &Path, branch: &str) -> Result<(), WorktreeError> {
    let out = Command::new("git")
        .args(["worktree", "remove", &path.to_string_lossy()])
        .output()?;
    if !out.status.success() {
        return Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&out.stderr).into(),
        ));
    }
    let _ = Command::new("git").args(["branch", "-d", branch]).status();
    Ok(())
}

/// Get the name of the currently checked-out branch.
#[allow(dead_code)]
pub fn current_branch() -> Result<String, WorktreeError> {
    let out = Command::new("git")
        .args(["branch", "--show-current"])
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&out.stderr).into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_spaces_become_dashes() {
        assert_eq!(slug("auth refactor"), "auth-refactor");
    }

    #[test]
    fn slug_collapses_consecutive_dashes() {
        assert_eq!(slug("hello -- world"), "hello-world");
    }

    #[test]
    fn slug_trims_trailing_dash() {
        assert_eq!(slug("task (done)"), "task-done");
    }

    #[test]
    fn slug_truncates_at_40_chars() {
        let long = "this is a very long task name that exceeds forty chars easily";
        let s = slug(long);
        assert!(s.len() <= 40);
        assert!(
            !s.ends_with('-'),
            "slug must not end in a dash after truncation"
        );
    }

    #[test]
    fn slug_already_clean() {
        assert_eq!(slug("update-readme"), "update-readme");
    }

    #[test]
    fn unique_slug_no_conflict() {
        assert_eq!(unique_slug("auth refactor", &[]), "auth-refactor");
    }

    #[test]
    fn unique_slug_appends_counter() {
        let existing = vec!["auth-refactor".to_string()];
        assert_eq!(unique_slug("auth refactor", &existing), "auth-refactor-2");
    }

    #[test]
    fn unique_slug_skips_used_counters() {
        let existing = vec!["auth-refactor".to_string(), "auth-refactor-2".to_string()];
        assert_eq!(unique_slug("auth refactor", &existing), "auth-refactor-3");
    }

    #[test]
    fn branch_name_prefixes_agent() {
        assert_eq!(branch_name("auth-refactor"), "agent/auth-refactor");
    }

    #[test]
    fn worktree_path_format() {
        assert_eq!(
            worktree_path("auth-refactor"),
            PathBuf::from(".worktrees/agent-auth-refactor")
        );
    }
}
