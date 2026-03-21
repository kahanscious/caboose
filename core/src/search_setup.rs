//! Embedded SearXNG setup — Docker Compose generation, lifecycle, and health checks.

use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Directory where search infrastructure files are stored.
pub fn search_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("caboose")
        .join("search")
}

/// Generate the docker-compose.yml content.
pub fn compose_yml() -> &'static str {
    r#"services:
  redis:
    image: redis:7-alpine
    container_name: caboose-redis
    restart: unless-stopped

  searxng:
    image: searxng/searxng:2026.3.10-8b95b2058
    container_name: caboose-searxng
    restart: unless-stopped
    ports:
      - "127.0.0.1:8080:8080"
    volumes:
      - ./searxng:/etc/searxng
    depends_on:
      - redis
    healthcheck:
      test: ["CMD", "wget", "-q", "--spider", "http://localhost:8080/search?q=test&format=json"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s
"#
}

/// Generate settings.yml content with a random secret key.
pub fn settings_yml() -> String {
    let secret_key = generate_secret_key();
    format!(
        r#"use_default_settings:
  engines:
    keep_only:
      - google
      - bing

server:
  secret_key: "{secret_key}"
  base_url: "http://127.0.0.1:8080"
  bind_address: "0.0.0.0:8080"
  public_instance: false

search:
  safe_search: 0
  default_lang: "en"
  formats:
    - html
    - json

outgoing:
  request_timeout: 3.0
  max_request_timeout: 5.0

redis:
  url: redis://redis:6379/0

engines:
  - name: google
    engine: google
    disabled: false
    shortcut: g
  - name: bing
    engine: bing
    disabled: false
    shortcut: b
"#
    )
}

fn generate_secret_key() -> String {
    use std::fmt::Write;
    let mut key = String::with_capacity(64);
    for _ in 0..32 {
        let byte: u8 = rand::random();
        write!(key, "{byte:02x}").unwrap();
    }
    key
}

/// Write compose and settings files to the search directory.
/// Returns the search directory path. Skips files that already exist.
pub fn write_files() -> Result<PathBuf> {
    let dir = search_dir();
    let searxng_dir = dir.join("searxng");
    std::fs::create_dir_all(&searxng_dir)?;

    let compose_path = dir.join("docker-compose.yml");
    if !compose_path.exists() {
        std::fs::write(&compose_path, compose_yml())?;
    }

    let settings_path = searxng_dir.join("settings.yml");
    if !settings_path.exists() {
        std::fs::write(&settings_path, settings_yml())?;
    }

    Ok(dir)
}

/// Check if `docker compose` is available.
pub fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["compose", "version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Start the SearXNG containers.
pub fn compose_up(dir: &Path) -> Result<String> {
    let output = std::process::Command::new("docker")
        .args(["compose", "up", "-d"])
        .current_dir(dir)
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(anyhow!(
            "docker compose up failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Stop the SearXNG containers.
pub fn compose_down(dir: &Path) -> Result<String> {
    let output = std::process::Command::new("docker")
        .args(["compose", "down"])
        .current_dir(dir)
        .output()?;

    if output.status.success() {
        Ok("Containers stopped.".to_string())
    } else {
        Err(anyhow!(
            "docker compose down failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Check if SearXNG containers are running.
pub fn is_running() -> bool {
    std::process::Command::new("docker")
        .args(["inspect", "--format", "{{.State.Running}}", "caboose-searxng"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Poll health endpoint until ready (max 30 seconds).
pub async fn wait_healthy() -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    for _ in 0..30 {
        if client
            .get("http://127.0.0.1:8080/search?q=test&format=json")
            .send()
            .await
            .is_ok_and(|resp| resp.status().is_success())
        {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    false
}

/// Append web search config to global config if not already present.
pub fn auto_configure() -> Result<bool> {
    let config_path = dirs::config_dir()
        .ok_or_else(|| anyhow!("no config directory"))?
        .join("caboose")
        .join("config.toml");

    let existing = if config_path.exists() {
        std::fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    if existing.contains("[services.web_search]") {
        return Ok(false); // Already configured
    }

    let block = "\n[services.web_search]\nprovider = \"searxng\"\nbase_url = \"http://127.0.0.1:8080\"\n";

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config_path)?;
    use std::io::Write;
    file.write_all(block.as_bytes())?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_yml_is_valid_and_local_only() {
        let content = compose_yml();
        assert!(content.contains("caboose-searxng"));
        assert!(content.contains("127.0.0.1:8080:8080"));
        assert!(!content.contains("cloudflared"));
        assert!(content.contains("caboose-redis"));
    }

    #[test]
    fn settings_yml_has_required_fields() {
        let content = settings_yml();
        assert!(content.contains("secret_key:"));
        assert!(content.contains("formats:"));
        assert!(content.contains("json"));
        assert!(content.contains("google"));
        assert!(content.contains("bing"));
        assert!(!content.contains("duckduckgo"));
    }

    #[test]
    fn settings_yml_generates_different_keys() {
        let a = settings_yml();
        let b = settings_yml();
        let key_a = a.lines().find(|l| l.contains("secret_key")).unwrap();
        let key_b = b.lines().find(|l| l.contains("secret_key")).unwrap();
        assert_ne!(key_a, key_b);
    }
}
