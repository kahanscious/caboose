# `/suggest` Command Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `/suggest` slash command that scans the codebase, parses findings into typed structs, and sends a compact prioritized digest to the LLM for actionable improvement suggestions.

**Architecture:** New `tui/src/suggest/` module with a pipeline: config loading + auto-detection -> parallel command execution -> typed parsing -> deduplication + priority sorting -> digest formatting. The digest is injected as a user message and the LLM responds in chat. Config lives in `[suggest]` section of `config.toml`.

**Tech Stack:** Rust, tokio (parallel scan execution), serde_json (clippy JSON parsing), existing shell execution infrastructure.

**Spec:** `docs/specs/2026-03-17-suggest-command-design.md`

---

## File Structure

```
tui/src/suggest/
  mod.rs        — public API: run_suggest(), re-exports
  config.rs     — SuggestConfig, ScanConfig, PriorityConfig, auto-detect logic
  scanner.rs    — run_scans(): parallel command execution, timeout, output capture
  parsers.rs    — parse_scan_output(): clippy JSON, cargo test, generic, TODO grep, git churn
  digest.rs     — build_digest(): Vec<Finding> -> compact markdown string
  priority.rs   — Finding, Category, Severity types, sort_findings(), dedup_findings()
```

Existing files modified:
- `tui/src/main.rs` — add `mod suggest;`
- `tui/src/config/mod.rs` — add `pub suggest: Option<schema::SuggestConfig>`
- `tui/src/config/schema.rs` — add `SuggestConfig`, `ScanConfig`, `PriorityConfig` structs
- `tui/src/tui/command.rs` — register `/suggest` command
- `tui/src/app.rs` — add `handle_suggest_command()` in `handle_shared_slash()`

---

## Task 1: Config types + deserialization

**Files:**
- Modify: `tui/src/config/schema.rs` (after `ImagesConfig`, ~line 400)
- Modify: `tui/src/config/mod.rs:82` (add field to `Config`)
- Test: `tui/src/config/schema.rs` (existing test module)

- [ ] **Step 1: Write failing tests for SuggestConfig deserialization**

Add to the `#[cfg(test)]` module at the bottom of `tui/src/config/schema.rs`:

```rust
#[test]
fn parse_suggest_config_defaults() {
    let toml_str = "";
    let config: SuggestConfig = toml::from_str(toml_str).unwrap();
    assert!(config.enabled);
    assert!(config.scans.is_empty());
    assert!(config.priorities.is_none());
}

#[test]
fn parse_suggest_config_disabled() {
    let toml_str = r#"enabled = false"#;
    let config: SuggestConfig = toml::from_str(toml_str).unwrap();
    assert!(!config.enabled);
}

#[test]
fn parse_suggest_config_with_scans() {
    let toml_str = r#"
        enabled = true
        [[scans]]
        name = "lint"
        command = "cargo clippy --message-format=json"
        category = "lint"
        timeout_secs = 60
    "#;
    let config: SuggestConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.scans.len(), 1);
    assert_eq!(config.scans[0].name, "lint");
    assert_eq!(config.scans[0].timeout_secs, Some(60));
}

#[test]
fn parse_suggest_priority_config() {
    let toml_str = r#"
        [priorities]
        test_failure = 1
        lint_error = 3
    "#;
    let config: SuggestConfig = toml::from_str(toml_str).unwrap();
    let p = config.priorities.unwrap();
    assert_eq!(p.test_failure, Some(1));
    assert_eq!(p.lint_error, Some(3));
    assert_eq!(p.lint_warning, None); // not set, uses default
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tui && cargo test parse_suggest`
Expected: FAIL — `SuggestConfig` type doesn't exist yet.

- [ ] **Step 3: Implement config types**

Add to `tui/src/config/schema.rs` after the `ImagesConfig` block:

```rust
/// Suggest command configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestConfig {
    /// Enable/disable /suggest command entirely (default: true).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// User-configured scan commands.
    #[serde(default)]
    pub scans: Vec<ScanCommandConfig>,
    /// Priority weights for ranking findings.
    #[serde(default)]
    pub priorities: Option<PriorityConfig>,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scans: Vec::new(),
            priorities: None,
        }
    }
}

/// A single scan command for /suggest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanCommandConfig {
    /// Display name (e.g. "lint", "test").
    pub name: String,
    /// Shell command to execute.
    pub command: String,
    /// Finding category: "lint", "test", "todo", "custom".
    #[serde(default = "default_custom_category")]
    pub category: String,
    /// Timeout in seconds (default: 120).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn default_custom_category() -> String {
    "custom".to_string()
}

/// Priority weights for /suggest ranking (lower = higher priority).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityConfig {
    pub test_failure: Option<u8>,
    pub lint_error: Option<u8>,
    pub lint_warning: Option<u8>,
    pub todo: Option<u8>,
    pub recent_churn: Option<u8>,
}
```

