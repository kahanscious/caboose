# Changelog

All notable changes to Caboose will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
