# app.rs Module Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the 13,607-line `tui/src/app.rs` into 15 files under `tui/src/app/` with zero behavior changes.

**Architecture:** Rename `app.rs` → `app/mod.rs`, then extract methods and types into submodule files one at a time. Each submodule contains `impl App` blocks (or standalone items) and uses `use super::*` to access shared types. `mod.rs` re-exports all public types so external consumers are unaffected.

**Tech Stack:** Rust (edition 2024), Cargo workspace

**Spec:** `docs/superpowers/specs/2026-03-20-app-module-split-design.md`

---

## Pre-Flight

**External dependents** — 23 files import from `crate::app::`. These types/functions must be re-exported from `app/mod.rs`:
- Types: `ChatMessage`, `State`, `ToolMessage`, `ToolStatus`, `TaskOutline`, `TaskStatus`, `FileStats`, `Task`
- Functions: `needs_new_session_confirm()`, `roundhouse_active()`

**Build/test commands:**
- Build: `cd tui && cargo build --workspace`
- Test: `cd tui && cargo test --workspace`
- Clippy: `cd tui && cargo clippy --workspace`

---

### Task 1: Scaffold — Convert `app.rs` to `app/mod.rs`

**Files:**
- Rename: `tui/src/app.rs` → `tui/src/app/mod.rs`

- [ ] **Step 1: Create the app/ directory and move the file**

```bash
mkdir -p tui/src/app
git mv tui/src/app.rs tui/src/app/mod.rs
```

- [ ] **Step 2: Build to verify the rename is transparent**

Run: `cd tui && cargo build --workspace`
Expected: Compiles cleanly — Rust treats `app/mod.rs` identically to `app.rs`.

- [ ] **Step 3: Run tests**

