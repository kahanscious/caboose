# Core Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract platform-agnostic domain logic from the `tui/` crate into a new `core/` workspace crate, making `tui/` a thin rendering consumer of `caboose-core`.

**Architecture:** Create a Cargo workspace with two members: `core/` (domain logic, no TUI deps) and `tui/` (ratatui frontend). The TUI depends on core. All domain modules move to core; rendering, input handling, and view state stay in TUI. `app.rs` (13.6K lines) splits into an `app/` module tree with domain logic extracted to core.

**Tech Stack:** Rust, Cargo workspaces, tokio, rusqlite, reqwest

**Spec:** `docs/superpowers/specs/2026-03-20-v0.7-architecture-design.md`

---

## Pre-flight

Before starting, verify the baseline:

- [ ] `cd tui && cargo test` — all 1139 tests pass
- [ ] `cd tui && cargo clippy` — clean
- [ ] `git status` — clean working tree on `feat/0.7.0`

---

## Task 1: Create Workspace Structure

**Goal:** Set up the Cargo workspace with `core/` and `tui/` as members. Both compile. Zero logic changes.

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs`
- Modify: `tui/Cargo.toml` (add `caboose-core` dependency)

- [ ] **Step 1:** Create workspace root `Cargo.toml`:
  ```toml
  [workspace]
  members = ["core", "tui"]
  resolver = "2"
  ```

- [ ] **Step 2:** Create `core/Cargo.toml` with shared dependencies extracted from `tui/Cargo.toml`. Core gets: tokio, reqwest, serde, serde_json, toml, dirs, rusqlite, chrono, uuid, anyhow, thiserror, tracing, regex, glob, ignore, futures, async-stream, tokio-stream, sha2, base64, image, png, reqwest-eventsource, eventsource-stream, rmcp, flate2. Core does NOT get: ratatui, crossterm, arboard, portable-pty, vt100, syntect, clap.

- [ ] **Step 3:** Create `core/src/lib.rs` as empty: `// caboose-core — platform-agnostic domain logic`

- [ ] **Step 4:** Add `caboose-core = { path = "../core" }` to `tui/Cargo.toml` dependencies.

- [ ] **Step 5:** Verify: `cargo build` from workspace root compiles both crates.

- [ ] **Step 6:** Verify: `cd tui && cargo test` — all tests still pass.

- [ ] **Step 7:** Commit: `"chore: create workspace with core/ and tui/ crates"`

---

## Task 2: Move Provider Module to Core

**Why first:** Provider has zero TUI dependencies, clean interfaces, lots of tests. Low-risk proving ground for the extraction pattern.

**Files:**
- Move: `tui/src/provider/` → `core/src/provider/`
- Modify: `core/src/lib.rs` (add `pub mod provider`)
- Modify: `tui/src/` (replace `mod provider` with `use caboose_core::provider`)

- [ ] **Step 1:** Copy `tui/src/provider/` to `core/src/provider/`.

- [ ] **Step 2:** Add `pub mod provider;` to `core/src/lib.rs`.

- [ ] **Step 3:** In `tui/src/`, remove `mod provider` declaration. Add `use caboose_core::provider;` or re-export as needed.

- [ ] **Step 4:** Fix all compilation errors in `tui/src/` — these will be import path changes (`crate::provider::` → `caboose_core::provider::`). Work through errors one by one. Do NOT change any logic.

- [ ] **Step 5:** Move provider tests to core. Any test that was in `tui/src/provider/` now lives in `core/src/provider/`. Tests in `tui/` that reference provider types update their imports.

- [ ] **Step 6:** Verify: `cargo test -p caboose-core` — provider tests pass in core.

- [ ] **Step 7:** Verify: `cargo test -p caboose` — all TUI tests pass.

- [ ] **Step 8:** Verify: `cargo clippy --workspace` — clean.

- [ ] **Step 9:** Delete `tui/src/provider/` (now lives in core).

- [ ] **Step 10:** Commit: `"refactor: move provider module to caboose-core"`

---

## Task 3: Move Config Module to Core

**Files:**
- Move: `tui/src/config/` → `core/src/config/`
- Modify: `core/src/lib.rs`
- Modify: all TUI files that import config

**Note:** `config/prefs.rs` imports `ThemeVariant` which is TUI-specific. This file should stay in TUI or the theme type should be a string in core. Handle this during the move.

- [ ] **Step 1:** Copy `tui/src/config/` to `core/src/config/`.

- [ ] **Step 2:** If `prefs.rs` references TUI types, either: (a) keep `prefs.rs` in TUI and have core config re-exported, or (b) replace the TUI type with a string in core.

