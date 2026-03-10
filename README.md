# Caboose

**A terminal-native AI coding agent built in Rust.**

Caboose is a fast, single-binary AI coding agent for your terminal. It streams responses from multiple LLM providers, executes tools (file reads, edits, shell commands, web fetches), manages persistent sessions, and supports an extensible skills system — all rendered in a rich TUI with syntax highlighting, markdown, and an embedded terminal.

### Why Caboose?

- **Fast** — Compiled Rust, direct HTTP streaming, no JS runtime. Instant startup, zero overhead.
- **Multi-provider** — Anthropic, OpenAI, Gemini, OpenRouter, DeepSeek, Groq, Mistral. Switch models mid-conversation with `/model`.
- **Permission modes** — Four levels from read-only (`Plan`) to full auto-execute (`Chug`). Cycle with `Tab`.
- **Persistent sessions** — SQLite-backed conversation history. Resume any session with `Ctrl+O`.
- **Skills** — 11 built-in slash commands (`/brainstorm`, `/debug`, `/tdd`, `/review`, etc.) plus user-defined skills.
- **Memory** — Cross-session fact extraction. Caboose remembers what matters between conversations.
- **MCP** — Extend tools via Model Context Protocol servers. Stdio transport, live status in sidebar.
- **Embedded terminal** — Full PTY-backed shell panel inside the TUI. Toggle with `Ctrl+=`.
- **Bring your own keys** — No subscription. You control costs with per-turn pricing and optional session budgets.

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

## Getting Started

### Build from source

```bash
cd tui
cargo build --release
```

The binary is at `tui/target/release/caboose` (or `caboose.exe` on Windows).

### Run

```bash
# Interactive mode
caboose

# Non-interactive (pipe-friendly)
caboose --prompt "explain this function"
```

### Configure a provider

On first launch, connect a provider:

```
/connect anthropic
```

Or set an environment variable:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

Keys can also be stored in `.caboose/auth.json` or `~/.config/caboose/config.toml`.

## Providers

| Provider | Default Model | Key Required |
|----------|--------------|:------------:|
| Anthropic | `claude-sonnet-4` | Yes |
| OpenAI | `gpt-4.1` | Yes |
| Gemini | `gemini-2.0-flash` | Yes |
| OpenRouter | `anthropic/claude-sonnet-4` | Yes |
| DeepSeek | `deepseek-chat` | Yes |
| Groq | `llama-3.3-70b-versatile` | Yes |
| Mistral | `mistral-large-latest` | Yes |

Switch providers at any time with `/model` or `Ctrl+M`.

## Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read files with optional offset/limit |
| `write_file` | Create new files |
| `edit_file` | Line-based editing of existing files |
| `glob` | Pattern-based file search |
| `grep` | Regex search across files |
| `bash` | Shell command execution (sandboxed) |
| `list_directory` | Directory listing |
| `fetch` | Web content retrieval |
| `web_search` | Web search via configured provider |
| `todo_write` | Create and update task lists during execution |
| `todo_read` | Read current task state |
| `explore` | Multi-step codebase exploration |
| `agent` | Spawn subagent for parallel work |
| `ask_user` | Ask structured multiple-choice questions mid-turn |

Tool approval is governed by your permission mode.

## Permission Modes

Cycle through modes with `Tab`:

| Mode | Behavior |
|------|----------|
| **Plan** | Read-only. No writes, no shell, no MCP. |
| **Create** | Interactive approval. Reads auto-approved, writes and commands prompt for confirmation. |
| **AutoEdit** | File edits auto-approved. Shell commands still prompt. |
| **Chug** | Everything auto-approved. Full autonomy. |

## Commands

| Command | Description |
|---------|-------------|
| `/model` | Switch model or provider |
| `/connect` | Add or update an API key |
| `/sessions` | Browse and resume previous sessions |
| `/new` | Start a fresh session |
| `/memories` | View stored memories |
| `/forget` | Remove memory entries |
| `/settings` | Toggle memory, skills, and other options |
| `/create-skill` | LLM-guided skill generator |
| `/skills` | List available skills |
| `/init` | Generate a `CABOOSE.md` project file |
| `/mcp` | List, add, or restart MCP servers |
| `/title` | Rename the current session |
| `/handoff` | Generate a session handoff summary (also auto-prompted at 90% context) |
| `/terminal` | Toggle embedded terminal |

Access all commands via `Ctrl+K` (command palette) or type `/` for autocomplete.

## Skills

Built-in skills are slash commands that inject structured prompts into the conversation:

| Skill | Purpose |
|-------|---------|
| `/brainstorm` | Diverge-then-converge design exploration |
| `/plan` | Granular implementation planning with file targets |
| `/debug` | Reproduce-bisect-read-prove fault isolation |
| `/tdd` | RED-GREEN-REFACTOR test-driven development |
| `/review` | Iterative code review (Rule of Five) |
| `/refactor` | Guided refactoring with safety checks |
| `/optimize` | Performance analysis and improvement |
| `/explain` | Code explanation and documentation |
| `/test` | Test generation and coverage analysis |
| `/finish` | Quality gates, diff audit, and handoff |
| `/handoff` | Compact session summary for continuity |

