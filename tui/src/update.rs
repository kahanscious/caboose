use anyhow::Result;
use sha2::{Digest, Sha256};

const DOWNLOADS_BASE_URL: &str = "https://downloads.trycaboose.dev";

fn verify_sha256(data: &[u8], expected_hex: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let hex = format!("{:x}", result);
    hex == expected_hex
}

fn find_checksum_for_artifact(checksums_content: &str, artifact_name: &str) -> Option<String> {
    for line in checksums_content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2 && parts[1] == artifact_name {
            return Some(parts[0].to_string());
        }
    }
    None
}

fn artifact_name() -> &'static str {
    if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "caboose-aarch64-apple-darwin.tar.gz"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "caboose-x86_64-apple-darwin.tar.gz"
    } else if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "caboose-x86_64-pc-windows-msvc.zip"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "caboose-x86_64-unknown-linux-musl.tar.gz"
    } else {
        "caboose-unknown-target"
    }
}

fn replace_binary(archive_bytes: &[u8], exe_path: &std::path::Path) -> Result<()> {
    use std::io::Read as _;

    let binary_bytes = if cfg!(windows) {
        // .zip archive — extract caboose.exe
        let cursor = std::io::Cursor::new(archive_bytes);
        let mut archive = zip::ZipArchive::new(cursor)?;
        let mut found = None;
        for i in 0..archive.len() {
            let name = archive.by_index(i)?.name().to_string();
            if name.ends_with("caboose.exe") || name == "caboose.exe" {
                found = Some(name);
                break;
            }
        }
        let name = found.ok_or_else(|| anyhow::anyhow!("Could not find caboose.exe in archive"))?;
        let mut file = archive.by_name(&name)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        buf
    } else {
        // .tar.gz archive — extract the caboose binary
        let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(archive_bytes));
        let mut archive = tar::Archive::new(decoder);
        let mut binary = Vec::new();
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;
            if path.file_name().is_some_and(|n| n == "caboose") {
                entry.read_to_end(&mut binary)?;
                break;
            }
        }
        if binary.is_empty() {
            anyhow::bail!("Could not find caboose binary in archive");
        }
        binary
    };

    // Write to a temp file next to the current exe, then rename
    let temp_path = exe_path.with_extension("new");
    std::fs::write(&temp_path, &binary_bytes)?;

    // On Unix, set executable permission
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Replace: on Windows, rename current exe first (can't overwrite running exe)
    if cfg!(windows) {
        let backup_path = exe_path.with_extension("old");
        let _ = std::fs::remove_file(&backup_path); // clean up previous backup
        std::fs::rename(exe_path, &backup_path)?;
        std::fs::rename(&temp_path, exe_path)?;
    } else {
        std::fs::rename(&temp_path, exe_path)?;
    }

    Ok(())
}

pub async fn run(check_only: bool) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    let method = detect_install_method();

    // For package manager installs, just tell the user what to do
    match method {
        InstallMethod::Homebrew => {
            println!("caboose is managed by Homebrew. Run:\n  brew upgrade caboose");
            return Ok(());
        }
        InstallMethod::Chocolatey => {
            println!("caboose is managed by Chocolatey. Run:\n  choco upgrade caboose");
            return Ok(());
        }
        InstallMethod::Winget => {
            println!("caboose is managed by Winget. Run:\n  winget upgrade TryCaboose.Caboose");
            return Ok(());
        }
        InstallMethod::Direct => {}
    }

    // Direct install — check for updates
    print!("Checking for updates... ");
    let latest = fetch_latest_version().await?;
    let latest_bare = latest.strip_prefix('v').unwrap_or(&latest);

    if !is_newer(latest_bare, current_version) {
        println!("caboose v{current_version} is already up to date.");
        return Ok(());
    }

    println!("v{current_version} → {latest} available.");

    if check_only {
        println!("\nRun `caboose update` to install.");
        return Ok(());
    }

    // Download the binary
    let artifact = artifact_name();
    let artifact_url = format!("{DOWNLOADS_BASE_URL}/{latest}/{artifact}");
    let checksums_url = format!("{DOWNLOADS_BASE_URL}/{latest}/checksums.txt");

    println!("Downloading {artifact}...");
    let artifact_bytes = reqwest::get(&artifact_url)
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // Verify checksum
    let checksums_text = reqwest::get(&checksums_url)
        .await?
        .error_for_status()?
        .text()
        .await?;

    let expected_checksum = find_checksum_for_artifact(&checksums_text, artifact)
        .ok_or_else(|| anyhow::anyhow!("Checksum not found for {artifact}"))?;

    if !verify_sha256(&artifact_bytes, &expected_checksum) {
        anyhow::bail!("Checksum verification failed for {artifact}");
    }
    println!("Checksum verified.");

    // Extract and replace binary
    let exe_path = std::env::current_exe()?;
    replace_binary(&artifact_bytes, &exe_path)?;

    // macOS: strip quarantine attribute
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&exe_path)
            .output();
    }

    println!("Updated to caboose {latest}.");
    Ok(())
}

