use std::path::{Path, PathBuf};
use std::process::Command;

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

pub fn branch_name(slug: &str) -> String {
    format!("agent/{slug}")
}

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

/// Run `git diff --unified=0` between a base SHA and a branch.
/// Must be called via spawn_blocking.
pub fn run_diff(base_sha: &str, branch: &str) -> Result<String, WorktreeError> {
    let out = Command::new("git")
        .args(["diff", "--unified=0", &format!("{base_sha}...{branch}")])
        .output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        // diff can return non-zero for binary files etc., but stderr tells us
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.is_empty() {
            // Non-zero exit but no error — treat output as valid
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(WorktreeError::GitFailed(stderr.into()))
        }
    }
}

/// Create a worktree at `path` with new branch `branch` from current HEAD.
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
pub fn merge_branch(branch: &str) -> Result<(), WorktreeError> {
    let out = Command::new("git")
        .args(["merge", "--no-ff", branch])
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    let _ = Command::new("git").args(["merge", "--abort"]).output();
    Err(WorktreeError::GitFailed(
        String::from_utf8_lossy(&out.stderr).into(),
    ))
}

/// Remove a worktree and (best-effort) delete its branch.
pub fn remove_worktree(path: &Path, branch: &str) -> Result<(), WorktreeError> {
    let out = Command::new("git")
        .args(["worktree", "remove", &path.to_string_lossy()])
        .output()?;
    if !out.status.success() {
        return Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&out.stderr).into(),
        ));
    }
    let _ = Command::new("git").args(["branch", "-d", branch]).output();
    Ok(())
}

/// Commit all staged and unstaged changes in a worktree branch.
/// Returns Ok(true) when a commit was created, Ok(false) when there was nothing to commit.
pub fn commit_worktree(path: &Path, message: &str) -> Result<bool, WorktreeError> {
    let status = Command::new("git")
        .args(["-C", &path.to_string_lossy(), "status", "--porcelain"])
        .output()?;
    if !status.status.success() {
        return Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&status.stderr).into(),
        ));
    }
    if String::from_utf8_lossy(&status.stdout).trim().is_empty() {
        return Ok(false);
    }

    let add = Command::new("git")
        .args(["-C", &path.to_string_lossy(), "add", "-A"])
        .output()?;
    if !add.status.success() {
        return Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&add.stderr).into(),
        ));
    }

    let commit = Command::new("git")
        .args([
            "-C",
            &path.to_string_lossy(),
            "-c",
            "user.name=Caboose",
            "-c",
            "user.email=caboose@local",
            "commit",
            "-m",
            message,
            "--no-gpg-sign",
        ])
        .output()?;
    if commit.status.success() {
        Ok(true)
    } else {
        Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&commit.stderr).into(),
        ))
    }
}

/// Get the current HEAD commit SHA.
pub fn current_head_sha() -> Result<String, WorktreeError> {
    let out = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(WorktreeError::GitFailed(
            String::from_utf8_lossy(&out.stderr).into(),
        ))
    }
}

/// Read a file from a specific commit. Returns Ok(None) if the file does not exist there.
pub fn read_file_at_commit(commit: &str, path: &str) -> Result<Option<String>, WorktreeError> {
    let spec = format!("{commit}:{path}");
    let out = Command::new("git").args(["show", &spec]).output()?;
    if out.status.success() {
        return Ok(Some(String::from_utf8_lossy(&out.stdout).to_string()));
    }

    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("exists on disk, but not in") || stderr.contains("does not exist in") {
        Ok(None)
    } else {
        Err(WorktreeError::GitFailed(stderr.into()))
    }
}

/// Read a file from a subagent worktree. Returns Ok(None) when the file is absent.
pub fn read_worktree_file(
    worktree_root: &Path,
    relative_path: &str,
) -> Result<Option<String>, WorktreeError> {
    let path = worktree_root.join(relative_path);
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(WorktreeError::Io(err)),
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