- [ ] **Step 3:** Add `pub mod config;` to `core/src/lib.rs`.

- [ ] **Step 4:** Update TUI imports. Fix compilation errors.

- [ ] **Step 5:** Verify: `cargo test --workspace` — all tests pass.

- [ ] **Step 6:** Verify: `cargo clippy --workspace` — clean.

- [ ] **Step 7:** Delete `tui/src/config/`, commit: `"refactor: move config module to caboose-core"`

---

## Task 4: Move Session Module to Core

**Files:**
- Move: `tui/src/session/` → `core/src/session/`
- Note: `session/export.rs` may import `ChatMessage` — handle the coupling.

- [ ] **Step 1-7:** Same pattern as Task 2. Copy, declare, fix imports, verify, delete, commit.

- [ ] **Coupling fix:** If `export.rs` imports presentation types, either move it to TUI or have it accept generic input instead of `ChatMessage`.

- [ ] **Commit:** `"refactor: move session module to caboose-core"`

---

## Task 5: Move Remaining Domain Modules (Batch)

Move these modules one at a time, in this order. Each follows the same pattern: copy → declare → fix imports → verify → delete → commit.

The order is chosen to minimize cross-dependency pain (independent modules first):

| Order | Module | Commit message |
|-------|--------|---------------|
| 5a | `safety/` | `"refactor: move safety module to caboose-core"` |
| 5b | `memory/` | `"refactor: move memory module to caboose-core"` |
| 5c | `checkpoint.rs` | `"refactor: move checkpoint module to caboose-core"` |
| 5d | `attachment.rs` | `"refactor: move attachment module to caboose-core"` |
| 5e | `tools/` | `"refactor: move tools module to caboose-core"` |
| 5f | `hooks/` | `"refactor: move hooks module to caboose-core"` |
| 5g | `mcp/` | `"refactor: move mcp module to caboose-core"` |
| 5h | `agent/` | `"refactor: move agent module to caboose-core"` |
| 5i | `sub_agent/` | `"refactor: move sub_agent module to caboose-core"` |
| 5j | `skills/` | `"refactor: move skills module to caboose-core"` |
| 5k | `scm/` | `"refactor: move scm module to caboose-core"` |
| 5l | `suggest/` | `"refactor: move suggest module to caboose-core"` |
| 5m | `roundhouse/` | `"refactor: move roundhouse module to caboose-core"` |
| 5n | `circuits/` | `"refactor: move circuits module to caboose-core"` |
| 5o | `init/` | `"refactor: move init module to caboose-core"` |
| 5p | `migrate/` | `"refactor: move migrate module to caboose-core"` |
| 5q | `agents/` | `"refactor: move agents module to caboose-core"` |

For each module:
- [ ] Copy to `core/src/`
- [ ] Add `pub mod X;` to `core/src/lib.rs`
- [ ] Fix TUI imports (`crate::X` → `caboose_core::X`)
- [ ] If module has cross-deps on other already-moved modules, update to `crate::X` (within core)
- [ ] Verify: `cargo test --workspace` passes
- [ ] Verify: `cargo clippy --workspace` clean
- [ ] Delete from `tui/src/`, commit

**After all modules moved, verify:**
- [ ] `core/src/lib.rs` declares all domain modules
- [ ] `tui/src/` only contains: `main.rs`, `app.rs`, `tui/`, `lsp/`, `terminal/`, `clipboard.rs`, `update.rs`
- [ ] `cargo test --workspace` — all 1139+ tests pass
- [ ] `cargo clippy --workspace` — clean

- [ ] **Commit:** `"refactor: all domain modules migrated to caboose-core"`

---

## Task 6: Break `app.rs` into `app/` Module Tree

**Goal:** Split the 13.6K line `app.rs` into focused modules. No logic changes — purely structural.

**Files:**
- Delete: `tui/src/app.rs`
- Create: `tui/src/app/mod.rs` — `App` struct, main event loop, `run()`
- Create: `tui/src/app/state.rs` — `State` struct (TUI view state only, core state accessed via handle)
- Create: `tui/src/app/slash_commands.rs` — all `/command` dispatch
- Create: `tui/src/app/tool_handlers.rs` — tool approval UI, execution result handling
- Create: `tui/src/app/dialog_handlers.rs` — key input for dialogs (API key, connect, MCP, etc.)
- Create: `tui/src/app/provider_mgmt.rs` — model switching, provider resolution, model picker
- Create: `tui/src/app/session_mgmt.rs` — session create/resume/fork, title management
- Create: `tui/src/app/roundhouse.rs` — roundhouse mode handling

**Approach:** This is a series of extract-method refactors. For each module:
1. Identify the methods on `App` (or blocks in the event loop) that belong to this responsibility
2. Move them to the new file as `impl App` methods
3. Verify compilation

