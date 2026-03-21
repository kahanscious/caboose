# Changelog

All notable changes to Caboose will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] - 2026-03-20

### Changed

- **Workspace architecture** — split the monolithic `tui/` crate into a Cargo workspace: `core/` (`caboose-core`, platform-agnostic domain logic) and `tui/` (terminal frontend). All domain modules — agent, provider, tools, config, session, memory, MCP, skills, roundhouse, safety, and more — now live in `caboose-core`. The TUI crate imports core and adds terminal-specific layers (ratatui rendering, LSP, embedded PTY, clipboard, self-updater).
- **`app.rs` module split** — broke the 13,600-line `app.rs` monolith into 15 focused modules under `app/`: `mod.rs` (event loop), `types.rs`, `state.rs`, `key_dispatch.rs`, `tool_handlers.rs`, `pickers.rs`, `dialogs.rs`, `provider_mgmt.rs`, `slash_commands.rs`, `handoff.rs`, `roundhouse.rs`, `session_mgmt.rs`, `skills.rs`, `input.rs`, `helpers.rs`.
- **Stale code cleanup** — deleted 30+ duplicate files that existed in both crates after the core extraction. TUI modules now re-export from `caboose-core` instead of carrying copies.
- **Dependency cleanup** — removed 12 duplicate deps from `tui/Cargo.toml` and 7 phantom deps from `core/Cargo.toml` that were declared but never used.

### Fixed

- **Fast typing no longer triggers paste detection** — tightened the rapid-input gap threshold from 180ms to 50ms. Previously, typing 12+ characters at ~80 WPM and pressing Enter would insert a newline instead of sending. Bracketed paste (enabled by default) handles real pastes; the key-by-key fallback now only triggers at inhuman input speeds.

---

## [0.6.4] - 2026-03-20

### Added

- **8 new LLM providers** — AI21 Labs, Moonshot AI (Kimi), 01.AI (Yi), Zhipu AI (GLM), Novita AI, Inflection AI, Hugging Face, Reka AI. Caboose now supports 23 API providers + 4 local options. Aliases: `kimi` → moonshot, `01ai` → yi, `hf` → huggingface.
- **Cache-aware cost calculation** — `cache_read_tokens` and `cache_creation_tokens` now factor into cost estimates. Anthropic models use explicit rates from docs (10% for cache reads, 125% for cache creation). All providers use default cache rates.
- **Session cost persistence** — `total_input_tokens`, `total_output_tokens`, and `total_cost_usd` saved to the session SQLite table. Auto-migrates existing databases.
- **`/cost` command** — alias for `/status`, opens the session status dialog showing provider, tokens, cost, and per-M rates.
- **`/checkpoint <name>`** — save a named checkpoint at the current state. Named checkpoints appear in the `/rewind` picker with their name instead of a prompt preview.
- **Diff preview on rewind** — after rewinding via `/rewind`, the system message now includes per-file detail: which files were restored (with +/- line counts) and which were deleted.
- **`compaction_model` config wired** — set `compaction_model = "claude-haiku-4-5-20251001"` in `[behavior]` config to use a cheaper model for context compaction summarization. Falls back to active model if resolution fails.
- **Pricing for new providers** — 15 new model entries across AI21, Moonshot, Yi, Zhipu, Novita, Inflection, Hugging Face, and Reka.

### Changed

- **Pricing refactored** — `ModelPricing::standard()` helper with default cache rates. `estimate_cost_with_cache()` method for full cache-aware cost breakdown.
- **README updated** — added documentation for hooks/lifecycle events, context compaction, checkpoint/rewind, and cost tracking. Provider count updated to 23+.

---

## [0.6.3] - 2026-03-19

### Added

- **`/help` overlay** — scrollable modal showing all keybindings and commands, grouped by category (Navigation, Session, Provider, Tools, Context Keys). Scroll with `j`/`k`, close with `Esc`.
- **Session picker message preview** — each row in the `/sessions` picker now shows a dimmed first-message preview below the title. Search results show matched content snippets instead.
- **Per-code-block hover-to-copy** — hovering over a code block in an assistant message shows `[ y copy ]` on the block header. Press `y` or click to copy just that block. Code block hover takes priority over message-level copy.
- **`/doc` skill** — new built-in skill that generates idiomatic documentation comments for modules, functions, and types.
- **`/pin --save`** — promotes a session pin to `CABOOSE.md` so the rule persists across all future sessions. Creates a `## Rules` section if one doesn't exist.
- **Home screen tips** — added tips for `/memories`, `/pin --save`, `/doc`, and `/help`.