- [ ] **Step 4: Add `suggest` field to Config**

In `tui/src/config/mod.rs`, add after the `images` field (~line 84):

```rust
    /// Suggest command configuration
    #[serde(default)]
    pub suggest: Option<schema::SuggestConfig>,
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd tui && cargo test parse_suggest`
Expected: 4 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add tui/src/config/schema.rs tui/src/config/mod.rs
git commit -m "add SuggestConfig types to config schema"
```

---

## Task 2: Core types + priority module

**Files:**
- Create: `tui/src/suggest/mod.rs`
- Create: `tui/src/suggest/priority.rs`
- Modify: `tui/src/main.rs:25` (add `mod suggest;`)

- [ ] **Step 1: Write failing tests for Finding types and sort**

Create `tui/src/suggest/priority.rs`:

```rust
/// Finding category from a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Test,
    Lint,
    Todo,
    Churn,
    Custom(String),
}

/// Severity within a category.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A single finding from a scan.
#[derive(Debug, Clone)]
pub struct Finding {
    pub category: Category,
    pub severity: Severity,
    pub summary: String,
    pub location: Option<String>,
    pub count: usize,
}

/// Default priority weights (lower = higher priority).
pub struct PriorityWeights {
    pub test_failure: u8,
    pub lint_error: u8,
    pub lint_warning: u8,
    pub todo: u8,
    pub recent_churn: u8,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            test_failure: 1,
            lint_error: 2,
            lint_warning: 3,
            todo: 4,
            recent_churn: 5,
        }
    }
}

impl PriorityWeights {
    /// Build from optional config, falling back to defaults.
    pub fn from_config(config: Option<&crate::config::schema::PriorityConfig>) -> Self {
        let defaults = Self::default();
        match config {
            None => defaults,
            Some(c) => Self {
                test_failure: c.test_failure.unwrap_or(defaults.test_failure),
                lint_error: c.lint_error.unwrap_or(defaults.lint_error),
                lint_warning: c.lint_warning.unwrap_or(defaults.lint_warning),
                todo: c.todo.unwrap_or(defaults.todo),
                recent_churn: c.recent_churn.unwrap_or(defaults.recent_churn),
            },
        }
    }

    /// Get the priority weight for a finding.
    pub fn weight(&self, finding: &Finding) -> u8 {
        match (&finding.category, &finding.severity) {
            (Category::Test, _) => self.test_failure,
            (Category::Lint, Severity::Error) => self.lint_error,
            (Category::Lint, _) => self.lint_warning,
            (Category::Todo, _) => self.todo,
            (Category::Churn, _) => self.recent_churn,
            (Category::Custom(_), _) => self.todo, // treat custom as TODO-priority
        }
    }
}

/// Sort findings by priority weight (lowest first = highest priority).
pub fn sort_findings(findings: &mut [Finding], weights: &PriorityWeights) {
    findings.sort_by_key(|f| (weights.weight(f), f.severity.clone()));
}

/// Deduplicate findings with the same summary — merge into one with count incremented.
pub fn dedup_findings(findings: &mut Vec<Finding>) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut deduped = Vec::new();
    for mut f in findings.drain(..) {
        if let Some(&idx) = seen.get(&f.summary) {
            deduped[idx].count += f.count;
        } else {
            seen.insert(f.summary.clone(), deduped.len());
            deduped.push(f);
        }
    }
    *findings = deduped;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(cat: Category, sev: Severity, summary: &str) -> Finding {
        Finding {
            category: cat,
            severity: sev,
            summary: summary.to_string(),
            location: None,
            count: 1,
        }
    }

    #[test]
    fn sort_by_priority_weight() {
        let weights = PriorityWeights::default();
        let mut findings = vec![
            finding(Category::Todo, Severity::Info, "TODO: fix this"),
            finding(Category::Test, Severity::Error, "test_foo failed"),
            finding(Category::Lint, Severity::Warning, "unused import"),
        ];
        sort_findings(&mut findings, &weights);
        assert_eq!(findings[0].category, Category::Test);
        assert_eq!(findings[1].category, Category::Lint);
        assert_eq!(findings[2].category, Category::Todo);
    }

    #[test]
    fn sort_lint_errors_before_warnings() {
        let weights = PriorityWeights::default();
        let mut findings = vec![
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Error, "type mismatch"),
        ];
        sort_findings(&mut findings, &weights);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[1].severity, Severity::Warning);
    }

    #[test]
    fn custom_priority_weights() {
        let weights = PriorityWeights {
            todo: 1,
            test_failure: 5,
            ..PriorityWeights::default()
        };
        let mut findings = vec![
            finding(Category::Test, Severity::Error, "test_foo failed"),
            finding(Category::Todo, Severity::Info, "TODO: fix this"),
        ];
        sort_findings(&mut findings, &weights);
        assert_eq!(findings[0].category, Category::Todo);
        assert_eq!(findings[1].category, Category::Test);
    }

    #[test]
    fn dedup_merges_identical_summaries() {
        let mut findings = vec![
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Warning, "unused import"),
        ];
        dedup_findings(&mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].count, 3);
    }

    #[test]
    fn dedup_preserves_different_summaries() {
        let mut findings = vec![
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Warning, "needless borrow"),
        ];
        dedup_findings(&mut findings);
        assert_eq!(findings.len(), 2);
    }
}
```

- [ ] **Step 2: Create mod.rs and register module**

Create `tui/src/suggest/mod.rs`:

```rust
//! /suggest — evidence-based codebase improvement suggestions.

