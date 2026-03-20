//! Parallel scan execution with timeout and output capture.

use crate::suggest::config::DEFAULT_TIMEOUT_SECS;
use crate::suggest::parsers;
use crate::suggest::priority::Finding;
use caboose_core::config::schema::ScanCommandConfig;

/// Result of a single scan command.
pub struct ScanResult {
    #[allow(dead_code)]
    pub name: String,
    pub findings: Vec<Finding>,
    pub error: Option<String>,
}

/// Max output lines to capture per scan command.
const MAX_OUTPUT_LINES: usize = 2000;
/// Max output bytes to capture per scan command.
const MAX_OUTPUT_BYTES: usize = 50_000;

/// Run all scan commands in parallel + built-in scans. Returns all results.
pub async fn run_scans(scans: &[ScanCommandConfig]) -> Vec<ScanResult> {
    let mut handles = Vec::new();

    for scan in scans {
        let name = scan.name.clone();
        let command = scan.command.clone();
        let category = scan.category.clone();
        let timeout = scan.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);

        handles.push(tokio::spawn(async move {
            run_single_scan(&name, &command, &category, timeout).await
        }));
    }

    // Built-in: TODO grep
    handles.push(tokio::spawn(run_todo_scan()));

    // Built-in: git churn
    handles.push(tokio::spawn(run_git_churn_scan()));

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(ScanResult {
                name: "unknown".to_string(),
                findings: vec![],
                error: Some(format!("scan task panicked: {e}")),
            }),
        }
    }
    results
}

async fn run_single_scan(
    name: &str,
    command: &str,
    category: &str,
    timeout_secs: u64,
) -> ScanResult {
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let result = tokio::time::timeout(timeout, async {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            let was_large =
                combined.len() > MAX_OUTPUT_BYTES || combined.lines().count() > MAX_OUTPUT_LINES;
            let truncated = truncate_output(&combined);
            let mut findings = parsers::parse_scan_output(category, &truncated);

            if was_large {
                findings.push(Finding {
                    category: crate::suggest::priority::Category::Custom("truncated".to_string()),
                    severity: crate::suggest::priority::Severity::Info,
                    summary: format!("{name}: output truncated — results may be incomplete"),
                    location: None,
                    count: 1,
                });
            }

            ScanResult {
                name: name.to_string(),
                findings,
                error: None,
            }
        }
        Ok(Err(e)) => ScanResult {
            name: name.to_string(),
            findings: vec![],
            error: Some(format!("{name} scan failed: {e}")),
        },
        Err(_) => ScanResult {
            name: name.to_string(),
            findings: vec![],
            error: Some(format!("{name} scan timed out after {timeout_secs}s")),
        },
    }
}

async fn run_todo_scan() -> ScanResult {
    let result = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(concat!(
            "grep -rn ",
            "--include='*.rs' --include='*.ts' --include='*.py' ",
            "--include='*.go' --include='*.js' --include='*.jsx' --include='*.tsx' ",
            "-E '(TODO|FIXME|HACK):?' . 2>/dev/null | head -100"
        ))
        .output()
        .await;

    match result {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            ScanResult {
                name: "todos".to_string(),
                findings: parsers::parse_todo_grep(&text),
                error: None,
            }
        }
        Err(e) => ScanResult {
            name: "todos".to_string(),
            findings: vec![],
            error: Some(format!("TODO scan failed: {e}")),
        },
    }
}

async fn run_git_churn_scan() -> ScanResult {
    let result = tokio::process::Command::new("git")
        .args(["log", "--format=", "--name-only", "-20"])
        .output()
        .await;

    match result {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            ScanResult {
                name: "churn".to_string(),
                findings: parsers::parse_git_churn(&text),
                error: None,
            }
        }
        Err(e) => ScanResult {
            name: "churn".to_string(),
            findings: vec![],
            error: Some(format!("git churn scan failed: {e}")),
        },
    }
}

fn truncate_output(output: &str) -> String {
    let mut result = String::new();
    for (i, line) in output.lines().enumerate() {
        if i >= MAX_OUTPUT_LINES || result.len() + line.len() + 1 > MAX_OUTPUT_BYTES {
            break;
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_output_caps_lines() {
        let long = (0..3000)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let truncated = truncate_output(&long);
        assert!(truncated.lines().count() <= MAX_OUTPUT_LINES);
    }

    #[test]
    fn truncate_output_caps_bytes() {
        let long = "x".repeat(100_000);
        let truncated = truncate_output(&long);
        assert!(truncated.len() <= MAX_OUTPUT_BYTES + 1);
    }
}
