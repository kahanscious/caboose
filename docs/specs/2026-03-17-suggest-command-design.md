# `/suggest` — Evidence-Based Codebase Improvement Suggestions

## Overview

`/suggest` scans the codebase by running configured commands (lint, test, TODO grep, git log), parses the output into typed findings, compresses them into a compact digest, and sends that digest to the LLM with a priority framework. The LLM produces a ranked list of actionable suggestions in chat.

The key design constraint is **context efficiency**: raw command output never enters the conversation. Findings are parsed, deduplicated, counted, and compressed locally before the LLM sees them.

## Config Schema

New `suggest` section in settings:

```toml
[suggest]
enabled = true  # false removes /suggest from slash commands and skill awareness

# User-configured scan commands
[[suggest.scans]]
name = "lint"
command = "cargo clippy --message-format=json 2>&1"
category = "lint"          # lint | test | todo | custom
timeout_secs = 120

[[suggest.scans]]
name = "test"
command = "cargo test --no-run 2>&1"
category = "test"
timeout_secs = 120

# Built-in scans (always run unless disabled, no config needed):
# - TODO/FIXME/HACK grep across workspace files
# - git log for recent commit activity (last 20 commits)

# Priority weights (optional override — lower number = higher priority)
[suggest.priorities]
test_failure = 1
lint_error = 2
lint_warning = 3
todo = 4
recent_churn = 5
```

When no `suggest.scans` are configured, auto-detection kicks in:
- `Cargo.toml` present → `cargo clippy --message-format=json`, `cargo test --no-run`
- `package.json` present → `npx eslint . --format=json`, `npm test -- --dry-run`
- `pyproject.toml` / `setup.py` present → `ruff check . --output-format=json`, `python -m pytest --collect-only`
- Fallback: only built-in scans (TODO grep + git log)

## Module Structure

New module: `tui/src/suggest/`

```
suggest/
  mod.rs        — public API: run_suggest() orchestrator
  config.rs     — SuggestConfig types, auto-detection, defaults
  scanner.rs    — runs commands in parallel, captures output
  parsers.rs    — category-specific output parsers
  digest.rs     — compresses Vec<Finding> into text digest
  priority.rs   — priority framework, sorting
```

## Data Types

```rust
pub enum Category {
    Test,
    Lint,
    Todo,
    Churn,
    Custom(String),
}

pub enum Severity {
    Error,
    Warning,
    Info,
}

pub struct Finding {
    pub category: Category,
    pub severity: Severity,
    pub summary: String,         // "unused import `std::io`"
    pub location: Option<String>, // "src/app.rs:42"
    pub count: usize,            // grouped duplicates
}

pub struct ScanResult {
    pub name: String,
    pub category: Category,
    pub exit_code: i32,
    pub findings: Vec<Finding>,
    pub raw_line_count: usize,   // for fallback summary
}
```

## Pipeline Flow

```
/suggest
  1. Load scan config (user-defined or auto-detect from project files)
  2. Run all scan commands in parallel via tokio::spawn
     - Each command has its own timeout (default 120s)
     - Built-in scans (TODO grep, git log) run alongside
  3. Parse each command's output with category-specific parser
     - Cargo clippy JSON → structured lint findings
     - Cargo test → test result line parser
     - Generic fallback → line count + first N error lines
  4. Merge all findings into Vec<Finding>
  5. Deduplicate: group findings with same summary, increment count
  6. Sort by priority framework weights
  7. Compress into text digest (target: 200-500 tokens)
  8. Inject digest + priority prompt into conversation as user message
  9. LLM produces ranked suggestion list in chat
```

## Parsers

### Cargo clippy (JSON message format)

Parses `--message-format=json` output. Each line is a JSON object with `reason`, `message.level`, `message.message`, and `message.spans[0].file_name`/`line_start`. Maps `level` to Severity (error/warning).

### Cargo test

Parses `test result:` summary lines and individual `test ... FAILED` lines. Extracts test names and failure count.

### Generic fallback

For any command without a dedicated parser: counts output lines, extracts first 5 lines containing "error" or "warning" (case-insensitive), reports as findings with Info severity.

### TODO/FIXME grep (built-in)

Uses the existing `grep` tool infrastructure to scan workspace files for `TODO`, `FIXME`, `HACK` markers. Extracts the comment text and location. Groups by marker type.