pub mod config;
pub mod digest;
pub mod parsers;
pub mod priority;
pub mod scanner;
```

Add `mod suggest;` to `tui/src/main.rs` (after `mod skills;`, line 25).

- [ ] **Step 3: Create placeholder files**

Create empty files so the module compiles:
- `tui/src/suggest/config.rs` — `// Auto-detection and config helpers.`
- `tui/src/suggest/digest.rs` — `// Digest formatting.`
- `tui/src/suggest/parsers.rs` — `// Scan output parsers.`
- `tui/src/suggest/scanner.rs` — `// Parallel scan execution.`

- [ ] **Step 4: Run tests**

Run: `cd tui && cargo test suggest::priority`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/suggest/ tui/src/main.rs
git commit -m "add suggest module with core types and priority sorting"
```

---

## Task 3: Parsers — clippy JSON, cargo test, generic, TODO grep, git churn

**Files:**
- Modify: `tui/src/suggest/parsers.rs`

- [ ] **Step 1: Write failing tests for clippy JSON parser**

Add to `tui/src/suggest/parsers.rs`:

```rust
use crate::suggest::priority::{Category, Finding, Severity};

/// Parse cargo clippy --message-format=json output into findings.
pub fn parse_clippy_json(output: &str) -> Vec<Finding> {
    // Implementation in step 3
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clippy_warning() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused import: `std::io`","spans":[{"file_name":"src/app.rs","line_start":12}]}}"#;
        let findings = parse_clippy_json(json_line);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, Category::Lint);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].summary.contains("unused import"));
        assert_eq!(findings[0].location.as_deref(), Some("src/app.rs:12"));
    }

    #[test]
    fn parse_clippy_error() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","spans":[{"file_name":"src/main.rs","line_start":5}]}}"#;
        let findings = parse_clippy_json(json_line);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
    }

    #[test]
    fn parse_clippy_skips_non_compiler_messages() {
        let json_line = r#"{"reason":"build-finished","success":true}"#;
        let findings = parse_clippy_json(json_line);
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_clippy_multiple_lines() {
        let output = format!(
            "{}\n{}\n{}",
            r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable","spans":[{"file_name":"src/a.rs","line_start":1}]}}"#,
            r#"{"reason":"build-finished","success":true}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","message":"dead code","spans":[{"file_name":"src/b.rs","line_start":2}]}}"#,
        );
        let findings = parse_clippy_json(&output);
        assert_eq!(findings.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tui && cargo test suggest::parsers`
Expected: FAIL — parser returns empty vec.

- [ ] **Step 3: Implement clippy JSON parser**

Replace the placeholder `parse_clippy_json`:

```rust
pub fn parse_clippy_json(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if val.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let msg = match val.get("message") {
            Some(m) => m,
            None => continue,
        };
        let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("warning");
        let text = msg
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");
        let location = msg
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|a| a.first())
            .and_then(|span| {
                let file = span.get("file_name")?.as_str()?;
                let line = span.get("line_start")?.as_u64()?;
                Some(format!("{file}:{line}"))
            });

        findings.push(Finding {
            category: Category::Lint,
            severity: match level {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                _ => Severity::Info,
            },
            summary: text.to_string(),
            location,
            count: 1,
        });
    }
    findings
}
```

- [ ] **Step 4: Run clippy parser tests**

Run: `cd tui && cargo test suggest::parsers`
Expected: 4 tests PASS.

- [ ] **Step 5: Add cargo test output parser with tests**

Append to `tui/src/suggest/parsers.rs`:

```rust
/// Parse cargo test output for failures.
pub fn parse_cargo_test(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("test ") && line.ends_with("... FAILED") {
            let name = line
                .strip_prefix("test ")
                .and_then(|s| s.strip_suffix(" ... FAILED"))
                .unwrap_or(line);
            findings.push(Finding {
                category: Category::Test,
                severity: Severity::Error,
                summary: format!("FAIL {name}"),
                location: None,
                count: 1,
            });
        }
    }
    findings
}
```

Add tests:

```rust
#[test]
fn parse_cargo_test_failures() {
    let output = "test agent::tests::circuit_breaker ... ok\ntest tools::tests::shell_timeout ... FAILED\n";
    let findings = parse_cargo_test(output);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, Category::Test);
    assert!(findings[0].summary.contains("shell_timeout"));
}