---

## [0.6.2] - 2026-03-19

### Added

- **Scroll-to-bottom badge** — when scrolled up, a clickable `[ ↓ new messages ]` badge appears bottom-right of the chat area. Click it or press `G` to jump to the latest message.
- **Context window usage indicator** — footer shows `XX% ctx` right-anchored next to the version string, colored dim/yellow/red at <75%/≥75%/≥90%. Appears after the first assistant response.
- **`/new` confirmation dialog** — starting a new session (via `/new` or Ctrl+N) now asks for confirmation when the session has real messages. Empty sessions skip the dialog.

### Fixed

- Enter key inserting a newline instead of executing slash commands when typed quickly. Paste detection now bypasses slash commands entirely.
- Paste detection state (`rapid_input_streak`) carrying over across `/new` sessions, causing the first Enter in a fresh session to be swallowed.

---

## [0.6.1] - 2026-03-19

### Added

- **Hover-to-copy on assistant messages** — mouse over any assistant message to reveal a `[ y copy ]` badge in the top-right corner. Press `y` or click the badge to copy the full message text to clipboard. Confirmation shown as a system message.
- **Roundhouse prompt in sidebar** — the original prompt is shown dim/italic below the Roundhouse header during all phases (including Complete and Cancelled), so you never lose track of what you asked.

---

## [0.6.0] - 2026-03-18

### Added

- **Roundhouse v2** — complete overhaul with a dedicated full-screen experience. Left panel (65%) streams model output; right panel (35%) shows phase navigator, model list with status icons, cost, and annotation count. Replaces inline chat rendering.
- **Gated phase flow** — human checkpoints between planning, critique, and synthesis. After each phase: `[c]` critique, `[s]` skip to synthesis, `[a]` annotate, `[q]` cancel. `j`/`k` switches between models in real-time.
- **Roundhouse annotations** — type feedback at any review gate (e.g. "Use Claude's DB approach"). Injected into subsequent phase prompts as a "User Guidance" section; models instructed to respect user guidance above their own judgment. Included in output file.
- **Collapsible model picker groups** — `/model` picker now groups models by provider with `▼`/`▶` headers. Press Enter on a header to expand/collapse. Active provider expanded by default; others collapsed. OpenRouter and other configured providers always shown regardless of active provider.
- **Local server connect entries** — Ollama, LM Studio, llama.cpp, and Custom server connect options pinned at the top of the model picker. Select to connect on a custom port. Session remembers manually connected servers.
- **`!` shell shortcut** — type `!<command>` to run a shell command directly without LLM involvement. Output shown as a system message in chat. Supports pipes, redirects, and shell builtins via `sh -c`. Truncates at 200 lines. Shows `[exit code: N]` on failure. Works with no API key configured.

### Changed

- Roundhouse output files now saved to `.caboose/roundhouse/<YYYY-MM-DD>-<slug>.md`; synthesis inserted as an Assistant message in Chat on completion.
- Removed `/roundhouse execute` subcommand — synthesis flows naturally into chat.
- Model picker selected item now stays centered in the viewport while scrolling.
- Roundhouse cancellation errors now display in red.
- Escape or Ctrl+C immediately exits roundhouse from any phase.
- Slash commands disabled while roundhouse is active to prevent conflicts.

---

## [0.5.0] - 2026-03-18

> Includes all features from the unreleased 0.4.1 cycle.

### Added

