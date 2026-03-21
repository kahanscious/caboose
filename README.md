<p align="center">
  <a href="https://docs.trycaboose.dev"><img src="media/caboose-transparent.svg" width="120" alt="Caboose"></a>
</p>

<h1 align="center">Caboose</h1>

<p align="center"><strong>A terminal-native AI coding agent built in Rust.</strong></p>

<p align="center">
  <a href="https://github.com/kahanscious/caboose/releases/latest"><img src="https://img.shields.io/github/v/release/kahanscious/caboose" alt="Latest release"></a>
  <a href="https://github.com/kahanscious/caboose/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
  <a href="https://docs.trycaboose.dev"><img src="https://img.shields.io/badge/docs-trycaboose.dev-green" alt="Documentation"></a>
</p>

---

Most AI coding agents lock you into one model and one subscription. Caboose doesn't. Bring your own API keys, pick any of 23+ providers, and work entirely in your terminal — no browser, no Electron, no cloud account required.

Outside of it being a coding assistant, it has **Roundhouse**: send the same prompt to multiple models in parallel, watch them plan independently, then synthesize the best ideas into one unified implementation plan. It's the closest thing to a second (and third) opinion before you write a line of code.

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

Once inside:

- `/connect` — add your API keys
- `/init` — generate a `CABOOSE.md` project context file
- `/model` — switch models mid-session
- Type `/` to see all commands

## What makes it different

**Roundhouse — multi-model parallel planning**
Send a prompt to Claude, GPT-4o, and Gemini at the same time. Each model plans independently. You review, annotate, and critique between phases. Caboose synthesizes the best approach into one plan and drops it into chat. Saved to `.caboose/roundhouse/` for reference.

**Bring your own keys, 23+ providers**
Anthropic, OpenAI, Gemini, OpenRouter, xAI, Together AI, Fireworks AI, Cerebras, SambaNova, Perplexity, Cohere, Qwen, DeepSeek, Groq, Mistral, AI21 Labs, Moonshot AI, 01.AI, Zhipu AI, Novita AI, Inflection AI, Hugging Face, Reka AI — plus Ollama, LM Studio, and llama.cpp for local models. No subscription. You pay the provider directly, per token.

**Subagents in isolated git worktrees**
Spawn parallel sub-agents for independent tasks. Each runs in its own worktree, merges back on success, and flags conflicts for review. You stay in the main session the whole time.

**Single binary, pure terminal**
Written in Rust. One binary, no runtime dependencies. Runs on macOS, Linux, and Windows. The TUI includes syntax highlighting, collapsible diffs, an embedded PTY terminal, and full mouse support.

**Persistent memory and project rules**
Caboose remembers across sessions — project conventions, architectural decisions, recurring patterns. Memory lives in human-editable markdown files with dual scoping (project `.caboose/memory/` vs global `~/.config/caboose/memory/`). End-of-session extraction captures what you did. `/pin` sets session rules; `/pin --save` promotes them to `CABOOSE.md` so every future session follows them.

**Skills**
Slash-command workflows that load structured prompts into the agent. Ships with 12 built-in skills (`/brainstorm`, `/tdd`, `/debug`, `/review`, `/doc`, and more). Add your own in `~/.config/caboose/skills/` or drop them in `.caboose/skills/` per project.

## More features