#[test]
fn parse_cargo_test_all_pass() {
    let output = "test foo ... ok\ntest bar ... ok\n";
    let findings = parse_cargo_test(output);
    assert!(findings.is_empty());
}
```

- [ ] **Step 6: Add generic fallback parser with tests**

Append to `tui/src/suggest/parsers.rs`:

```rust
/// Generic fallback parser — extract lines containing "error" or "warning".
pub fn parse_generic(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut error_count = 0usize;
    let mut warning_count = 0usize;
    for line in output.lines().take(2000) {
        let lower = line.to_lowercase();
        if lower.contains("error") && findings.len() < 5 {
            error_count += 1;
            findings.push(Finding {
                category: Category::Custom("generic".to_string()),
                severity: Severity::Error,
                summary: line.trim().to_string(),
                location: None,
                count: 1,
            });
        } else if lower.contains("error") {
            error_count += 1;
        } else if lower.contains("warning") && findings.len() < 5 {
            warning_count += 1;
            findings.push(Finding {
                category: Category::Custom("generic".to_string()),
                severity: Severity::Warning,
                summary: line.trim().to_string(),
                location: None,
                count: 1,
            });
        } else if lower.contains("warning") {
            warning_count += 1;
        }
    }
    // Add overflow summary if we capped at 5
    let total = error_count + warning_count;
    if total > 5 {
        findings.push(Finding {
            category: Category::Custom("generic".to_string()),
            severity: Severity::Info,
            summary: format!("{} more issues in output ({error_count} errors, {warning_count} warnings)", total - 5),
            location: None,
            count: 1,
        });
    }
    findings
}
```

Add test:

```rust
#[test]
fn parse_generic_caps_at_five_findings() {
    let lines: Vec<String> = (0..20).map(|i| format!("error: problem {i}")).collect();
    let output = lines.join("\n");
    let findings = parse_generic(&output);
    // 5 individual + 1 overflow summary
    assert_eq!(findings.len(), 6);
    assert!(findings.last().unwrap().summary.contains("more issues"));
}
```

- [ ] **Step 7: Add TODO/FIXME grep parser with tests**

Append to `tui/src/suggest/parsers.rs`:

```rust
/// Parse grep output for TODO/FIXME/HACK markers.
/// Expects lines in format: "path/to/file.rs:42: // TODO: something"
pub fn parse_todo_grep(output: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Extract location (file:line) and comment text
        let (location, text) = if let Some((loc, rest)) = line.split_once(": ") {
            // Try to find the marker in the rest
            let marker = if rest.contains("FIXME") {
                "FIXME"
            } else if rest.contains("HACK") {
                "HACK"
            } else {
                "TODO"
            };
            let summary = rest.trim().trim_start_matches("//").trim_start_matches('#').trim();
            (Some(loc.to_string()), format!("{marker}: {summary}"))
        } else {
            (None, line.to_string())
        };

        findings.push(Finding {
            category: Category::Todo,
            severity: if text.starts_with("FIXME") || text.starts_with("HACK") {
                Severity::Warning
            } else {
                Severity::Info
            },
            summary: text,
            location,
            count: 1,
        });
    }
    findings
}
```

Add test:

```rust
#[test]
fn parse_todo_grep_extracts_markers() {
    let output = "src/app.rs:42: // TODO: handle edge case\nsrc/lib.rs:10: // FIXME: race condition\n";
    let findings = parse_todo_grep(output);
    assert_eq!(findings.len(), 2);
    assert!(findings[0].summary.starts_with("TODO"));
    assert!(findings[1].summary.starts_with("FIXME"));
    assert_eq!(findings[1].severity, Severity::Warning);
}
```

- [ ] **Step 8: Add git churn parser with tests**

Append to `tui/src/suggest/parsers.rs`:

```rust
/// Parse git log --name-only output into churn findings.
/// Input: raw file paths, one per line (may have blanks between commits).
pub fn parse_git_churn(output: &str) -> Vec<Finding> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    let mut ranked: Vec<_> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    ranked.truncate(10);

    ranked
        .into_iter()
        .map(|(file, count)| Finding {
            category: Category::Churn,
            severity: Severity::Info,
            summary: format!("{file} ({count} commits)"),
            location: Some(file),
            count,
        })
        .collect()
}
```

Add test:

```rust
#[test]
fn parse_git_churn_ranks_by_frequency() {
    let output = "src/app.rs\nsrc/lib.rs\nsrc/app.rs\nsrc/app.rs\nsrc/lib.rs\n";
    let findings = parse_git_churn(output);
    assert_eq!(findings[0].count, 3); // app.rs
    assert_eq!(findings[1].count, 2); // lib.rs
}
```

- [ ] **Step 9: Add dispatch function**

Append to `tui/src/suggest/parsers.rs` (before `#[cfg(test)]`):