User-defined skills in `.caboose/skills/` or `~/.config/caboose/skills/` override built-ins by name.

Create new skills interactively with `/create-skill <name> <goal>`.

## Key Bindings

| Key | Action |
|-----|--------|
| `Enter` | Send message (queues up to 3 while agent is busy) |
| `Shift+Enter` | Insert newline |
| `Tab` | Cycle permission mode |
| `Up` / `Down` | Input history |
| `Ctrl+K` | Command palette |
| `Ctrl+O` | Session picker |
| `Ctrl+M` | Model picker |
| `Ctrl+B` | Toggle sidebar |
| `Ctrl+=` | Toggle embedded terminal |
| `Ctrl+C` | Copy selection / cancel / quit (press twice) |
| `Ctrl+V` | Paste from clipboard |
| `e` | Expand truncated message |

## Configuration

Caboose loads config from two layers (project overrides global):

- **Global**: `~/.config/caboose/config.toml`
- **Project**: `.caboose/config.toml`

```toml
[provider]
default = "anthropic"
model = "claude-sonnet-4"

[tools]
allow = ["read_file", "write_file", "edit_file", "glob", "grep", "bash", "list_directory", "fetch"]

[behavior]
auto_handoff_prompt = true  # prompt to handoff at 90% context (default: true)
max_session_cost = 10.0     # pause agent when session spend reaches $10 (default: off)

[mcp.servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
```

### CABOOSE.md

Run `/init` to generate a `CABOOSE.md` file in your project root. This file is automatically included in every prompt, giving the agent persistent context about your project structure, conventions, and preferences.

## MCP (Model Context Protocol)

Extend Caboose with external tool servers:

1. **Built-in presets** — Context7 (library docs) and Fetch (web content) ship as toggleable presets
2. **`/mcp` dropdown** — Toggle servers on/off with `Tab`, manage with `Enter` (Restart/Remove)
3. **Async connections** — Servers connect in the background. The UI never blocks.
4. **Custom servers** — Add your own in `.caboose/config.toml` under `[mcp.servers]`
5. Tools are namespaced as `server:tool_name` and available to the agent automatically

```toml
[mcp.servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
```

## Memory

Caboose extracts and persists facts across sessions:

- **Project memory**: `.caboose/memory/MEMORY.md` — project-specific knowledge
- **Global memory**: `~/.config/caboose/memory/` — cross-project knowledge
- **Auto-extraction**: At session end, observations are summarized and stored
- **Commands**: `/memories` to view, `/forget` to remove entries

## Sessions

Every conversation is persisted to SQLite automatically:

- `Ctrl+O` opens the session picker with search
- Sessions are titled from the first message (rename with `/title`)
- Full conversation history is preserved and resumable
- Non-interactive mode (`--prompt`) also creates sessions

## Session Budget

Set a maximum spend per session to prevent runaway costs during long agentic loops:

- **Configure** via `/settings` (preset amounts) or `max_session_cost` in config (any custom amount)
- **Warning** at 80% — cost indicator appears in the footer status bar
- **Pause** at 100% — agent stops before the next request with options to **continue**, **raise the limit**, or **stop**
- Tracks cost across all models used in a session, resets each app run
- Default: **off** (no limit)

## Workflow

The intended development loop with Caboose:

1. **Brainstorm** (`/brainstorm`) — Explore the idea, clarify requirements, pick an approach
2. **Plan** (`/plan`) — Break the approach into granular, file-targeted implementation steps
3. **Execute** — Work through the plan step by step. Caboose tracks progress via task lists and manages context automatically (compaction, cold storage, handoff prompts)
4. **Review** (`/review`) — Iterative code review against the plan
5. **Finish** (`/finish`) — Quality gates, diff audit, commit, and PR
6. **Handoff** (`/handoff`) — Generate a compact session summary. Resume in a new session or hand off to a teammate

Each step is a skill (slash command) that injects a structured prompt. You can enter the loop at any point — not every task needs all six steps.

## Project Structure

```
tui/
├── src/
│   ├── main.rs              # CLI entry point
│   ├── app.rs               # Event loop and state machine
│   ├── agent/               # Agent loop, conversation, compaction, permissions
│   ├── provider/            # LLM clients (Anthropic, OpenAI, Gemini, etc.)
│   ├── tools/               # Tool implementations (read, write, glob, grep, bash, fetch)
│   ├── tui/                 # Rendering (layout, chat, home, sidebar, footer, theme)
│   ├── config/              # Config loading, auth, preferences
│   ├── session/             # SQLite persistence
│   ├── mcp/                 # MCP client and server management
│   ├── memory/              # Cross-session fact storage and retrieval
│   ├── skills/              # Built-in + user skill system
│   ├── safety/              # Command policy, env filtering
│   └── init/                # Project initialization (/init)
└── Cargo.toml
```

## Development

```bash
cd tui
cargo build              # debug build
cargo build --release    # optimized release build
cargo test               # run all tests (~516)
cargo test -- --nocapture  # tests with output
```
