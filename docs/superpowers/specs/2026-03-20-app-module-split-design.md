# app.rs Module Split Design

## Goal

Break the 13,607-line `tui/src/app.rs` monolith into an `app/` module tree. No behavior changes — pure structural refactor.

## Current State

Single file containing:
- ~470 lines of type definitions (ChatMessage, ToolMessage, TaskOutline, etc.)
- State struct with ~60 fields + 2 methods
- `impl App` with ~110 methods spanning initialization, event loop, key handling, tool execution, slash commands, dialogs, provider management, sessions, roundhouse, skills
- 17 standalone helper functions
- 9 test modules (~40 tests total)

## Rust Module Mechanics

All submodules are `impl App` blocks in separate files under `mod app`. Each file does `use super::*` and calls methods on `self` — no circular dependency risk since Rust sees all `impl` blocks within the same module as one unit.

**Field visibility:** `State` fields are already `pub`. The `App` struct's fields (currently private) will need `pub(super)` so submodule files can access `self.state`, `self.terminal`, etc.

## Module Tree

```
tui/src/app/
├── mod.rs            (~1700 lines)  Event loop core + top-level routing
├── types.rs          (~470 lines)   Shared data types
├── state.rs          (~600 lines)   State struct + App::new
├── key_dispatch.rs   (~1650 lines)  Screen-level key routing
├── slash_commands.rs (~750 lines)   /command handlers
├── handoff.rs        (~1880 lines)  /handoff, /fork, model switch context
├── tool_handlers.rs  (~2400 lines)  Tool execution pipeline
├── pickers.rs        (~1540 lines)  Session/model/workspace pickers
├── dialogs.rs        (~740 lines)   Input dialogs, file browser, MCP
├── provider_mgmt.rs  (~1000 lines)  Model switching, MCP connections
├── session_mgmt.rs   (~340 lines)   Session persistence
├── roundhouse.rs     (~650 lines)   Multi-model planning
├── skills.rs         (~330 lines)   Skill CRUD
├── input.rs          (~235 lines)   Paste detection, text input
└── helpers.rs        (~600 lines)   Standalone utility functions
```

## Module Responsibilities

### `mod.rs` — Event Loop Core (~1700 lines)
- `App` struct definition (fields marked `pub(super)`)
- `run` method (~1300 lines) — main crossterm event loop (poll, draw, route events)
- `handle_key` (~280 lines) — top-level key router (terminal focus, Ctrl+H, command dispatch, fans out to screen handlers)
- `process_action` (~12 lines) — dispatch `Action` enum from command registry
- `handle_turn_complete` (~420 lines) — post-LLM-turn processing (tokens, cost, compaction, circuit events, title generation, MCP polling)
- `handle_menu_scroll` (~80 lines) — scroll event routing across palettes/pickers
- Core lifecycle: `request_quit`, `clear_composer_input`, `extract_selected_text`, `flush_assistant_text`, `spawn_title_generation`, `check_budget_exceeded`
- `pub mod` declarations and re-exports for public API compatibility

### `types.rs` — Shared Data Types (~470 lines)
- `ChatMessage` enum (User, Assistant, Tool, System, Error, ProviderError, TaskOutline, Skill, Queued, AskUser)
- `ToolMessage` struct
- `TaskOutline` struct + `impl TaskOutline` (from_tool_input, to_json)
- `Task` struct
- `ToolStatus` enum
- `TaskStatus` enum
- `FileStats` struct
- `TextSelection` struct
- `SpawnAgentHandle` struct

Rationale: These are leaf-node data contracts consumed by rendering, persistence, and app logic. Separating them from State keeps imports clean — any module can use `ChatMessage` without pulling in State initialization.

### `state.rs` — State Struct + Initialization (~600 lines)
- `State` struct definition (~60 fields)
- `impl State`: `update_slash_auto`, `update_file_auto`
- `impl App`: `new` (495 lines — full app initialization: config, providers, agents, skills, sessions, MCP)

Rationale: `new` constructs State, so it belongs with the struct definition. This is the "setup" module.

### `key_dispatch.rs` — Screen-Level Key Routing (~1650 lines)
- `handle_home_key` (566 lines) — home screen navigation (session list, recent items, tips)
- `handle_chat_key` (972 lines) — main chat key dispatch (input, scrolling, tool focus, sidebar, autocomplete)
- `handle_approval_key` (29 lines) — tool approval y/n/a
- `pending_tool_rejection_msg` (12 lines) — build rejection message
- `replace_pending_with_rejection` (12 lines) — replace pending tool with rejection

These are the second level of the event routing hierarchy: `handle_key` → `handle_*_key` → specific handlers in other modules. They stay together because they share the same role (screen-level dispatch).