Run: `cd tui && cargo test --workspace`
Expected: All tests pass (1501).

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/mod.rs
git commit -m "refactor: convert app.rs to app/mod.rs"
```

---

### Task 2: Extract `types.rs`

**Files:**
- Create: `tui/src/app/types.rs`
- Modify: `tui/src/app/mod.rs`

Move these items from `mod.rs` to `types.rs`:
- `TextSelection` struct (lines 19–25)
- `SpawnAgentHandle` struct (lines 27–34)
- `ToolStatus` enum (lines 304–311)
- `TaskStatus` enum (lines 313–320)
- `Task` struct (lines 322–328)
- `TaskOutline` struct (lines 330–335) + `impl TaskOutline` (lines 336–398)
- `ToolMessage` struct (lines 400–414)
- `ChatMessage` enum (lines 416–459)
- `FileStats` struct (lines 461–467)
- `task_outline_tests` test module

- [ ] **Step 1: Create `types.rs` with all type definitions**

Cut the type definitions listed above from `mod.rs` and paste into `types.rs`. At the top of `types.rs`, add any imports these types need (check their field types — e.g., `tokio::task::JoinHandle`, `serde_json::Value`, `std::time::Instant`).

- [ ] **Step 2: Add module declaration and re-exports to `mod.rs`**

At the top of `mod.rs`, add:

```rust
mod types;
pub use types::*;
```

Remove the type definitions that were moved (they now live in `types.rs`).

- [ ] **Step 3: Build**

Run: `cd tui && cargo build --workspace`
Expected: Compiles cleanly.

- [ ] **Step 4: Run tests**

Run: `cd tui && cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add tui/src/app/types.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/types.rs"
```

---

### Task 3: Extract `helpers.rs`

**Files:**
- Create: `tui/src/app/helpers.rs`
- Modify: `tui/src/app/mod.rs`

Move these standalone functions (not `impl App` methods) from `mod.rs` to `helpers.rs`:
- `slice_chars` (line 36)
- `roundhouse_active` (line 547)
- `needs_new_session_confirm` (line 563)
- `run_spawn_agent_task` (line 12590)
- `task_likely_requires_changes` (line 12771)
- `build_noop_retry_task` (line 12787)
- `parse_interval` (line 12794)
- `format_duration` (line 12808)
- `parse_circuit_args` (line 12821)
- `parse_tasks_from_text` (line 12841)
- `scan_roots` (line 12919)
- `spawn_dir_scan` (line 12942)
- `walk_dirs_fuzzy` (line 12960)
- `is_ignored_dir` (line 13035)
- `build_workspace_list_state` (line 13070)
- `workspace_system_prompt_block` (line 13091)
- `has_meaningful_model_switch_context` (line 13152)
- Test modules: `task_text_parse_tests`, `model_switch_handoff_tests`, `circuit_parse_tests`, `workspace_list_handler_tests`, `workspace_prompt_tests`, `new_session_confirm_tests`
- Also keep `execute_command_tests` here (it tests `sub_agent::pipeline::extract_tasks` but lives in app.rs)

- [ ] **Step 1: Create `helpers.rs` with all standalone functions and their tests**

Cut all standalone functions and their associated test modules from `mod.rs` into `helpers.rs`. Add necessary imports at the top. For `pub(crate)` functions, keep that visibility.

- [ ] **Step 2: Add module declaration and re-exports to `mod.rs`**

```rust
mod helpers;
pub(crate) use helpers::{roundhouse_active, needs_new_session_confirm};
// Also re-export any functions whose test modules use `super::function_name` —
// since tests in helpers.rs see super = mod.rs, the function must be visible there.
// This includes: workspace_system_prompt_block, build_workspace_list_state,
// needs_new_session_confirm, has_meaningful_model_switch_context, and any others
// whose tests call super::<name>.
pub(crate) use helpers::*;
```

Use `pub(crate) use helpers::*` to re-export all helper functions. This is needed because test modules inside `helpers.rs` use `super::function_name`, and `super` resolves to `mod.rs`, not `helpers.rs`. The re-export makes the functions visible at the `mod.rs` level.

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`
Expected: All pass.

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/helpers.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/helpers.rs"
```

---

### Task 4: Fix App Field Visibility + Extract `state.rs`

**Files:**
- Create: `tui/src/app/state.rs`
- Modify: `tui/src/app/mod.rs`

**IMPORTANT — Field visibility prerequisite:** Before any submodule can access `App` struct fields, private fields must be marked `pub(super)`. The `App` struct has `state` and `terminal` as `pub`, but other fields (like `provider`) are private. All subsequent tasks (5–15) will fail to compile if submodule code accesses private fields.

Move from `mod.rs` to `state.rs`:
- `State` struct definition (lines 43–301)
- `impl State` block (lines 469–536): `update_slash_auto`, `update_file_auto`
- `impl App::new` method (lines 572–1066) — this is a single method from the `impl App` block

- [ ] **Step 0: Mark App struct fields `pub(super)`**

In `mod.rs`, change all private fields on the `App` struct to `pub(super)`:

```rust
pub struct App {
    pub(super) state: State,
    pub(super) terminal: Terminal,
    // ... mark ALL fields pub(super) ...
}
```

This must happen before extracting any `impl App` methods into submodules.

- [ ] **Step 1: Create `state.rs`**

At the top: `use super::*;`

Paste the `State` struct, `impl State` block, and create a new `impl App` block containing only the `new` method:

```rust
use super::*;

pub struct State {
    // ... all fields ...
}

impl State {
    pub fn update_slash_auto(&mut self, ...) { ... }
    pub fn update_file_auto(&mut self, ...) { ... }
}

impl App {
    pub async fn new(...) -> Result<Self> { ... }
}
```

- [ ] **Step 2: Update `mod.rs`**

Add `mod state; pub use state::State;` and remove the moved items from `mod.rs`. The remaining `impl App` block in `mod.rs` loses the `new` method but keeps everything else.

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/state.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/state.rs"
```

---

### Task 5: Extract `input.rs`

**Files:**
- Create: `tui/src/app/input.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_paste`
- `record_text_input_activity`
- `reset_text_input_activity`
- `should_treat_enter_as_paste_newline`
- `should_insert_text`

- [ ] **Step 1: Create `input.rs`**

```rust
use super::*;

impl App {
    // paste all 5 methods here
}
```

- [ ] **Step 2: Update `mod.rs` — add `mod input;`, remove the 5 methods from `impl App`**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/input.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/input.rs"
```

---

### Task 6: Extract `skills.rs`

**Files:**
- Create: `tui/src/app/skills.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_skill_creation_key`
- `start_skill_creation`
- `save_created_skill`
- `toggle_skill_disabled`
- `delete_user_skill`
- `handle_create_skill_command`

- [ ] **Step 1: Create `skills.rs` with `use super::*;` and an `impl App` block containing all 6 methods**

- [ ] **Step 2: Update `mod.rs` — add `mod skills;`, remove the 6 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/skills.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/skills.rs"
```

---

### Task 7: Extract `session_mgmt.rs`

