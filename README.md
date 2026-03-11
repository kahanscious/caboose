<p align="center">
  <img src="media/caboose-transparent.svg" width="120" alt="Caboose">
</p>

<h1 align="center">Caboose</h1>

<p align="center"><strong>A terminal-native AI coding agent built in Rust.</strong></p>

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
- **Local LLMs** — Ollama, LM Studio, llama.cpp with auto-discovery and one-click connect
- **Roundhouse** — Multi-LLM planning: multiple models plan independently, then synthesize into one unified plan
- **Circuits** — Scheduled recurring tasks (in-session or persistent via daemon)
- **SCM integration** — GitHub and GitLab tools (issues, PRs/MRs, file contents) with MCP presets
- **Settings migration** — Import MCP servers, system prompts, and project files from Claude Code, Open Code, and Codex
- **Permission modes** — Plan, Create, AutoEdit, Chug. Cycle with `Tab`
- **Persistent sessions** — SQLite-backed. Resume any session with `Ctrl+O`
- **Skills** — Built-in slash commands (`/brainstorm`, `/debug`, `/tdd`, `/review`, `/plan`) plus user-defined
- **Memory** — Cross-session fact extraction
- **MCP** — Extend tools via Model Context Protocol servers, with built-in presets
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

## Built-in Skills

Caboose ships with 11 slash-command skills. Every skill can be toggled on or off via `/settings`, and you can add your own in `~/.config/caboose/skills/` or `.caboose/skills/`.

| Skill | Description |
|-------|-------------|
| `/brainstorm` | Explore 3–5 design approaches, then converge on the best option with a decision record |
| `/plan` | Write a granular, step-by-step implementation plan with file paths, code, and test commands |
| `/debug` | Systematic fault isolation — reproduce, bisect, read, prove with a failing test |
| `/tdd` | Enforce strict RED-GREEN-REFACTOR test-driven development |
| `/finish` | Audit the current branch before integration — build, tests, lint, diff review |
| `/handoff` | Generate a structured session summary so the next session picks up where you left off |
| `/review` | Five-pass code review — exploration, correctness, clarity, edge cases, excellence |
| `/refactor` | Identify DRY violations, naming issues, complexity, and extraction opportunities |
| `/test` | Generate comprehensive test cases covering happy paths, edge cases, and error conditions |
| `/explain` | Explain how code works — summary, key functions, data flow, design decisions, dependencies |
| `/optimize` | Identify performance bottlenecks with impact ratings and before/after suggestions |

## Acknowledgments

Built-in skills inspired in part by [superpowers](https://github.com/obra/superpowers) by Jesse Vincent. If you prefer the superpowers workflow, you can use it with Caboose:

1. Disable any overlapping built-in skills via `/settings` (or add them to `disabled_skills` in your config)
2. Copy the superpowers `SKILL.md` files into `~/.config/caboose/skills/` (global) or `.caboose/skills/` (per-project)

User skills with the same name as a built-in automatically override it.

## License

MIT