- [ ] **Step 1:** Create `tui/src/app/` directory. Rename `app.rs` → `app/mod.rs`. Verify: compiles, tests pass.

- [ ] **Step 2:** Extract `State` struct and its initialization into `app/state.rs`. Re-export from `app/mod.rs`. Verify.

- [ ] **Step 3:** Extract slash command handlers into `app/slash_commands.rs` — all methods called from `handle_shared_slash()` and the chat-mode slash dispatch. Verify.

- [ ] **Step 4:** Extract tool execution handling into `app/tool_handlers.rs` — tool approval key handling, `handle_tool_execution()`, checkpoint snapshotting around tools. Verify.

- [ ] **Step 5:** Extract dialog handlers into `app/dialog_handlers.rs` — API key input, local provider connect, MCP input, workspace add, confirm dialogs. Verify.

- [ ] **Step 6:** Extract provider management into `app/provider_mgmt.rs` — `require_provider()`, `select_model()`, `open_model_dropdown()`, `resolve_compaction_provider()`. Verify.

- [ ] **Step 7:** Extract session management into `app/session_mgmt.rs` — `create_session()`, `restore_session()`, `fork_session()`, `update_session_meta()`, title handling. Verify.

- [ ] **Step 8:** Extract roundhouse handling into `app/roundhouse.rs` — all roundhouse-related key handlers, planner dispatch, synthesis, critique. Verify.

- [ ] **Step 9:** Final verification:
  - `cargo test --workspace` — all tests pass
  - `cargo clippy --workspace` — clean
  - No file in `app/` exceeds ~2500 lines
  - `app/mod.rs` is under 2000 lines (event loop + glue only)

- [ ] **Step 10:** Commit: `"refactor: break app.rs into app/ module tree"`

---

## Task 7: Clean Up and Verify

- [ ] **Step 1:** Run full test suite: `cargo test --workspace`
- [ ] **Step 2:** Run clippy: `cargo clippy --workspace`
- [ ] **Step 3:** Build release: `cargo build --release -p caboose`
- [ ] **Step 4:** Verify `core/` has zero imports of `ratatui`, `crossterm`, `arboard`, `portable-pty`, `vt100`, or `syntect`:
  ```bash
  grep -r "ratatui\|crossterm\|arboard\|portable.pty\|vt100\|syntect" core/src/
  ```
  Expected: no matches.

- [ ] **Step 5:** Verify `tui/src/` no longer contains domain modules (only `app/`, `tui/`, `lsp/`, `terminal/`, `clipboard.rs`, `update.rs`, `main.rs`).

- [ ] **Step 6:** Update `tui/Cargo.toml` — remove dependencies that are now only used by core (if any moved cleanly).

- [ ] **Step 7:** Commit: `"refactor: core extraction complete — verify clean separation"`

---

## Task 8: Update Documentation

- [ ] **Step 1:** Update `README.md` Development section to reflect workspace:
  ```
  cargo build              # build all (from workspace root)
  cargo test               # test all
  cargo build -p caboose   # TUI only
  cargo test -p caboose-core  # core tests only
  ```

- [ ] **Step 2:** Update `CHANGELOG.md` with core extraction entry.

- [ ] **Step 3:** Commit: `"docs: update README and CHANGELOG for workspace structure"`

- [ ] **Step 4:** Push branch, create PR.

---

## Verification Checklist (End State)

After all tasks complete:

- [ ] Workspace compiles: `cargo build --workspace`
- [ ] All tests pass: `cargo test --workspace` (1139+ tests)
- [ ] Clippy clean: `cargo clippy --workspace`
- [ ] Release builds: `cargo build --release -p caboose`
- [ ] Core has zero TUI dependencies
- [ ] `app.rs` no longer exists (replaced by `app/` module tree)
- [ ] No behavior changes — Caboose works identically to 0.6.4
- [ ] Binary size is comparable (slight increase from workspace overhead is fine)

---

## Risk Notes

- **Import path changes are the bulk of the work.** Every file that references a moved module needs `crate::X` → `caboose_core::X`. This is mechanical but error-prone. Let the compiler guide you — fix errors one at a time.
- **Cross-module dependencies within core:** When module A (already moved to core) depends on module B (still in TUI), you'll get compilation errors. The ordering in Task 5 minimizes this, but you may need to move some modules together.
- **`app.rs` breakup (Task 6) is the riskiest task.** The methods are deeply interleaved. Start with the easiest extractions (slash commands, dialogs) and leave the event loop for last.
- **Feature freeze:** Do not add features during this refactor. Every merge conflict is a tax on extraction work.
