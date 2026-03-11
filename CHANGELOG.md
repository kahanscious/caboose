# Changelog

All notable changes to Caboose will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - Unreleased

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