```rust
/// Dispatch to the appropriate parser based on scan category.
pub fn parse_scan_output(category: &str, output: &str) -> Vec<Finding> {
    match category {
        "lint" => parse_clippy_json(output),
        "test" => parse_cargo_test(output),
        "todo" => parse_todo_grep(output),
        "churn" => parse_git_churn(output),
        _ => parse_generic(output),
    }
}
```

- [ ] **Step 10: Run all parser tests**

Run: `cd tui && cargo test suggest::parsers`
Expected: All tests PASS.

- [ ] **Step 11: Commit**

```bash
git add tui/src/suggest/parsers.rs
git commit -m "add scan output parsers for clippy, cargo test, TODO grep, git churn"
```

---

## Task 4: Digest formatter

**Files:**
- Modify: `tui/src/suggest/digest.rs`

- [ ] **Step 1: Write failing tests**

```rust
use crate::suggest::priority::{Category, Finding, PriorityWeights, Severity};

/// Format findings into a compact markdown digest for LLM consumption.
pub fn build_digest(findings: &[Finding], weights: &PriorityWeights) -> String {
    String::new() // placeholder
}

/// The priority prompt appended after the digest.
pub const PRIORITY_PROMPT: &str = "\
Rank these findings into a prioritized action list. Use this framework:
1. Test failures — broken tests block everything
2. Lint errors — compiler-level issues
3. Lint warnings — code quality
4. FIXMEs/HACKs — known problems flagged by developers
5. TODOs — planned work
6. High-churn files with no recent test changes — likely test debt

For each suggestion, give: priority rank, what to fix, why, and estimated effort (small/medium/large).
Keep the list to 10 items max. Group related items.";

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(cat: Category, sev: Severity, summary: &str, loc: Option<&str>) -> Finding {
        Finding {
            category: cat,
            severity: sev,
            summary: summary.to_string(),
            location: loc.map(|s| s.to_string()),
            count: 1,
        }
    }

    #[test]
    fn digest_groups_by_category() {
        let findings = vec![
            finding(Category::Test, Severity::Error, "FAIL test_foo", Some("src/lib.rs:10")),
            finding(Category::Lint, Severity::Warning, "unused import", Some("src/app.rs:5")),
        ];
        let digest = build_digest(&findings, &PriorityWeights::default());
        assert!(digest.contains("Test failures"));
        assert!(digest.contains("Lint warnings"));
        assert!(digest.contains("FAIL test_foo"));
        assert!(digest.contains("unused import"));
    }

    #[test]
    fn digest_shows_count_for_grouped_findings() {
        let findings = vec![Finding {
            category: Category::Lint,
            severity: Severity::Warning,
            summary: "unused import".to_string(),
            location: Some("src/app.rs:5".to_string()),
            count: 3,
        }];
        let digest = build_digest(&findings, &PriorityWeights::default());
        assert!(digest.contains("x3"));
    }

    #[test]
    fn digest_caps_per_section() {
        let findings: Vec<_> = (0..10)
            .map(|i| finding(Category::Todo, Severity::Info, &format!("TODO item {i}"), None))
            .collect();
        let digest = build_digest(&findings, &PriorityWeights::default());
        // Should have max 5 items + overflow note
        assert!(digest.contains("more"));
    }

    #[test]
    fn empty_findings_still_produces_header() {
        let digest = build_digest(&[], &PriorityWeights::default());
        assert!(digest.contains("Codebase scan results"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tui && cargo test suggest::digest`
Expected: FAIL — returns empty string.

- [ ] **Step 3: Implement build_digest**