### `slash_commands.rs` — Command Handlers (~750 lines)
- `handle_shared_slash` — generic slash command dispatcher
- `handle_workspace_command` — `/workspace` parsing
- `handle_init_command` + `finalize_init` — `/init` for CABOOSE.md
- `handle_forget_command` — `/forget` memory
- `handle_pin_command`, `handle_pins_command`, `handle_unpin_command`
- `handle_circuit_command` + `create_circuit` — `/circuit` recurring tasks
- `handle_watch_command` + `create_watcher` — `/watch pr`
- `handle_memories_command` — `/memories`
- `handle_suggest_command` — `/suggest`
- `handle_mcp_command` — `/mcp list|restart|connect|disconnect`

### `handoff.rs` — Session Handoff & Fork (~1880 lines)
- `handle_handoff_command` (376 lines) — `/handoff <model>` to hand off to subagent
- `spawn_handoff_agent` — build and launch the handoff subagent
- `build_model_switch_handoff_context` (1048 lines) — build context for model switch
- `handle_fork_command` (454 lines) — `/fork` for session forking

Rationale: Handoff context building is a single 1048-line function — it's a self-contained feature with its own data flow. Fork is closely related (both create new sessions from existing context).

### `tool_handlers.rs` — Tool Execution Pipeline (~2400 lines)
- `start_tool_execution` (777 lines) — batch execute pending tool calls with approval
- `spawn_next_tool` (~270 lines) — dequeue and dispatch next tool from execution queue
- `spawn_agent_setup` (~250 lines) — create worktrees, register subagents for spawn_agent calls
- `poll_tool_execution` (~65 lines) — poll oneshot receiver for background tool results
- `compute_pending_diff` — compute diff previews for pending tool calls
- `handle_tool_result` (65 lines) — process tool result, inject into chat
- `finalize_tool_execution` (131 lines) — wrap up, check completion
- `poll_mcp_connections` (163 lines) — poll background MCP results
- `poll_spawn_agent_handles` — poll completed spawn_agent background tasks
- `merge_reviewed_agents` + `collect_review_agent_changes` + `merge_single_agent` (~250 lines) — reviewed subagent merge pipeline
- `poll_circuit_events` (~140 lines) — drive circuit LLM execution
- `cancel_all_operations` (403 lines) — cancel agent, sub-agents, approvals
- `handle_ask_user_calls` + `render_current_ask_user_question` + `finalize_ask_user` + `handle_ask_user_key` + `dismiss_ask_user`
- `handle_todo_calls` (145 lines) — process todo_write
- `handle_generate_skill_calls` (92 lines) — inject generated skills

Left as one module because tool execution is one cohesive pipeline — splitting would scatter tightly coupled methods.

### `pickers.rs` — Picker UI Logic (~1540 lines)
- `handle_picker_key` (227 lines) — picker navigation (up/down/enter/escape)
- `handle_picker_select` (490 lines) — execute picker selection (sessions, models, skills)
- `handle_session_picker_confirm` (63 lines) — delete session confirmation
- `refresh_session_search` (661 lines) — rebuild session list from filter/search
- `picker_item_count` (97 lines) — count items in current picker

### `dialogs.rs` — Dialog Handlers (~740 lines)
- `handle_file_browser_key` (123 lines) — file browser navigation
- `handle_agents_list_key` (36 lines) — sidebar agents list
- `handle_circuits_list_key` (50 lines) — circuits dropdown
- `handle_migration_checklist_key` (69 lines) — migration modal
- `handle_key_input_key` (68 lines) — text input dialog (MCP input, password)
- `handle_local_connect_key` (216 lines) — local LLM connection wizard
- `handle_mcp_input_key` (32 lines) + `handle_mcp_input_submit` (63 lines) — MCP tool input
- `handle_agent_stream_overlay_key` (66 lines) — subagent stream overlay
- `handle_workspace_list_key` (88 lines) + `handle_workspace_add_key` (132 lines) + `handle_workspace_add_confirm` (197 lines)
- `refresh_workspace_list_state` — rebuild workspace list after add/remove
- `handle_command_palette_key` (63 lines) — /command palette search
- `open_settings_picker` (103 lines)
- `open_rewind_picker` (34 lines)

### `provider_mgmt.rs` — Provider & Model Management (~1000 lines)
- `select_model` (548 lines) — switch active model/provider, update capabilities
- `open_model_dropdown` (~200 lines) — fetch model lists, populate picker, handle OpenRouter pricing
- `connect_provider` — connect to a named provider, update capabilities, persist prefs
- `require_provider` (23 lines) — ensure provider is available
- `resolve_compaction_provider` (29 lines) — setup compaction model
- `connect_mcp_servers` (21 lines) — connect configured MCP servers
- `open_mcp_picker` (9 lines) + `refresh_mcp_dropdown` (111 lines)
- `handle_mcp_command` — `/mcp list|restart|connect|disconnect`
- `handle_mcp_tab` — MCP picker tab interaction
- `build_tool_defs` (15 lines) — build tool definitions for LLM
- `images_config` (5 lines)

