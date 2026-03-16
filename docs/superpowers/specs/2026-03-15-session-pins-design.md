# Session-Scoped Pins Design

## Overview

Session-scoped pins let users set temporary rules that the model follows for the current session. Pins are injected into the system prompt and persist with the session in SQLite, but are scoped — they don't affect other sessions or global behavior.

## Commands

- `/pin <text>` — add a pin (e.g., `/pin don't touch auth module`)
- `/pins` — list all pins with numbered indices in chat
- `/unpin` — clear all pins
- `/unpin <n>` — remove pin at 1-based index

## Data Model

Add a `pins TEXT` nullable column to the `sessions` table. Stores a JSON array of strings: `["don't touch auth module", "use snake_case"]`. NULL or `"[]"` means no pins.

`State` gets a `pins: Vec<String>` field. Loaded from session on restore, saved back on any `/pin` or `/unpin` change.

The `Session` struct gets a `pins: Vec<String>` field (deserialized from JSON on load, serialized on save).

## System Prompt Injection

When `pins` is non-empty, append a section to the system prompt (after skills/agent awareness, before conversation):

```
## Session Pins (user-set rules for this session)
1. don't touch auth module
2. use snake_case in this file
```

Built in the system prompt builder in `app.rs` alongside the existing CABOOSE.md, memory, skills, and agent context sections.

## Pin Bar UI

A collapsible bar rendered between the header row and the chat area in `layout.rs`. Only allocates space when pins exist.

**Collapsed (default):** 1 row — `▶ 2 pins` (clickable to expand)

**Expanded:** N+1 rows — header line + one line per pin with 1-based index:
```
▼ Pins
  1. don't touch auth module
  2. use snake_case in this file
```

Click toggles between collapsed/expanded. `State` tracks `pins_expanded: bool` (default false).

**Layout change:** The vertical constraint list in `render_chat_layout()` currently has `[header(1), chat(min 1), input(N)]`. When pins exist, insert `pin_bar(height)` between header and chat: `[header(1), pin_bar(1 or N+1), chat(min 1), input(N)]`. When no pins, the pin_bar constraint is omitted entirely.

## Session Restore and Fork

- **Restore:** `pins` loaded from JSON column via `get_session()`, populates `State.pins`
- **Fork:** pins are part of the session row, so `copy` in fork naturally includes them (the `INSERT ... SELECT` copies all columns including `pins`)

## Storage Operations

- `update_pins(session_id, pins: &[String])` — serializes pins to JSON, updates the `pins` column
- No new table needed — single column on existing `sessions` table

## Error Handling

- `/unpin <n>` with out-of-range index: show "Pin N does not exist. You have M pins." in chat
- `/pin` with empty text: show "Usage: /pin <text>"
- `/unpin` with no pins: show "No pins to remove."

## Testing

- JSON serialization: empty vec → `"[]"`, multiple items round-trip correctly
- `/pin` appends, `/unpin` clears all, `/unpin N` removes by index and re-indexes
- `/unpin 99` on 2 pins shows error
- `/pin` with empty text shows usage
- System prompt includes pins section when non-empty, omits when empty
- Session restore loads pins from JSON column
- Pin bar height calculation: 0 when no pins, 1 collapsed, N+1 expanded
- `update_pins` persists to DB correctly
