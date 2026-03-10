use anyhow::Result;
use std::path::PathBuf;

/// Lockfile location for daemon discovery
pub fn lockfile_path() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("caboose");
    data_dir.join("daemon.lock")
}

/// Check if a daemon is already running
pub fn is_daemon_running() -> bool {
    let path = lockfile_path();
    if !path.exists() {
        return false;
    }
    // Read PID from lockfile and check if process exists
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            // Format: "PID:PORT"
            if let Some(pid_str) = contents.split(':').next() {
                if let Ok(_pid) = pid_str.parse::<u32>() {
                    // TODO: check if PID is alive (platform-specific)
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
}

/// Write the daemon lockfile
pub fn write_lockfile(port: u16) -> Result<()> {
    let path = lockfile_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pid = std::process::id();
    std::fs::write(&path, format!("{pid}:{port}"))?;
    Ok(())
}

/// Remove the daemon lockfile
pub fn remove_lockfile() -> Result<()> {
    let path = lockfile_path();
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockfile_path_is_in_data_dir() {
        let path = lockfile_path();
        assert!(path.to_string_lossy().contains("caboose"));
        assert!(path.file_name().unwrap() == "daemon.lock");
    }
}
