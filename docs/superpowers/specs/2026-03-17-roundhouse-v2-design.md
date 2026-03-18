# Roundhouse v2 — Design Spec

**Feature:** Dedicated Roundhouse screen with model viewer, gated phases, and annotations
**Date:** 2026-03-17
**Branch:** feat/0.6.0

## Problem

Roundhouse works but feels opaque. You can't see what each model is doing — only the primary streams in chat while secondaries are status dots in the sidebar. The pipeline runs automatically with no pause between phases. The result is low visibility, no user control, and no confidence that the final synthesis captured the best ideas.

## Solution

A dedicated full-screen Roundhouse experience with:
1. A model viewer that lets you watch any model's output live
2. Review gates between phases where you decide what happens next
3. Annotations that let you steer the synthesis with your judgment

## Screen Layout

Roundhouse gets its own screen (alongside Home and Chat). When planning begins, the app transitions to this screen.

### Left Panel (65% width) — Model Output Viewer

- Shows the currently-selected model's full streaming output
- Markdown rendering with syntax highlighting (same renderer as chat)
- Scrollable (Page Up/Down, mouse scroll)
- Header: model name, provider, and status
- During synthesis phase: shows the primary model's synthesis stream

### Right Panel (35% width) — Navigator

From top to bottom:
- **Phase indicator** — current phase name with animated dots when active
- **Model list** — each model as a row with:
  - `▶` marker on the currently-viewed model
  - Status icon (spinning for streaming, checkmark for done, X for failed)
  - Model name (truncated to fit)
  - Brief status text ("streaming", "reading src/main.rs", "done")
- **Cost** — running total for the session
- **Keybind hints** — contextual, changes per phase

### Navigation

- `j` / `k` or `Up` / `Down` — switch between models in the navigator
- `Page Up` / `Page Down` — scroll the model output viewer
- Phase-specific keybinds shown at bottom of navigator

## Phase Flow

```
/roundhouse → Provider Picker → Prompt Input → Planning → [Gate] → Critique → [Gate] → Synthesis → Chat
```

### Phase 1: Planning

All models plan in parallel with read-only tools (read_file, glob, grep, list_directory). User can switch between models in real-time to watch any of them work.

When all models complete (or fail/timeout), the review gate activates.

### Review Gate (after Planning)

Bottom bar appears:

```
All plans ready.  [c] critique  [s] skip to synthesis  [a] annotate  [q] cancel
```

- **`c`** — proceed to critique phase
- **`s`** — skip critique, go straight to synthesis
- **`a`** — open annotation input (see Annotations below)
- **`q`** — cancel roundhouse, return to chat

User can scroll through each model's completed plan using j/k before deciding.

### Phase 2: Critique

All models critique each other's plans in parallel (each model sees all plans except its own — unchanged from v1). No tools available during critique.

When all critiques complete, the review gate activates.

### Review Gate (after Critique)

```
All critiques ready.  [s] synthesize  [a] annotate  [q] cancel
```

User can review each model's critique, add annotations, then proceed.

### Phase 3: Synthesis

Primary model synthesizes all plans + critiques + user annotations into one unified plan. Streams in the left panel. No model switching during synthesis (only one model is active).

When synthesis completes, the app transitions back to Chat.

## Annotations

At any review gate, pressing `a` opens a text input bar at the bottom of the Roundhouse screen (similar to the chat input, but contextual).

The user types guidance, e.g.:
- "Claude's database migration approach is better, use that"
- "Ignore Gemini's suggestion to add caching, it's premature"
- "All plans missed error handling for the webhook endpoint"

Pressing Enter saves the annotation. Pressing Escape cancels. Multiple annotations can be added (press `a` again).

Annotations are injected into the next phase's system prompt as a clearly-marked section:

```
--- User Guidance ---
Claude's database migration approach is better, use that.
All plans missed error handling for the webhook endpoint.
```

The prompt instructs the model to respect user guidance above its own judgment.

Annotations are stored on the RoundhouseSession and included in the output file.

## Output

When synthesis completes:

1. **File written** to `.caboose/roundhouse/<YYYY-MM-DD>-<title-slug>.md`
   - Contains: prompt, all individual plans (labeled by model), all critiques (labeled by model), user annotations (if any), final synthesis
   - Created automatically, no user action needed

2. **Chat transition** — app switches back to Chat screen with the synthesis inserted as an Assistant message. User can continue conversing, execute the plan, or start a new roundhouse.

3. **Sidebar update** — Roundhouse section shows "Complete" with link to the output file and `/roundhouse clear` hint.

## Data Model Changes

### RoundhousePhase

Add `ReviewingPlans` and `ReviewingCritiques` phases between existing phases:

```
SelectingProviders → AwaitingPrompt → Planning → ReviewingPlans → Critiquing → ReviewingCritiques → Synthesizing → Complete
```

Remove `Reviewing` (replaced by the two specific review phases) and `Executing` (execution happens from chat now).

### RoundhouseSession

Add:
- `annotations: Vec<String>` — user guidance collected at review gates
- `selected_model_index: usize` — which model is shown in the left panel (0 = primary)
- `viewer_scroll_offset: u16` — scroll position in the model output viewer

### Per-Model Streaming Text

Already stored: `primary_streaming_text` and `secondary.streaming_text`. These become the content shown in the left panel when that model is selected.

## Screen Integration

### Screen Enum

Add `Roundhouse` variant to the `Screen` enum in `dialog.rs`. The base screen can be Home, Chat, or Roundhouse.

### Transition Points

- **Chat → Roundhouse**: when planning starts (after prompt is entered)
- **Roundhouse → Chat**: when synthesis completes or user cancels

### Input Handling

When on the Roundhouse screen, key events route to a dedicated `handle_roundhouse_screen_key()` handler instead of the chat handler. This handles:
- j/k for model switching
- Page Up/Down for scrolling
- Phase gate keybinds (c/s/a/q)
- Annotation input mode

## What Stays the Same

- Provider picker dialog (existing, works fine)
- Parallel tokio streaming architecture (no changes to planner engine)
- Read-only tools during planning
- No tools during critique
- Primary model does synthesis
- System/critique/synthesis prompt construction (planner.rs)
- PlannerUpdate event types and mpsc channels
- Config: planning_timeout, critique_timeout, critique_enabled

## Touchpoints

### New Files
- `tui/src/tui/roundhouse_screen.rs` — dedicated screen rendering (left panel + right panel)

### Modified Files
- `tui/src/tui/dialog.rs` — add `Screen::Roundhouse` variant
- `tui/src/tui/layout.rs` — route to roundhouse screen renderer when on Roundhouse screen
- `tui/src/app.rs` — roundhouse key handler, screen transitions, annotation input mode
- `tui/src/roundhouse/types.rs` — new phases, annotation field, selected_model_index
- `tui/src/roundhouse/session.rs` — annotation management, model selection
- `tui/src/roundhouse/output.rs` — include annotations in output file, write to `.caboose/roundhouse/`
- `tui/src/roundhouse/planner.rs` — inject annotations into critique/synthesis prompts

## Not In Scope

- Execution from within Roundhouse screen (happens from chat after transition)
- Cherry-picking specific sections from plans (full annotation guidance is sufficient)
- Saving/loading roundhouse pipelines
- Custom phase sequences (generalized rounds architecture — future)