**Files:**
- Create: `tui/src/app/session_mgmt.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `restore_session`
- `persist_message`
- `update_session_meta`
- `sync_pins_to_system_prompt`
- `recompute_modified_files`
- `execute_new_session`
- `extract_session_memories`

- [ ] **Step 1: Create `session_mgmt.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod session_mgmt;`, remove the 7 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/session_mgmt.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/session_mgmt.rs"
```

---

### Task 8: Extract `roundhouse.rs`

**Files:**
- Create: `tui/src/app/roundhouse.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods (note: `handle_roundhouse_key` stays in `key_dispatch.rs` with the other screen-level dispatchers — see Task 15):
- `start_roundhouse_planning`
- `start_roundhouse_critique`
- `start_roundhouse_synthesis`
- `handle_roundhouse_picker_key`
- `handle_roundhouse_subcommand`
- `clear_roundhouse_session`
- `extract_code_blocks`
- `copy_hovered_code_block`
- `copy_hovered_message`

- [ ] **Step 1: Create `roundhouse.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod roundhouse;`, remove the 9 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/roundhouse.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/roundhouse.rs"
```

---

### Task 9: Extract `provider_mgmt.rs`

**Files:**
- Create: `tui/src/app/provider_mgmt.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `select_model`
- `open_model_dropdown`
- `connect_provider`
- `require_provider`
- `resolve_compaction_provider`
- `connect_mcp_servers`
- `open_mcp_picker`
- `refresh_mcp_dropdown`
- `handle_mcp_command`
- `handle_mcp_tab`
- `build_tool_defs`
- `images_config`

- [ ] **Step 1: Create `provider_mgmt.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod provider_mgmt;`, remove the 12 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/provider_mgmt.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/provider_mgmt.rs"
```

---

### Task 10: Extract `slash_commands.rs`

**Files:**
- Create: `tui/src/app/slash_commands.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_shared_slash`
- `handle_workspace_command`
- `handle_init_command`
- `finalize_init`
- `handle_forget_command`
- `handle_pin_command`
- `handle_pins_command`
- `handle_unpin_command`
- `handle_circuit_command`
- `create_circuit`
- `handle_watch_command`
- `create_watcher`
- `handle_memories_command`
- `handle_suggest_command`
Note: `handle_mcp_command` is in `provider_mgmt.rs` (Task 9), not here.

- [ ] **Step 1: Create `slash_commands.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod slash_commands;`, remove methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/slash_commands.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/slash_commands.rs"
```

---

### Task 11: Extract `handoff.rs`

**Files:**
- Create: `tui/src/app/handoff.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_handoff_command`
- `spawn_handoff_agent`
- `build_model_switch_handoff_context`
- `handle_fork_command`

- [ ] **Step 1: Create `handoff.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod handoff;`, remove the 4 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/handoff.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/handoff.rs"
```

---

### Task 12: Extract `tool_handlers.rs`

**Files:**
- Create: `tui/src/app/tool_handlers.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `start_tool_execution`
- `spawn_next_tool`
- `spawn_agent_setup`
- `poll_tool_execution`
- `compute_pending_diff`
- `handle_tool_result`
- `finalize_tool_execution`
- `poll_mcp_connections`
- `poll_spawn_agent_handles`
- `merge_reviewed_agents`
- `collect_review_agent_changes`
- `merge_single_agent`
- `poll_circuit_events`
- `cancel_all_operations`
- `handle_ask_user_calls`
- `render_current_ask_user_question`
- `finalize_ask_user`
- `handle_ask_user_key`
- `dismiss_ask_user`
- `handle_todo_calls`
- `handle_generate_skill_calls`

- [ ] **Step 1: Create `tool_handlers.rs` with `use super::*;` and `impl App` block containing all 21 methods**

This is the largest extraction. Take care to grab all methods and their associated imports.

- [ ] **Step 2: Update `mod.rs` — add `mod tool_handlers;`, remove the 21 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/tool_handlers.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/tool_handlers.rs"
```

---

### Task 13: Extract `pickers.rs`

**Files:**
- Create: `tui/src/app/pickers.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_picker_key`
- `handle_picker_select`
- `handle_session_picker_confirm`
- `refresh_session_search`
- `picker_item_count`