### Git log (built-in)

Runs `git log --oneline -20` and `git log --format="" --name-only -20 | sort | uniq -c | sort -rn | head -10` to identify high-churn files. Reports as Churn findings.

## Digest Format

Example output sent to the LLM:

```
## Codebase scan results

### Test failures (priority 1)
- FAIL test_session_restore — src/sessions/mod.rs:234

### Lint errors (priority 2)
(none)

### Lint warnings (priority 3)
- unused import `std::io` — src/app.rs:12 (x3 similar)
- needless borrow — src/agent/mod.rs:87

### TODOs/FIXMEs (priority 4)
- TODO: handle timeout edge case — src/tools/shell.rs:45
- FIXME: race condition in tool exec — src/app.rs:7801
- HACK: hardcoded path — src/config/mod.rs:22
- 4 more TODOs across 3 files

### Recent churn (priority 5)
Most-changed files (last 20 commits): app.rs (8), agent/mod.rs (5), tools/shell.rs (3)
```

Within each priority section, findings are capped at 5 items with a "N more" summary to keep the digest bounded.

## Priority Prompt

Appended after the digest when injected into conversation:

```
Rank these findings into a prioritized action list. Use this framework:
1. Test failures — broken tests block everything
2. Lint errors — compiler-level issues
3. Lint warnings — code quality
4. FIXMEs/HACKs — known problems flagged by developers
5. TODOs — planned work
6. High-churn files with no recent test changes — likely test debt

For each suggestion, give: priority rank, what to fix, why, and estimated effort (small/medium/large).
Keep the list to 10 items max. Group related items.
```

## Slash Command Integration

- Register `/suggest` in `CommandRegistry` with `category: Tools`
- Availability gated on `config.suggest.enabled` (default: true)
- When disabled: removed from command registry and skill awareness block
- Execution: `handle_shared_slash` dispatches to `suggest::run_suggest()`
- Shows a "Scanning..." system message while commands run
- On completion, injects digest as user message and triggers agent stream

## Config Integration

Add `SuggestConfig` to the existing config schema:

```rust
pub struct SuggestConfig {
    pub enabled: bool,                    // default: true
    pub scans: Vec<ScanConfig>,           // default: empty (auto-detect)
    pub priorities: Option<PriorityConfig>, // default: built-in weights
}

pub struct ScanConfig {
    pub name: String,
    pub command: String,
    pub category: String,         // "lint", "test", "todo", "custom"
    pub timeout_secs: Option<u64>, // default: 120
}

pub struct PriorityConfig {
    pub test_failure: Option<u8>,  // default: 1
    pub lint_error: Option<u8>,    // default: 2
    pub lint_warning: Option<u8>,  // default: 3
    pub todo: Option<u8>,          // default: 4
    pub recent_churn: Option<u8>,  // default: 5
}
```

## Edge Cases

### Cancellation
If the user sends a new message while scans are running, cancel all in-flight scan tasks via `tokio::select!` / `CancellationToken`. Drop partial results silently.

### No scans available
If no `suggest.scans` are configured and auto-detection finds no project files (no `Cargo.toml`, `package.json`, etc.), still run the built-in scans (TODO grep + git log). If even those produce zero findings, show a system message: "No issues found — codebase looks clean."

### Windows compatibility
The git churn analysis must not rely on Unix pipe chains (`sort | uniq -c`). Implement file-change counting in Rust by parsing `git log --format="" --name-only -20` output directly — split lines, count with a `HashMap<String, usize>`, sort by count descending.

### Command failures
If a scan command fails (non-zero exit, timeout, not found), report it as a single Info-severity finding: "lint scan failed: command not found" rather than crashing the pipeline. Other scans continue independently.

### Large output
Scan command output is capped at the same limits as `run_command` (2000 lines / 50KB). Parsers work on the truncated output. If truncated, append a finding: "output truncated — results may be incomplete."

## Testing Strategy

- **Parser unit tests**: feed known clippy JSON / test output, verify Finding extraction
- **Digest tests**: feed Vec<Finding>, verify output format and token budget
- **Priority tests**: verify sorting with custom weights
- **Auto-detection tests**: mock filesystem with Cargo.toml/package.json, verify inferred commands
- **Config tests**: deserialize TOML config, verify defaults
- **Integration**: no full integration test (requires real project + toolchain), but each stage is independently testable