#[derive(Debug, PartialEq)]
enum InstallMethod {
    Homebrew,
    Chocolatey,
    Winget,
    Direct,
}

fn detect_install_method_from_path(path: &str) -> InstallMethod {
    let path_lower = path.to_lowercase();

    // Homebrew: /opt/homebrew/Cellar/..., /usr/local/Cellar/..., .linuxbrew/Cellar/...
    if path_lower.contains("/cellar/") || path_lower.contains("/homebrew/") {
        return InstallMethod::Homebrew;
    }

    // Chocolatey: C:\ProgramData\chocolatey\...
    if path_lower.contains("chocolatey") {
        return InstallMethod::Chocolatey;
    }

    // Winget: ...\WinGet\Packages\...
    if path_lower.contains("winget") {
        return InstallMethod::Winget;
    }

    InstallMethod::Direct
}

fn detect_install_method() -> InstallMethod {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_default();
    detect_install_method_from_path(&exe)
}

/// Compare semver strings. Returns true if `remote` is newer than `local`.
pub fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |v: &str| -> (u32, u32, u32) {
        let v = v.strip_prefix('v').unwrap_or(v);
        let parts: Vec<&str> = v.split('.').collect();
        let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(remote) > parse(local)
}

pub async fn fetch_latest_version() -> Result<String> {
    let url = format!("{DOWNLOADS_BASE_URL}/latest.txt");
    let body = reqwest::get(&url).await?.text().await?;
    Ok(body.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_homebrew_cellar() {
        let method =
            detect_install_method_from_path("/opt/homebrew/Cellar/caboose/0.1.0/bin/caboose");
        assert_eq!(method, InstallMethod::Homebrew);
    }

    #[test]
    fn detect_homebrew_linuxbrew() {
        let method = detect_install_method_from_path(
            "/home/user/.linuxbrew/Cellar/caboose/0.1.0/bin/caboose",
        );
        assert_eq!(method, InstallMethod::Homebrew);
    }

    #[test]
    fn detect_homebrew_usr_local() {
        let method = detect_install_method_from_path("/usr/local/Cellar/caboose/0.1.0/bin/caboose");
        assert_eq!(method, InstallMethod::Homebrew);
    }

    #[test]
    fn detect_chocolatey() {
        let method = detect_install_method_from_path(r"C:\ProgramData\chocolatey\bin\caboose.exe");
        assert_eq!(method, InstallMethod::Chocolatey);
    }

    #[test]
    fn detect_winget() {
        let method = detect_install_method_from_path(
            r"C:\Users\Alex\AppData\Local\Microsoft\WinGet\Packages\TryCaboose.Caboose_Microsoft.Winget.Source_8wekyb3d8bbwe\caboose.exe",
        );
        assert_eq!(method, InstallMethod::Winget);
    }

    #[test]
    fn detect_direct_install_unix() {
        let method = detect_install_method_from_path("/usr/local/bin/caboose");
        assert_eq!(method, InstallMethod::Direct);
    }

    #[test]
    fn detect_direct_install_windows() {
        let method = detect_install_method_from_path(r"C:\Users\Alex\.caboose\bin\caboose.exe");
        assert_eq!(method, InstallMethod::Direct);
    }

    #[test]
    fn version_newer_detected() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.1", "0.1.0"));
    }

    #[test]
    fn version_same_or_older_not_newer() {
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn version_strips_v_prefix() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.2.0", "v0.1.0"));
        assert!(is_newer("v0.2.0", "v0.1.0"));
    }

    #[test]
    fn verify_checksum_valid() {
        let data = b"hello world";
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_sha256(data, expected));
    }

    #[test]
    fn verify_checksum_invalid() {
        let data = b"hello world";
        assert!(!verify_sha256(
            data,
            "0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }

    #[test]
    fn parse_checksums_file() {
        let content = "abc123  caboose-aarch64-apple-darwin.tar.gz\ndef456  caboose-x86_64-pc-windows-msvc.zip\n";
        let checksum = find_checksum_for_artifact(content, "caboose-aarch64-apple-darwin.tar.gz");
        assert_eq!(checksum, Some("abc123".to_string()));
    }

    #[test]
    fn parse_checksums_missing_artifact() {
        let content = "abc123  caboose-aarch64-apple-darwin.tar.gz\n";
        let checksum =
            find_checksum_for_artifact(content, "caboose-x86_64-unknown-linux-musl.tar.gz");
        assert_eq!(checksum, None);
    }

    #[test]
    fn artifact_name_is_valid() {
        let name = artifact_name();
        assert!(name.starts_with("caboose-"));
        assert!(name.ends_with(".tar.gz") || name.ends_with(".zip"));
    }
}
