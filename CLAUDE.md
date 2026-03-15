# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Caboose is a terminal-native AI coding agent built in Rust. Single binary, streams responses from multiple LLM providers, executes tools (file reads, edits, shell commands, web fetches), manages persistent sessions via SQLite, and supports MCP servers and an extensible skills system — all rendered in a rich TUI with syntax highlighting and markdown.

The Rust codebase lives in `tui/`. The root also contains legacy `_archive/`, `packages/`, and Node artifacts from the previous Electron incarnation — these are historical and not part of the active build.

## Build & Development Commands

```bash
cd tui
cargo build                  # debug build
cargo build --release        # optimized release build (LTO, strip)
cargo test                   # run all tests (~514)
cargo test -- --nocapture    # tests with stdout visible
cargo test <test_name>       # run a single test by name
cargo clippy                 # lint
```

Binary output: `tui/target/release/caboose` (or `caboose.exe` on Windows).

Run interactively: `caboose`
Run non-interactively: `caboose --prompt "explain this function"` (also accepts piped stdin).

## Architecture

All source is in `tui/src/`. ~25k lines of Rust, edition 2024.

### Entry Point & App Loop (`main.rs`, `app.rs`)

- `main.rs` — CLI parsing (clap), config loading, non-interactive mode, panic hook that restores terminal
- `app.rs` — `App` struct owns all state (`State`), runs the crossterm event loop, dispatches key events, manages agent lifecycle. The `State` struct is the central mutable state bag (agent, providers, tools, sessions, MCP, memory, UI state)

### Agent System (`agent/`)

Multi-turn conversation engine:
- `mod.rs` — `AgentLoop` state machine: `Idle → Streaming → ExecutingTools → PendingApproval → Idle`. Events flow from a background tokio stream task via `mpsc` channel
- `conversation.rs` — `Conversation` with `Message`/`ContentBlock` types, provider-format serialization
- `permission.rs` — Four permission modes (Plan, Default/Create, AutoEdit, Chug) with per-tool-type approval logic
- `compaction.rs` — Context window management via LLM summarization when conversation exceeds limits
- `cold_storage.rs` — Offloads old conversation segments to SQLite for ultra-long sessions
- `tools.rs` — Tool execution dispatcher, result formatting

### Provider System (`provider/`)

All providers implement the `Provider` trait (`stream`, `name`, `model`, `list_models`). Returns `StreamEvent` (TextDelta, ThinkingDelta, ToolCall, Done, Error).

- `anthropic.rs`, `openai.rs`, `gemini.rs`, `openrouter.rs` — direct HTTP + SSE streaming via reqwest
- `catalog.rs` — model capability database (context windows, tool support, pricing)
- `models_dev.rs` — OpenRouter models.dev API for dynamic model discovery
- `retry.rs` — `RetryProvider` wrapper with exponential backoff and error classification
- `error.rs` — Structured error categories (auth, rate limit, context length, server, network)
- `pricing.rs` — Per-model token cost calculation

Provider resolution: CLI flag → per-provider config → global default → "anthropic". Model resolution follows the same cascade.

### Tools (`tools/`)

`ToolRegistry` provides definitions to the LLM. Each tool is a module:
- `read.rs`, `write.rs`, `patch.rs` — file I/O (read with offset/limit, write, search-and-replace edit)
- `glob.rs`, `grep.rs` — file search (glob patterns, regex content search)
- `shell.rs` — sandboxed command execution with timeout
- `fetch.rs` — web content retrieval
- `diagnostics.rs` — LSP diagnostics tool
- `recall.rs` — memory recall tool

Additional agent-level tools (todo_write, todo_read, explore, agent, ask_user) are defined inline in `agent/tools.rs`.

### TUI Layer (`tui/`)

Ratatui + crossterm rendering:
- `layout.rs` — screen layout (header, chat area, input, footer, sidebar)
- `chat.rs` — message rendering with markdown, syntax highlighting, tool result display
- `home.rs` — home screen (logo, tips, recent sessions)
- `sidebar.rs` — MCP servers, memories, session info
- `dialog.rs` — `DialogStack` with `Screen` enum for modal overlays (model picker, session picker, etc.)
- `command.rs`, `command_palette.rs` — `/command` system and Ctrl+K palette
- `slash_auto.rs`, `file_auto.rs` — autocomplete for `/skills` and `@file` references
- `highlight.rs` — syntect-based syntax highlighting
- `theme.rs` — railroad-themed color palette
- `approval.rs` — tool approval UI for permission modes
- `input_buffer.rs`, `input_history.rs` — multi-line input with history