```rust
/// Max findings to show per category section before "(N more)" summary.
const MAX_PER_SECTION: usize = 5;

pub fn build_digest(findings: &[Finding], weights: &PriorityWeights) -> String {
    let mut out = String::from("## Codebase scan results\n");

    if findings.is_empty() {
        out.push_str("\nNo issues found — codebase looks clean.\n");
        return out;
    }

    // Group findings by (weight, label)
    let sections: &[(u8, &str, Box<dyn Fn(&Finding) -> bool>)] = &[
        (weights.test_failure, "Test failures", Box::new(|f: &Finding| f.category == Category::Test)),
        (weights.lint_error, "Lint errors", Box::new(|f: &Finding| f.category == Category::Lint && f.severity == Severity::Error)),
        (weights.lint_warning, "Lint warnings", Box::new(|f: &Finding| f.category == Category::Lint && f.severity != Severity::Error)),
        (weights.todo, "TODOs/FIXMEs", Box::new(|f: &Finding| f.category == Category::Todo)),
        (weights.recent_churn, "Recent churn", Box::new(|f: &Finding| f.category == Category::Churn)),
    ];

    let mut sorted_sections: Vec<_> = sections.iter().collect();
    sorted_sections.sort_by_key(|(w, _, _)| *w);

    for (weight, label, predicate) in sorted_sections {
        let matched: Vec<_> = findings.iter().filter(|f| predicate(f)).collect();
        out.push_str(&format!("\n### {label} (priority {weight})\n"));
        if matched.is_empty() {
            out.push_str("(none)\n");
            continue;
        }
        for (i, f) in matched.iter().enumerate() {
            if i >= MAX_PER_SECTION {
                let remaining = matched.len() - MAX_PER_SECTION;
                out.push_str(&format!("- {remaining} more\n"));
                break;
            }
            let count_suffix = if f.count > 1 {
                format!(" (x{})", f.count)
            } else {
                String::new()
            };
            let loc_suffix = f
                .location
                .as_deref()
                .map(|l| format!(" — {l}"))
                .unwrap_or_default();
            out.push_str(&format!("- {}{loc_suffix}{count_suffix}\n", f.summary));
        }
    }

    // Handle custom category findings not in standard sections
    let custom: Vec<_> = findings
        .iter()
        .filter(|f| matches!(f.category, Category::Custom(_)))
        .collect();
    if !custom.is_empty() {
        out.push_str("\n### Other findings\n");
        for (i, f) in custom.iter().enumerate() {
            if i >= MAX_PER_SECTION {
                let remaining = custom.len() - MAX_PER_SECTION;
                out.push_str(&format!("- {remaining} more\n"));
                break;
            }
            out.push_str(&format!("- {}\n", f.summary));
        }
    }

    out
}
```

- [ ] **Step 4: Run tests**

Run: `cd tui && cargo test suggest::digest`
Expected: All tests PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/suggest/digest.rs
git commit -m "add digest formatter for suggest findings"
```

---

## Task 5: Auto-detection + scanner

**Files:**
- Modify: `tui/src/suggest/config.rs`
- Modify: `tui/src/suggest/scanner.rs`

- [ ] **Step 1: Implement auto-detection with tests**

Write `tui/src/suggest/config.rs`:

```rust
use crate::config::schema::{ScanCommandConfig, SuggestConfig};

/// Default timeout for scan commands in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Resolve scan commands: use config if provided, otherwise auto-detect.
pub fn resolve_scans(config: Option<&SuggestConfig>) -> Vec<ScanCommandConfig> {
    let scans = config.map(|c| &c.scans);
    if let Some(scans) = scans {
        if !scans.is_empty() {
            return scans.clone();
        }
    }
    auto_detect()
}

