```
       ▄████████▄
       █        █
▄▄████████████████████▄▄
  █    █        █    █
  ████████████████████
  ▀ ▄██▄        ▄██▄ ▀
    ▀██▀        ▀██▀

▄▀▀▀ ▄▀▀▄ █▀▀▄ ▄▀▀▄ ▄▀▀▄ ▄▀▀▀ █▀▀▀
█    █▀▀█ █▀▀▄ █  █ █  █ ▀▀▀█ █▀▀
 ▀▀▀ ▀  ▀ ▀▀▀   ▀▀   ▀▀  ▀▀▀  ▀▀▀▀
```

**A terminal-native AI coding agent built in Rust.**

Caboose is a fast, single-binary AI coding agent for your terminal. It streams responses from multiple LLM providers, executes tools, manages persistent sessions, and supports an extensible skills system — all rendered in a rich TUI with syntax highlighting and an embedded terminal.

## Install

**macOS (Homebrew):**

```bash
brew install kahanscious/tap/caboose
```

**macOS/Linux (curl):**

```bash
curl -fsSL https://downloads.trycaboose.dev/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://downloads.trycaboose.dev/install.ps1 | iex
```

**Build from source:**

```bash
cd tui && cargo build --release
```

## Quick Start

```bash
caboose
```

Once inside, use slash commands:

- `/connect` — connect your API keys
- `/init` — generate a `CABOOSE.md` project context file
- `/settings` — configure providers, models, and preferences
- Type `/` to see all available commands

## Highlights

- **Multi-provider** — Anthropic, OpenAI, Gemini, OpenRouter, DeepSeek, Groq, Mistral
- **Permission modes** — Plan, Create, AutoEdit, Chug. Cycle with `Tab`
- **Persistent sessions** — SQLite-backed. Resume any session with `Ctrl+O`
- **Skills** — Built-in slash commands (`/brainstorm`, `/debug`, `/tdd`, `/review`, `/plan`) plus user-defined
- **Memory** — Cross-session fact extraction
- **MCP** — Extend tools via Model Context Protocol servers
- **Embedded terminal** — Full PTY shell inside the TUI (`Ctrl+=`)
- **Bring your own keys** — No subscription. Per-turn pricing with optional session budgets

## Documentation

Full docs, configuration reference, and guides at **[docs.trycaboose.dev](https://docs.trycaboose.dev)**.

## Development

```bash
cd tui
cargo build              # debug build
cargo test               # run all tests
cargo clippy             # lint
```

## Acknowledgments

Built-in skills inspired in part by [superpowers](https://github.com/obra/superpowers) by Jesse Vincent. If you prefer the superpowers workflow, you can use it with Caboose:

1. Disable any overlapping built-in skills via `/settings` (or add them to `disabled_skills` in your config)
2. Copy the superpowers `SKILL.md` files into `~/.config/caboose/skills/` (global) or `.caboose/skills/` (per-project)

User skills with the same name as a built-in automatically override it.

## License

MIT
