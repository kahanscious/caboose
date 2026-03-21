# Development Status — 2026-03-20

## What Shipped Today

### 0.7.0 (branch: `feat/0.7.0`, PR #14 — pending merge)
- **Core extraction** — split monolithic `tui/` into Cargo workspace: `core/` (`caboose-core`, platform-agnostic domain logic) and `tui/` (terminal frontend). All domain modules live in core.
- **`app.rs` module split** — broke 13.6K line monolith into 15 focused modules under `app/`: mod.rs (2400), tool_handlers.rs (2060), key_dispatch.rs (1709), dialogs.rs (1343), helpers.rs (971), state.rs (830), slash_commands.rs (777), pickers.rs (762), provider_mgmt.rs (660), roundhouse.rs (557), handoff.rs (469), session_mgmt.rs (407), skills.rs (329), types.rs (273), input.rs (131).
- **Stale code cleanup** — deleted 30+ duplicate files from TUI that were copies of core. TUI modules now re-export from `caboose-core`.
- **Dependency audit** — removed 12 duplicate deps from `tui/Cargo.toml`, 7 phantom deps from `core/Cargo.toml`.
- **Core isolation verified** — zero ratatui/crossterm references in `core/src/`.
- **Test deduplication** — 811 core + 403 TUI = 1214 total (down from 1501 duplicated).
- **Paste detection fix** — tightened rapid-input gap from 180ms to 50ms so fast typing no longer triggers newline-instead-of-send.

### 0.6.4 (released, tagged `v0.6.4`, merged to main)
- 8 new LLM providers (AI21, Moonshot, Yi, Zhipu, Novita, Inflection, HuggingFace, Reka) — 23 total
- Cost tracking: cache-aware pricing, session cost persistence to SQLite, `/cost` command
- Checkpoint polish: `/checkpoint <name>` for named bookmarks, diff preview on rewind
- Compaction model override: `compaction_model` config wired
- Dynamic pricing: user config overrides (`[pricing]`), OpenRouter cross-provider model ID mapping
- Collapsible `/connect` picker: Popular / Engines / Local sections
- README + CHANGELOG updated

## Architecture (post-0.7.0)

```
caboose/
├── core/           caboose-core (platform-agnostic domain logic)
│   └── src/
│       ├── agent/          state machine, conversation, compaction, permission
│       ├── agents/         custom agent definitions
│       ├── config/         schema, keys, auth, loading
│       ├── provider/       trait, registry, pricing, all implementations
│       ├── tools/          definitions, registry, execution
│       ├── session/        types, SQLite persistence
│       ├── memory/         observations, search, extraction
│       ├── mcp/            MCP client, presets
│       ├── skills/         loading, resolution, builtins, creation
│       ├── roundhouse/     multi-LLM planning engine
│       ├── sub_agent/      conflict, pipeline, worktree
│       ├── safety/         command policy, env filter
│       ├── scm/            source control detection, tools, watcher
│       ├── circuits/       scheduled tasks
│       ├── hooks/          lifecycle shell hooks
│       ├── init/           CABOOSE.md generation
│       ├── migrate/        settings import
│       ├── suggest/        codebase scanning, parsers
│       ├── attachment.rs   image compression
│       └── checkpoint.rs   file snapshots, rewind
└── tui/            caboose (terminal frontend)
    └── src/
        ├── app/            15 modules (event loop, state, key dispatch, etc.)
        ├── tui/            ratatui rendering, widgets, layout
        ├── lsp/            language server client
        ├── terminal/       embedded PTY panel
        ├── hooks/          PostToolHook trait, LSP diagnostics hook
        ├── tools/          LSP-dependent tools (diagnostics, lsp)
        ├── sub_agent/      executor (depends on TUI hooks/LSP)
        ├── session/        export (depends on ChatMessage)
        ├── migrate/        converter apply_migration (depends on TUI dialogs)
        ├── clipboard.rs    platform clipboard
        ├── update.rs       binary update checker
        └── prefs.rs        theme preferences
```

## What's Next

### 0.7.x — `caboose-server` (WebSocket API)
- New crate in workspace: `server/`
- Axum + tokio-tungstenite WebSocket server wrapping `caboose-core`
- QR code pairing auth for mobile connection
- Exposes: agent conversations, tool execution, session management, model switching
- Prerequisite for mobile app

### 0.7.x — Background Agents
- `/bg <prompt>` — spawn background agents with auto-approve
- Per-agent and global budget enforcement
- Can land before or after the server

### 0.7.x — Tender Highball Integration
- Wire `web_search` tool to self-hosted SearXNG backend
- Configure endpoint URL in `ServicesConfig`
- Needs other computer for deployment

### 0.8.0 — Mobile App
- Flutter (Android native + Web PWA for iOS)
- Repo: `kahanscious/caboose-mobile` (private, empty)
- Two tiers: Connected (live WebSocket to desktop) / Disconnected (standalone + GitHub API)
- Four tabs: Console, Repos, Build, Config
- Depends on `caboose-server` for Connected mode

### 1.0.0 — Platform Stable
- Codebase indexing / semantic retrieval
- Stable `caboose-core` API surface
- All major competitive gaps closed