- [ ] **Step 1: Create `pickers.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod pickers;`, remove the 5 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/pickers.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/pickers.rs"
```

---

### Task 14: Extract `dialogs.rs`

**Files:**
- Create: `tui/src/app/dialogs.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_file_browser_key`
- `handle_agents_list_key`
- `handle_circuits_list_key`
- `handle_migration_checklist_key`
- `handle_key_input_key`
- `handle_local_connect_key`
- `handle_mcp_input_key`
- `handle_mcp_input_submit`
- `handle_agent_stream_overlay_key`
- `handle_workspace_list_key`
- `handle_workspace_add_key`
- `handle_workspace_add_confirm`
- `refresh_workspace_list_state`
- `handle_command_palette_key`
- `open_settings_picker`
- `open_rewind_picker`
- Test module: `workspace_add_validation_tests`

- [ ] **Step 1: Create `dialogs.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod dialogs;`, remove the 16 methods and test module**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/dialogs.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/dialogs.rs"
```

---

### Task 15: Extract `key_dispatch.rs`

**Files:**
- Create: `tui/src/app/key_dispatch.rs`
- Modify: `tui/src/app/mod.rs`

Move these `impl App` methods:
- `handle_home_key`
- `handle_chat_key`
- `handle_approval_key`
- `handle_roundhouse_key` (screen-level dispatcher, same tier as the above three)
- `pending_tool_rejection_msg`
- `replace_pending_with_rejection`

- [ ] **Step 1: Create `key_dispatch.rs` with `use super::*;` and `impl App` block**

- [ ] **Step 2: Update `mod.rs` — add `mod key_dispatch;`, remove the 6 methods**

- [ ] **Step 3: Build and test**

Run: `cd tui && cargo build --workspace && cargo test --workspace`

- [ ] **Step 4: Commit**

```bash
git add tui/src/app/key_dispatch.rs tui/src/app/mod.rs
git commit -m "refactor: extract app/key_dispatch.rs"
```

---

### Task 16: Final Verification

**Files:**
- Possibly modify: `tui/src/app/mod.rs` (any remaining fixes)

- [ ] **Step 1: Run full build + clippy**

Run: `cd tui && cargo build --workspace && cargo clippy --workspace`

- [ ] **Step 3: Run full test suite**

Run: `cd tui && cargo test --workspace`
Expected: All 1501 tests pass.

- [ ] **Step 4: Verify mod.rs is the right size**

Run: `wc -l tui/src/app/mod.rs`
Expected: ~1700 lines (run, handle_key, handle_turn_complete, handle_menu_scroll, process_action, lifecycle methods).

- [ ] **Step 5: Verify all submodule files exist**

```bash
ls -la tui/src/app/
```

Expected 15 files: `mod.rs`, `types.rs`, `state.rs`, `helpers.rs`, `input.rs`, `skills.rs`, `session_mgmt.rs`, `roundhouse.rs`, `provider_mgmt.rs`, `slash_commands.rs`, `handoff.rs`, `tool_handlers.rs`, `pickers.rs`, `dialogs.rs`, `key_dispatch.rs`

- [ ] **Step 5: Commit any remaining fixes**

```bash
git add tui/src/app/
git commit -m "refactor: finalize app module split"
```

---

## Ordering Rationale

Tasks are ordered from least-coupled to most-coupled:

1. **Scaffold** — mechanical rename, zero risk
2. **Types** — leaf nodes, no method dependencies
3. **Helpers** — standalone functions, no `self`
4. **State** — struct + constructor, minimal coupling
5–8. **Small modules** (input, skills, session, roundhouse) — few methods, isolated concerns
9–11. **Medium modules** (provider, slash commands, handoff) — some cross-module calls but manageable
12. **Tool handlers** — largest extraction, most internal cross-calls
13–14. **Pickers, dialogs** — UI handlers that call into many other modules
15. **Key dispatch** — top-level routers that call everything else; extracted last because they reference methods across all modules
16. **Cleanup** — field visibility, final verification

## Notes for Implementers

- **`use super::*`** at the top of every submodule file gives access to everything in `mod.rs` scope
- **Imports:** When moving methods, also move any `use` statements that are only needed by those methods. If an import is used by methods in multiple modules, keep it in `mod.rs` (it'll be available via `super::*`)
- **`pub(crate)` visibility:** Keep existing visibility on all methods. Don't change `fn` to `pub fn` or vice versa
- **Cross-module method calls:** `self.method()` works across submodules because they're all `impl App` blocks in the same module tree. No special imports needed.
- **Don't delete old code** — this is an additive refactor. The original `app.rs` becomes `app/mod.rs` and shrinks as we extract. Nothing is lost.