- **8 new API providers** — xAI (Grok), Together AI, Fireworks AI, Cerebras, SambaNova, Perplexity, Cohere, Qwen (DashScope). Caboose now supports 15 API providers + 4 local options.
- **MCP SSE/HTTP transport** — connect to remote MCP servers via URL (`url = "https://..."`) instead of only spawning local processes.
- **Auto session titling** — LLM-generated 3–6 word titles after first turn. Non-blocking; falls back to truncated first message. Configurable via `auto_title` in `[behavior]` config.
- **`/status` command** — replaces `/usage`, expanded with provider, model, and permission mode. `/usage` kept as alias.
- **`/undo` command** — quick shortcut to rewind the most recent checkpoint with file changes.
- **Non-interactive JSON output** — `-f json` flag for `--prompt` mode with structured response, token counts, and tool calls.
- **Image attachments + compression** — drag-and-drop, `@path.png` references, absolute path detection. 3-step cascade: passthrough → resize → JPEG re-encode. Alpha-aware (PNG/WebP with transparency skip JPEG). `[images]` config section.
- **Reasoning level control** — `ThinkingMode` expanded to Off/Low/Medium/High. `/reasoning` slash command with picker. Provider-native mapping (Anthropic budget_tokens, OpenAI reasoning_effort, Gemini thinking_budget). Ctrl+T toggles off/medium.
- **`/suggest`** — evidence-based codebase scanning. Configurable lint/test commands or auto-detected from project files. Typed parsers for cargo clippy (JSON), cargo test, TODO/FIXME grep, git churn. Findings deduplicated and prioritized. Toggleable via `/settings`.
- **Session full-text search** — FTS5 virtual table indexes all message content. Typing in `/sessions` searches across all sessions via ranked matching.
- **Auto-fix error recovery** — agent automatically retries failed shell commands. Circuit breaker stops after 3 consecutive failures of the same command.

---

## [0.4.0] - 2026-03-16

### Added

- **Conflict detection layer** — proactive hunk-level analysis before merging parallel agent results. Detects overlapping file edits, add-vs-add, delete-vs-modify, and rename conflicts. Non-overlapping edits auto-merge; blocking overlaps surface a structured report with per-agent line ranges for user approval. Agents enter a `Review` state after execution, waiting for all siblings before any merge.

---

## [0.3.0] - 2026-03-13

### Added

- **Multi-repo workspaces** — `/workspace` registers sibling repos the agent can read from and write to. Supports proactive (auto-searched) and explicit (reference-only) modes, with read-only or read-write access per workspace. Workspace add flow guides through path, name, mode, and permissions. Agent is blocked from accessing paths outside the primary project unless explicitly registered as a workspace.
- **Inline diff viewer** — pending write/edit/patch approvals show a collapsible diff preview. `d` key or click toggles expand/collapse, `j/k/arrows` scroll. Post-execution diffs also collapsible per-message.
- **Autonomous subagent spawning** — model calls `spawn_agent` to parallelize independent tasks into isolated git worktrees. Non-blocking design with approval bubbling for non-Chug modes (y/n/a where "a" is always-approve). Auto-merge on success, conflict detection, worktree cleanup. Sidebar shows live agent status with blinking dots, elapsed time.
- **Thinking blocks** — streaming thinking content from Anthropic models rendered as collapsible blocks in chat. Click or arrow to expand/collapse. Thinking persisted and restored across sessions. OpenAI-compatible providers (OpenRouter, DeepSeek) now emit reasoning content via `reasoning`/`reasoning_content` fields.
- **Thinking toggle** — `Ctrl+T` toggles thinking on/off for models that support it. Status bar shows `thinking` indicator when active. Anthropic models get `anthropic-beta` header with `thinking` param. OpenAI/OpenRouter models get `reasoning_effort`. Gemini 2.5 models get `thinkingConfig`. Per-model capability detection: Anthropic hardcoded, OpenRouter from `supported_parameters`, OpenAI by model prefix (`o1`/`o3`/`o4`), Gemini by model prefix (`gemini-2.5`). Toggle hidden for non-thinking models.
- **Model picker search** — typing in the `/model` picker now shows a visible search textbox with live filter text and cursor. Also visible in `/sessions`, `/connect`, and `/skills` pickers.
- **Collapsible files modified** — sidebar "Files Modified" section is now collapsible. Click the header to toggle. Collapsed view shows file count and total +/- on one line.
- **Sidebar resize** — drag the sidebar border to resize. Clamped between 20–80 columns.

### Changed

- Thinking blocks show static "Thought process" label when collapsed after completion instead of looping typewriter animation. Click to re-expand and view full thinking content.

