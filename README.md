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
# Interactive mode
caboose

# Connect a provider
/connect anthropic

# Non-interactive
caboose --prompt "explain this function"
```

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

## License

MIT
