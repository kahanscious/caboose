//! Git snapshot — save/restore points before tool execution.

use anyhow::Result;
use std::process::Command;

/// Create a git stash snapshot before a potentially destructive operation.
#[allow(dead_code)]
pub fn create_snapshot(message: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["stash", "push", "-m", message, "--include-untracked"])
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Restore the most recent snapshot.
#[allow(dead_code)]
pub fn restore_snapshot() -> Result<String> {
    let output = Command::new("git").args(["stash", "pop"]).output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