- **System prompt overhaul** — Caboose now has a defined personality: conversational, direct, narrates what it's about to do, no filler. Replaces the generic "helpful assistant" prompt. Still overridable via `system_prompt` in config.
- Task outlines automatically cleared when user sends a new message, so stale tasks don't linger after topic changes.

- Subagent dismiss is now a clickable "clear" button instead of `D` keyboard shortcut.
- Subagent cost tracking uses actual model pricing from PricingRegistry instead of hardcoded rates.

### Fixed

- Escape cancel now fully cleans up state so the next prompt works correctly.
- Subagent approval dialog now appears properly (layout allocates space for subagent approvals).
- WaitingApproval agents show yellow dot instead of gray.
- Stale git branches from failed agent runs no longer block new agent spawning.

---

## [0.2.0] - 2026-03-11

### Added

- **Local LLM providers** — Connect to Ollama, LM Studio, llama.cpp, or custom OpenAI-compatible servers. Auto-discovery probes local ports on startup; one-click connect dialog with address input and model picker.
- **Roundhouse (multi-LLM planning)** — Launch parallel planning sessions across multiple models. Each model plans independently, then the primary synthesizes all plans into a single unified implementation plan. Live streaming in the main chat window with typewriter status in the sidebar.
- **Circuits (scheduled tasks)** — `/circuit` command runs recurring prompts on a timer. In-session circuits run inside the TUI; `--persist` circuits survive via the background daemon. `/circuits` lists and manages active circuits.
- **SCM integration** — Auto-detects GitHub or GitLab from git remotes. Registers platform-specific tools (issues, PRs/MRs, file contents, repo search). Built-in MCP presets for GitHub and GitLab servers.
- **Settings migration** — `/migrate` imports configuration from Claude Code, Open Code, and Codex. Scans MCP servers, system prompts, and project instruction files with a toggle checklist and preview before applying.
- **Daemon subsystem** — `caboose daemon` runs a background TCP server for persistent circuits. Lockfile-based discovery with PID liveness checks to auto-clean stale lockfiles.
- **MCP presets** — Built-in GitHub and GitLab MCP server configurations, toggleable from `/settings`.
- Roundhouse purple accent color across all theme variants.

### Changed

- Model picker now groups models by provider with separator headers.
- Sidebar Roundhouse section shows per-planner typewriter status animation.
- Roundhouse plan files are gitignored by default (`roundhouse-*.md`).

### Fixed

- UTF-8 string slicing panics in provider name truncation, shell output truncation, and handoff message truncation.
- `local_providers` config merge used wrong precedence — project config now correctly overrides global.
- Roundhouse planner failure left update channel active, causing wasted work every event loop tick.
- `roundhouse_model_add` flag not cleared on cancel/clear/failure — could misroute subsequent model picker selections.
- Config save functions could panic on corrupted config files.
- Stale daemon lockfiles permanently blocked new daemon starts after a crash.
- Circuit IDs used a small modulo range with no duplicate check.
- Roundhouse execute race condition — phase transitioned to Complete before the agent started working on the plan.
- Triple Ctrl+C required to quit — now consistently requires only two presses.

## [0.1.1] - 2026-03-10

### Added

- Install via Homebrew: `brew install kahanscious/tap/caboose`
- Linux support — static musl binary, works on any distro
- Automated cross-platform builds via cargo-dist (macOS ARM/Intel, Windows, Linux)
- Image support — attach PNG, JPG, WebP, GIF via `@file` or Ctrl+A file browser
- Smarter compaction — lower threshold (85%), post-compaction file re-reading, richer summaries, tool output pruning
- Update notification — shows in footer when a new version is available
- `caboose update` command — self-update that detects install method (Homebrew/Chocolatey/direct)

### Changed

- Install script now uses `.tar.xz` format (smaller downloads)
- CI runs on macOS and Windows in addition to Linux

## [0.1.0] - 2026-03-06

### Added

- Embedded terminal panel
- Web search tool
- Inline diff display for edits in chat
- Theme picker
- MCP server presets with TUI toggle
- Scroll wheel support in menus and dropdowns
- `@file` fuzzy search and files modified sidebar
- Session budget limit with checkpoint/rewind
- LSP diagnostics and navigation tools
- Clipboard copy support
- Skill creator and handoff skill
- Curl/PowerShell install scripts
- Chocolatey package for Windows
