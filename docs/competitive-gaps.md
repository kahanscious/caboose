# Competitive Gap Tracker

Feature gaps identified by comparing Caboose against Claude Code, Codex CLI, Aider, Cursor, Cline/Roo Code, and OpenCode. Updated 2026-03-17.

## Status Key

- **DONE** — shipped
- **WIP** — in progress
- **NEXT** — queued for implementation
- **DEFER** — acknowledged but not prioritized
- **SKIP** — decided against

---

## Tier 1 — High Impact, Core Gaps

| # | Feature | Status | Who Has It | Notes |
|---|---------|--------|-----------|-------|
| 1 | Web search tool | NEXT | Codex CLI, Aider | Built-in tool — Tender Highball MCP covers this but requires setup; native tool is better |
| 2 | Auto session titling | NEXT | OpenCode, Claude Code | Dedicated LLM call after first message to generate a short title |
| 3 | Git undo (`/undo`) | NEXT | Aider, Cline | Revert last git change (not conversation rewind) |
| 4 | More providers | NEXT | OpenCode (all 5) | Easy wins — same HTTP/SSE pattern as existing providers. See breakdown below |

### 4. Provider Expansion Status

| Provider | Status | Auth | Notes |
|----------|--------|------|-------|
| Anthropic | DONE | `ANTHROPIC_API_KEY` | Primary default provider |
| OpenAI | DONE | `OPENAI_API_KEY` | GPT-4o, o1, o3, etc. |
| Google Gemini | DONE | `GEMINI_API_KEY` | Direct API |
| OpenRouter | DONE | `OPENROUTER_API_KEY` | Dynamic model discovery via models.dev |
| Groq | DONE | `GROQ_API_KEY` | OpenAI-compatible API, fast inference |
| Mistral | DONE | `MISTRAL_API_KEY` | OpenAI-compatible API |
| DeepSeek | DONE | `DEEPSEEK_API_KEY` | OpenAI-compatible API |
| xAI (Grok) | DONE | `XAI_API_KEY` | OpenAI-compatible API |
| Together AI | DONE | `TOGETHER_API_KEY` | OpenAI-compatible API |
| Fireworks AI | DONE | `FIREWORKS_API_KEY` | OpenAI-compatible API |
| Cerebras | DONE | `CEREBRAS_API_KEY` | OpenAI-compatible API, fast inference |
| SambaNova | DONE | `SAMBANOVA_API_KEY` | OpenAI-compatible API, fast inference |
| Perplexity | DONE | `PERPLEXITY_API_KEY` | OpenAI-compatible API, search-augmented |
| Cohere | DONE | `COHERE_API_KEY` | OpenAI-compatible API |
| Qwen (DashScope) | DONE | `DASHSCOPE_API_KEY` | OpenAI-compatible API |
| AWS Bedrock | TODO | AWS SDK creds | Custom auth — Claude, Llama, Mistral models |
| Azure OpenAI | TODO | `AZURE_OPENAI_API_KEY` + endpoint | Custom auth — enterprise GPT deployments |
| Google Vertex AI | TODO | GCP service account | Custom auth — enterprise Gemini deployments |
| GitHub Copilot | TODO | `GITHUB_TOKEN` / VS Code token | Custom auth — routes to GPT, Claude, Gemini |

| # | Feature | Status | Who Has It | Notes |
|---|---------|--------|-----------|-------|
| 5 | SSE transport for MCP | NEXT | OpenCode | Connect to remote MCP servers over HTTP instead of stdio only |
| 6 | Non-interactive JSON output (`-f json`) | NEXT | OpenCode, Codex CLI | Structured output for CI/CD and scripting pipelines |
| 7 | `/status` command | DONE | Codex CLI, OpenCode | Renamed from `/usage`, shows provider, model, mode, tokens, cost |

## Tier 2 — Medium Impact, Differentiation

| # | Feature | Status | Who Has It | Notes |
|---|---------|--------|-----------|-------|
| 8 | Repo map / codebase indexing | DEFER | Aider (tree-sitter), Cursor (semantic index) | Auto-discover relevant files via AST graph ranking |
| 9 | Side-by-side diff view | DEFER | OpenCode | Character-level intra-line diff in TUI |
| 10 | Auto-commit option | DEFER | Aider | Automatic git commits after edits with conventional commit messages |
| 11 | Auto-test after edits | DEFER | Aider, Cline | Configurable test command runs after every file change |
| 12 | Custom command arguments | DEFER | OpenCode | `$PLACEHOLDER` templating in skills with argument prompts |
| 13 | Named config profiles | DEFER | Codex CLI | Switch between named configuration sets |
| 14 | Session/chat export | DEFER | Aider | Export conversation as markdown file |
| 15 | Ignore patterns file (`.cabooseignore`) | DEFER | Aider, Cursor | Project-level ignore for tool context |

## Tier 3 — Nice to Have / Long-term

| # | Feature | Status | Who Has It | Notes |
|---|---------|--------|-----------|-------|
| 16 | OS-level sandboxing | DEFER | Codex CLI | macOS sandbox / Linux landlock enforcement |
| 17 | Browser automation | SKIP | Cline, Roo Code | Not appropriate for terminal-native app |
| 18 | Voice input | DEFER | Aider, Cursor | Accessibility and hands-free coding |
| 19 | Sourcegraph integration | DEFER | OpenCode | Search public code repositories |
| 20 | `!` shell shortcut | DEFER | Codex CLI | Quick inline command execution from input |
| 21 | Architect mode | DEFER | Aider | Two-model split (architect proposes, editor implements) |

---

## What Caboose Leads On (No Gaps)

Features Caboose has that most competitors don't:

- **Roundhouse** — multi-model parallel planning with critique phase
- **Circuits** — recurring scheduled agent tasks
- **`/suggest`** — evidence-based codebase scanning with priority sorting
- **`/fork`** — session branching with parent tracking
- **`/pin`** — session-scoped rules injected into system prompt
- **Cold storage** — offload old conversation segments to SQLite
- **Embedded terminal panel** — PTY terminal within TUI
- **Sub-agents with worktree isolation** — parallel agents in git worktrees
- **`/watch`** — PR/MR status monitoring (GitHub/GitLab)
- **Skills system** — built-in + user-defined with LLM-guided creation
- **`/handoff`** — structured session summary generation
- **`/rewind`** — conversation checkpoint rollback

---

## Research Sources

Competitive analysis conducted 2026-03-17 covering:
- Claude Code (Anthropic CLI)
- Codex CLI (OpenAI)
- Aider (aider-chat)
- Cursor IDE
- Cline / Roo Code (VS Code extensions)
- OpenCode (opencode-ai, now archived as Crush)