### `session_mgmt.rs` — Session Persistence (~340 lines)
- `restore_session` (150 lines) — load session from DB into chat
- `persist_message` (31 lines) — create session on first message, save to DB
- `update_session_meta` (25 lines) — update title, turn count
- `sync_pins_to_system_prompt` (28 lines) — inject pins into system prompt
- `recompute_modified_files` (26 lines) — scan file changes in session
- `execute_new_session` (~50 lines) — reset all session state for `/new`
- `extract_session_memories` — run end-of-session memory extraction

### `roundhouse.rs` — Multi-Model Planning (~650 lines)
- `handle_roundhouse_key` (109 lines) — key routing in roundhouse mode
- `start_roundhouse_planning` (99 lines) + `start_roundhouse_critique` (143 lines) + `start_roundhouse_synthesis` (87 lines)
- `handle_roundhouse_picker_key` (92 lines) — model selection
- `handle_roundhouse_subcommand` (47 lines) — `/roundhouse <sub>`
- `clear_roundhouse_session` (9 lines)
- `extract_code_blocks` (24 lines) + `copy_hovered_code_block` (28 lines) + `copy_hovered_message` (22 lines)

### `skills.rs` — Skill CRUD (~330 lines)
- `handle_skill_creation_key` (55 lines) — skill creation UI
- `start_skill_creation` (37 lines) — initiate skill creation session
- `save_created_skill` (50 lines) — persist to `.caboose/skills/`
- `toggle_skill_disabled` (43 lines) — enable/disable
- `delete_user_skill` (67 lines) — delete skill file
- `handle_create_skill_command` (78 lines) — `/create-skill`

### `input.rs` — Text Input Handling (~235 lines)
- `handle_paste` (75 lines) — paste event with newline/burst detection
- `record_text_input_activity` (22 lines) — track rapid input for paste detection
- `reset_text_input_activity` (6 lines) — clear paste state
- `should_treat_enter_as_paste_newline` (129 lines) — heuristic for multiline paste vs deliberate enter
- `should_insert_text` (3 lines) — check if modifiers allow text insertion

### `helpers.rs` — Standalone Utility Functions (~600 lines)
- `slice_chars` — substring by char count
- `roundhouse_active` — check if roundhouse in active phase
- `needs_new_session_confirm` — check if session has meaningful content
- `run_spawn_agent_task` — background subagent executor (180 lines)
- `task_likely_requires_changes`, `build_noop_retry_task` — agent task heuristics
- `parse_interval`, `format_duration`, `parse_circuit_args` — circuit parsing
- `parse_tasks_from_text` — extract task list from text (77 lines)
- `scan_roots`, `spawn_dir_scan`, `walk_dirs_fuzzy`, `is_ignored_dir` — directory scanning
- `build_workspace_list_state`, `workspace_system_prompt_block` — workspace utilities
- `has_meaningful_model_switch_context` — model switch heuristic

## Test Migration

Tests move with their associated functions:
- `task_text_parse_tests` → `helpers.rs`
- `model_switch_handoff_tests` → `helpers.rs`
- `task_outline_tests` → `types.rs`
- `circuit_args_tests` → `helpers.rs`
- `workspace_list_tests` → `helpers.rs`
- `workspace_system_prompt_tests` → `helpers.rs`
- `new_session_confirm_tests` → `helpers.rs`
- `workspace_add_validation_tests` → `dialogs.rs`
- `execute_command_tests` → lives in `app.rs` but tests `sub_agent::pipeline::extract_tasks`; move to `sub_agent/pipeline.rs` in core or keep in `helpers.rs`

## Implementation Strategy

1. Create `app/` directory with `mod.rs`
2. Mark `App` struct fields `pub(super)` so submodule files can access them
3. Move types first (no `impl App` dependencies)
4. Move standalone helpers (no `self` references)
5. Move `impl App` methods module by module, keeping `pub(crate)` visibility
6. Each submodule file: `use super::*;` at the top, methods call `self.method()` normally
7. Each module step: move code → compile → run tests
8. `mod.rs` re-exports everything needed by external consumers (`pub use types::*`, etc.)
9. Final pass: verify all `use` paths, run full test suite

## Non-Goals

- No behavior changes
- No renaming functions
- No refactoring function internals
- No changing public API surface — external consumers see the same types and methods