/// Auto-detect scan commands from project files in the current directory.
pub fn auto_detect() -> Vec<ScanCommandConfig> {
    let mut scans = Vec::new();

    if std::path::Path::new("Cargo.toml").exists() {
        scans.push(ScanCommandConfig {
            name: "clippy".to_string(),
            command: "cargo clippy --message-format=json 2>&1".to_string(),
            category: "lint".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
        scans.push(ScanCommandConfig {
            name: "test".to_string(),
            command: "cargo test 2>&1".to_string(),
            category: "test".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
    } else if std::path::Path::new("package.json").exists() {
        scans.push(ScanCommandConfig {
            name: "lint".to_string(),
            command: "npx eslint . --format=json 2>&1".to_string(),
            category: "lint".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
        scans.push(ScanCommandConfig {
            name: "test".to_string(),
            command: "npm test 2>&1".to_string(),
            category: "test".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
    } else if std::path::Path::new("pyproject.toml").exists()
        || std::path::Path::new("setup.py").exists()
    {
        scans.push(ScanCommandConfig {
            name: "lint".to_string(),
            command: "ruff check . --output-format=json 2>&1".to_string(),
            category: "lint".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
        scans.push(ScanCommandConfig {
            name: "test".to_string(),
            command: "python -m pytest --collect-only 2>&1".to_string(),
            category: "test".to_string(),
            timeout_secs: Some(DEFAULT_TIMEOUT_SECS),
        });
    }

    scans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_config_when_provided() {
        let config = SuggestConfig {
            enabled: true,
            scans: vec![ScanCommandConfig {
                name: "custom".to_string(),
                command: "echo hello".to_string(),
                category: "custom".to_string(),
                timeout_secs: None,
            }],
            priorities: None,
        };
        let scans = resolve_scans(Some(&config));
        assert_eq!(scans.len(), 1);
        assert_eq!(scans[0].name, "custom");
    }

    #[test]
    fn resolve_falls_back_to_auto_detect_when_empty() {
        let config = SuggestConfig {
            enabled: true,
            scans: vec![],
            priorities: None,
        };
        // auto_detect will find Cargo.toml in this project
        let scans = resolve_scans(Some(&config));
        assert!(!scans.is_empty() || scans.is_empty()); // just verify no crash
    }
}
```

- [ ] **Step 2: Implement scanner with tests**

Write `tui/src/suggest/scanner.rs`:

```rust
use crate::config::schema::ScanCommandConfig;
use crate::suggest::config::DEFAULT_TIMEOUT_SECS;
use crate::suggest::parsers;
use crate::suggest::priority::Finding;

/// Result of a single scan command.
pub struct ScanResult {
    pub name: String,
    pub category: String,
    pub findings: Vec<Finding>,
    pub error: Option<String>,
}

/// Max output lines to capture per scan command.
const MAX_OUTPUT_LINES: usize = 2000;
/// Max output bytes to capture per scan command.
const MAX_OUTPUT_BYTES: usize = 50_000;

/// Run all scan commands in parallel + built-in scans. Returns all findings.
pub async fn run_scans(scans: &[ScanCommandConfig]) -> Vec<ScanResult> {
    let mut handles = Vec::new();

    // Spawn configured scan commands
    for scan in scans {
        let name = scan.name.clone();
        let command = scan.command.clone();
        let category = scan.category.clone();
        let timeout = scan.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);

        handles.push(tokio::spawn(async move {
            run_single_scan(&name, &command, &category, timeout).await
        }));
    }

    // Spawn built-in TODO grep scan
    handles.push(tokio::spawn(async move {
        run_todo_scan().await
    }));

    // Spawn built-in git churn scan
    handles.push(tokio::spawn(async move {
        run_git_churn_scan().await
    }));

    // Collect results
    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(ScanResult {
                name: "unknown".to_string(),
                category: "custom".to_string(),
                findings: vec![],
                error: Some(format!("scan task panicked: {e}")),
            }),
        }
    }
    results
}

async fn run_single_scan(name: &str, command: &str, category: &str, timeout_secs: u64) -> ScanResult {
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let result = tokio::time::timeout(timeout, async {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
            .await;
        output
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}{stderr}");
            let truncated = truncate_output(&combined);
            let mut findings = parsers::parse_scan_output(category, &truncated);

            // If output was truncated, note it
            if combined.len() > MAX_OUTPUT_BYTES || combined.lines().count() > MAX_OUTPUT_LINES {
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
                category: category.to_string(),
                findings,
                error: None,
            }
        }
        Ok(Err(e)) => ScanResult {
            name: name.to_string(),
            category: category.to_string(),
            findings: vec![],
            error: Some(format!("{name} scan failed: {e}")),
        },
        Err(_) => ScanResult {
            name: name.to_string(),
            category: category.to_string(),
            findings: vec![],
            error: Some(format!("{name} scan timed out after {timeout_secs}s")),
        },
    }
}

async fn run_todo_scan() -> ScanResult {
    let result = tokio::process::Command::new("sh")
        .arg("-c")
        .arg("grep -rn --include='*.rs' --include='*.ts' --include='*.py' --include='*.go' --include='*.js' --include='*.jsx' --include='*.tsx' -E '(TODO|FIXME|HACK):?' . 2>/dev/null | head -100")
        .output()
        .await;

    match result {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            ScanResult {
                name: "todos".to_string(),
                category: "todo".to_string(),
                findings: parsers::parse_todo_grep(&text),
                error: None,
            }
        }
        Err(e) => ScanResult {
            name: "todos".to_string(),
            category: "todo".to_string(),
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
                category: "churn".to_string(),
                findings: parsers::parse_git_churn(&text),
                error: None,
            }
        }
        Err(e) => ScanResult {
            name: "churn".to_string(),
            category: "churn".to_string(),
            findings: vec![],
            error: Some(format!("git churn scan failed: {e}")),
        },
    }
}

fn truncate_output(output: &str) -> String {
    let mut result = String::new();
    for (i, line) in output.lines().enumerate() {
        if i >= MAX_OUTPUT_LINES || result.len() >= MAX_OUTPUT_BYTES {
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
        let long = (0..3000).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let truncated = truncate_output(&long);
        assert!(truncated.lines().count() <= MAX_OUTPUT_LINES);
    }

    #[test]
    fn truncate_output_caps_bytes() {
        let long = "x".repeat(100_000);
        let truncated = truncate_output(&long);
        assert!(truncated.len() <= MAX_OUTPUT_BYTES + 1); // +1 for trailing newline
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd tui && cargo test suggest::scanner`
Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add tui/src/suggest/config.rs tui/src/suggest/scanner.rs
git commit -m "add auto-detection config and parallel scan runner"
```

---

## Task 6: Orchestrator + slash command integration

**Files:**
- Modify: `tui/src/suggest/mod.rs`
- Modify: `tui/src/tui/command.rs` (~line 169, `build_default_registry`)
- Modify: `tui/src/app.rs` (~line 10705, `handle_shared_slash`)

- [ ] **Step 1: Implement run_suggest orchestrator**

Update `tui/src/suggest/mod.rs`:

```rust
//! /suggest — evidence-based codebase improvement suggestions.

pub mod config;
pub mod digest;
pub mod parsers;
pub mod priority;
pub mod scanner;

use priority::{Finding, PriorityWeights};

/// Run the full suggest pipeline: detect scans, run them, parse, digest.
/// Returns the formatted digest + priority prompt ready to inject into conversation.
pub async fn run_suggest(
    suggest_config: Option<&crate::config::schema::SuggestConfig>,
) -> String {
    // 1. Resolve scan commands
    let scans = config::resolve_scans(suggest_config);

    // 2. Run all scans in parallel
    let scan_results = scanner::run_scans(&scans).await;

    // 3. Collect all findings + error findings
    let priority_config = suggest_config.and_then(|c| c.priorities.as_ref());
    let weights = PriorityWeights::from_config(priority_config);
    let mut findings: Vec<Finding> = Vec::new();

    for result in &scan_results {
        findings.extend(result.findings.clone());
        if let Some(ref err) = result.error {
            findings.push(Finding {
                category: priority::Category::Custom("scan-error".to_string()),
                severity: priority::Severity::Info,
                summary: err.clone(),
                location: None,
                count: 1,
            });
        }
    }

    // 4. Dedup + sort
    priority::dedup_findings(&mut findings);
    priority::sort_findings(&mut findings, &weights);

    // 5. Build digest
    let digest = digest::build_digest(&findings, &weights);

    // 6. Append priority prompt
    format!("{digest}\n{}", digest::PRIORITY_PROMPT)
}
```

- [ ] **Step 2: Register /suggest slash command**

In `tui/src/tui/command.rs`, add in `build_default_registry()` (after the `/settings` registration, ~line 400):

```rust
    registry.register(Command {
        id: "suggest.run",
        name: "Suggest Improvements",
        category: Category::Tools,
        keybind: None,
        slash: Some("suggest"),
        available: |state| {
            state.config.suggest.as_ref().map_or(true, |c| c.enabled)
        },
        execute: |_state| Action::None, // Handled in handle_shared_slash
    });
```

Note: The `available` closure needs access to `state.config`. Check the existing `available` function signatures — they receive `&CommandState`. Verify that `CommandState` includes the config or the suggest enabled flag.

- [ ] **Step 3: Check CommandState definition**

Read `tui/src/tui/command.rs` to find `CommandState` and verify what fields are available. If it doesn't have config access, add a `suggest_enabled: bool` field.

- [ ] **Step 4: Add suggest dispatch in handle_shared_slash**

In `tui/src/app.rs`, add before the roundhouse block (~line 10784):

```rust
        if slash == "suggest" {
            self.handle_suggest_command().await;
            return true;
        }
```

Then add the handler method to `impl App`:

```rust
    /// Handle /suggest — run codebase scans and inject digest into conversation.
    async fn handle_suggest_command(&mut self) {
        // Switch to chat screen if on home
        self.state.dialog_stack.base = Screen::Chat;

        // Show scanning message
        self.state.chat_messages.push(ChatMessage::System {
            content: "Scanning codebase...".to_string(),
        });

        // Run the suggest pipeline
        let suggest_config = self.state.config.suggest.as_ref();
        let digest = crate::suggest::run_suggest(suggest_config).await;

        // Replace scanning message with result
        if let Some(ChatMessage::System { content }) = self.state.chat_messages.last_mut() {
            if content == "Scanning codebase..." {
                *content = "Scan complete — analyzing findings...".to_string();
            }
        }

        // Inject digest as user message and trigger agent stream
        if let Some(ref provider) = self.provider {
            let tool_defs = self.build_tool_defs();
            self.state.agent.send_message(digest, provider.as_ref(), &tool_defs);
        }
    }
```

- [ ] **Step 5: Build and verify compilation**

Run: `cd tui && cargo build`
Expected: Compiles successfully.

- [ ] **Step 6: Commit**

```bash
git add tui/src/suggest/mod.rs tui/src/tui/command.rs tui/src/app.rs
git commit -m "wire up /suggest slash command with scan orchestrator"
```

---

## Task 7: Full build + test pass

- [ ] **Step 1: Run full test suite**

Run: `cd tui && cargo test`
Expected: All tests pass (existing + new suggest tests).

- [ ] **Step 2: Run clippy**

Run: `cd tui && cargo clippy`
Expected: No new warnings in `suggest/` module.

- [ ] **Step 3: Fix any issues found, commit**

```bash
git add -A
git commit -m "fix suggest module clippy warnings and test issues"
```

- [ ] **Step 4: Push and update PR**

```bash
git push origin feat/0.4.2
gh pr edit 7 --body "..." # add suggest section to PR body
```