### Config (`config/`)

TOML-based, two layers (project `.caboose/config.toml` overrides global `~/.config/caboose/config.toml`):
- `mod.rs` — `Config` struct with all settings (provider, model, keys, tools, MCP, memory, skills, behavior)
- `auth.rs` — `AuthStore` for API key management (env vars, config file, or `/connect` command)
- `keys.rs` — API key resolution from multiple sources
- `schema.rs` — config sub-schemas (ToolsConfig, McpConfig, MemoryConfig, etc.)
- `prefs.rs` — runtime user preferences (sidebar state, etc.)

### Sessions (`session/`)

SQLite-backed conversation persistence:
- `storage.rs` — CRUD operations, message serialization
- `snapshot.rs` — session snapshots for crash recovery

### Memory (`memory/`)

Cross-session persistent knowledge:
- `store.rs` — file-based `MEMORY.md` (human-editable, always loaded) + SQLite FTS5 index
- `observations.rs` — captures facts during conversation
- `extraction.rs` — LLM-guided end-of-session fact extraction
- `search.rs` — FTS5 search across stored memories

### MCP (`mcp/`)

Model Context Protocol client — extends tools via external servers:
- `manager.rs` — `McpManager` manages server lifecycle (connect, disconnect, status, tool calls). Uses `rmcp` crate with stdio transport. Connections and tool calls are fully async (spawned on background tokio tasks via mpsc/oneshot channels) so the event loop never blocks. `McpPreparedCall` clones the rmcp `Peer` handle for thread-safe background execution with 30s timeout.
- `presets.rs` — built-in server presets (context7, fetch). Presets start disabled and can be toggled via Tab in the `/mcp` dropdown. Config persists `disabled`/`removed` fields to `.caboose/config.toml`.

### Skills (`skills/`)

Slash command system with built-in + user-defined skills:
- `builtins.rs` — 11 built-in skills (brainstorm, plan, debug, tdd, review, etc.)
- `loader.rs` — loads user skills from `.caboose/skills/` and `~/.config/caboose/skills/`
- `resolver.rs` — 3-tier resolution: built-in → user → error
- `creation.rs` — LLM-guided `/create-skill` flow
- `handoff.rs` — `/handoff` session summary generation
- `awareness.rs` — skill context injection into system prompts
- `hints.rs` — contextual skill suggestions

### Safety (`safety/`)

- `command_policy.rs` — allow/deny command lists with shell-segment analysis
- `env_filter.rs` — strips secret env vars before command execution

### Other Modules

- `lsp/` — LSP client for language server integration (diagnostics, detection)
- `terminal/` — embedded PTY terminal panel (portable-pty + vt100)
- `init/` — `/init` command to generate `CABOOSE.md` project files
- `clipboard.rs` — clipboard access via arboard

## Gotchas

- **Windows key events** — crossterm on Windows emits both Press and Release events; all key handlers filter on `KeyEventKind::Press` to avoid double-processing
- **The `packages/` and `_archive/` directories are inert** — the Electron app is no longer maintained. All active code is under `tui/`
- **CABOOSE.md vs CLAUDE.md** — `CABOOSE.md` is the project-context file that Caboose (the product) loads into prompts via `/init`. `CLAUDE.md` (this file) is for Claude Code
- **Config is TOML** — not JSON. Global at `~/.config/caboose/config.toml`, project at `.caboose/config.toml`
- **Default provider is Anthropic** with fallback model `claude-sonnet-4-6` if nothing else is configured
- **All providers stream via raw HTTP/SSE** — no SDK dependencies. Each provider module does its own request building and SSE parsing
- **RetryProvider wraps all providers** — automatic retries with exponential backoff and error classification
- **Tests are all unit tests** — no integration tests requiring API keys or external services. ~514 tests, all run in ~2 seconds

## Commit Style

- Short, lowercase, descriptive messages (no conventional commits prefix)
- Never include co-authored-by lines
- Examples: `fix update checker artifacts and r2 workflow triggers`, `roundhouse streaming in main window, looping typewriter sidebar, readme logo`