- **Hooks / lifecycle events** — 13 hook events (SessionStart, PreToolUse, PostToolUse, Stop, PreCompact, and more). Run external shell commands on tool execution, session start/end, or any lifecycle event. PreToolUse hooks can block tool calls. Configure in `config.toml` with tool name filtering and timeouts.
- **Context compaction** — three-pass automatic summarization when context fills up. Tool output pruning, mechanical noise removal, then LLM-generated structured summary. Configurable threshold, optional cheaper model for summarization. Sessions run indefinitely without hitting context limits.
- **Checkpoint / rewind** — automatic file snapshots before every tool execution. `/undo` reverts the last change; `/rewind` opens a picker showing all checkpoints with prompt previews and file counts. `/checkpoint <name>` saves a named bookmark. Rewind to any point in your session.
- **Cost tracking** — real-time token and cost display in sidebar and `/status` dialog. Per-turn, per-session, and per-subagent cost breakdown. Cache-aware pricing (Anthropic cache read/creation rates). Session budgets with pause/raise/stop dialog. Cost persisted to SQLite across sessions.
- **Thinking / reasoning** — streaming thinking blocks from Anthropic, OpenAI, and Gemini. Configurable level via `Ctrl+T`
- **`!` shell shortcut** — run shell commands inline without leaving chat (`!git log`, `!ls`, etc.)
- **Circuits** — scheduled recurring prompts, in-session or persistent via background daemon
- **MCP** — extend tools via Model Context Protocol servers (stdio and SSE/HTTP), with built-in GitHub and GitLab presets
- **Multi-repo workspaces** — register sibling repos the agent can read from and write to
- **Image attachments** — drag-and-drop, `@path.png` references, or clipboard paste
- **`/suggest`** — scans your codebase with clippy, tests, TODO/FIXME grep, and git churn; surfaces prioritized findings
- **Session search** — full-text search across all past sessions in the session picker
- **Persistent sessions** — SQLite-backed. Resume any session with `Ctrl+O`
- **Settings migration** — import MCP servers, system prompts, and project files from Claude Code, Open Code, and Codex
- **Permission modes** — Plan, Create, AutoEdit, Chug. Cycle with `Tab`
- **Memory** — dual-scoped (project + global), auto-extraction at session end, FTS5 search across all stored facts. Human-editable markdown — no hidden database
- **Session pins** — `/pin` adds runtime rules; `/pin --save` writes them to `CABOOSE.md` so they persist permanently
- **Hover-to-copy** — mouse over any assistant message or code block to copy with `y` or a click
- **Context window indicator** — live `XX% ctx` display in the footer, color-coded by usage
- **Embedded server** — WebSocket server for mobile and web clients, with device pairing auth and bidirectional event bridge
- **Background agents** — `/bg` spawns background agents with per-agent and global token budget enforcement
- **Web search backends** — pluggable `SearchBackend` trait with SearXNG (self-hosted) and Tavily (paid API) implementations
- **`/context`** — shows model, tokens, tools, MCP servers, skills, memory, and permission mode at a glance

## Built-in Skills

Every skill can be toggled on or off via `/settings`. Add your own in `~/.config/caboose/skills/` (global) or `.caboose/skills/` (per-project). User skills with the same name as a built-in automatically override it.

| Skill | Description |
|-------|-------------|
| `/brainstorm` | Explore 3–5 design approaches, then converge on the best option with a decision record |
| `/plan` | Write a granular, step-by-step implementation plan with file paths, code, and test commands |
| `/debug` | Systematic fault isolation — reproduce, bisect, read, prove with a failing test |
| `/doc` | Generate idiomatic documentation comments for modules, functions, and types |
| `/tdd` | Enforce strict RED-GREEN-REFACTOR test-driven development |
| `/finish` | Audit the current branch before integration — build, tests, lint, diff review |
| `/handoff` | Generate a structured session summary so the next session picks up where you left off |
| `/review` | Five-pass code review — exploration, correctness, clarity, edge cases, excellence |
| `/refactor` | Identify DRY violations, naming issues, complexity, and extraction opportunities |
| `/test` | Generate comprehensive test cases covering happy paths, edge cases, and error conditions |
| `/explain` | Explain how code works — summary, key functions, data flow, design decisions, dependencies |
| `/optimize` | Identify performance bottlenecks with impact ratings and before/after suggestions |

## Documentation

Full docs, configuration reference, and guides at **[docs.trycaboose.dev](https://docs.trycaboose.dev)**.

## Development

Caboose is a Cargo workspace: `core/` contains platform-agnostic domain logic (`caboose-core`), `tui/` contains the terminal frontend.

```bash
cargo build --workspace       # build both crates
cargo test --workspace        # run all tests (~1256)
cargo clippy --workspace      # lint
cargo build --release -p caboose  # optimized release binary
```

## Acknowledgments

Built-in skills inspired in part by [superpowers](https://github.com/obra/superpowers) by Jesse Vincent. If you prefer the superpowers workflow, you can use it directly with Caboose — disable any overlapping built-in skills via `/settings` and copy the `SKILL.md` files into `~/.config/caboose/skills/`.

## License

MIT
