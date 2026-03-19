# Roundhouse v2 Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Roundhouse's chat-embedded output with a dedicated full-screen experience featuring model-switching, gated phases, and user annotations.

**Architecture:** Add a `Screen::Roundhouse` variant to the base screen enum. When planning starts, transition to a new screen rendered by `roundhouse_screen.rs` with a left panel (model output viewer) and right panel (navigator/status). Phase transitions pause at review gates where the user decides to proceed, annotate, or cancel. Annotations inject into subsequent phase prompts. On completion, write to `.caboose/roundhouse/` and transition back to Chat with the synthesis as a message.

**Tech Stack:** Rust, Ratatui, crossterm, tokio (existing streaming infrastructure unchanged)

---

## File Structure

| File | Responsibility |
|------|---------------|
| `tui/src/roundhouse/types.rs` | Add `ReviewingPlans`, `ReviewingCritiques` phases; remove `Executing` |
| `tui/src/roundhouse/session.rs` | Add `annotations`, `selected_model_index`, `viewer_scroll_offset`, `annotation_input` fields; annotation helpers |
| `tui/src/roundhouse/output.rs` | Update output path to `.caboose/roundhouse/`, include annotations in document |
| `tui/src/roundhouse/planner.rs` | Inject annotations into critique/synthesis prompts |
| `tui/src/tui/dialog.rs` | Add `Screen::Roundhouse` variant |
| `tui/src/tui/roundhouse_screen.rs` | **New** — full-screen renderer (left panel + right panel + gate bar) |
| `tui/src/tui/layout.rs` | Route `Screen::Roundhouse` to the new renderer |
| `tui/src/app.rs` | Roundhouse key handler for new screen, screen transitions, gate actions, annotation input |

---

### Task 1: Add new phases and session fields

**Files:**
- Modify: `tui/src/roundhouse/types.rs`
- Modify: `tui/src/roundhouse/session.rs`

- [ ] **Step 1: Update RoundhousePhase enum**

In `types.rs`, replace the phase enum with:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum RoundhousePhase {
    SelectingProviders,
    AwaitingPrompt,
    Planning,
    ReviewingPlans,
    Critiquing,
    ReviewingCritiques,
    Synthesizing,
    Complete,
    Cancelled,
}
```

Remove `Reviewing` and `Executing` — those are replaced by `ReviewingPlans`/`ReviewingCritiques` and execution happens from chat.

- [ ] **Step 2: Fix all compile errors from phase rename**

Search for `RoundhousePhase::Reviewing` and `RoundhousePhase::Executing` across the codebase. Update:
- `sidebar.rs`: `Reviewing` → `ReviewingPlans`, handle `ReviewingCritiques` similarly. Remove `Executing` arm.
- `layout.rs`: any match arms on roundhouse phases
- `app.rs`: all phase checks — this will be the bulk of changes. `Reviewing` → `ReviewingPlans`. Remove execution-from-roundhouse logic (will be re-added as chat-based).

Run: `cd tui && cargo check`
Expected: compiles with warnings about dead code (the new phases aren't used in transitions yet)

- [ ] **Step 3: Add session fields**

In `session.rs`, add to `RoundhouseSession`:

```rust
pub annotations: Vec<String>,
pub selected_model_index: usize, // 0 = primary, 1+ = secondaries
pub viewer_scroll_offset: u16,
pub annotation_input: Option<String>, // Some("") when input is active
```

Initialize all in `new()`:
```rust
annotations: Vec::new(),
selected_model_index: 0,
viewer_scroll_offset: 0,
annotation_input: None,
```

- [ ] **Step 4: Add annotation helper methods**

In `session.rs`, add:

```rust
pub fn add_annotation(&mut self, text: String) {
    if !text.trim().is_empty() {
        self.annotations.push(text.trim().to_string());
    }
}

/// Get the streaming text for the currently selected model
pub fn selected_model_text(&self) -> &str {
    if self.selected_model_index == 0 {
        &self.primary_streaming_text
    } else {
        self.secondaries
            .get(self.selected_model_index - 1)
            .map(|s| s.streaming_text.as_str())
            .unwrap_or("")
    }
}

/// Get the critique text for the currently selected model
pub fn selected_critique_text(&self) -> &str {
    if self.selected_model_index == 0 {
        &self.primary_critique_streaming_text
    } else {
        self.secondaries
            .get(self.selected_model_index - 1)
            .map(|s| s.critique_streaming_text.as_str())
            .unwrap_or("")
    }
}

/// Get display name for a model by index (0 = primary)
pub fn model_display_name(&self, index: usize) -> String {
    if index == 0 {
        self.primary_model.clone()
    } else {
        self.secondaries
            .get(index - 1)
            .map(|s| s.model_name.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

/// Total number of models (primary + secondaries)
pub fn model_count(&self) -> usize {
    1 + self.secondaries.len()
}

/// Navigate to next model
pub fn select_next_model(&mut self) {
    let count = self.model_count();
    if count > 0 {
        self.selected_model_index = (self.selected_model_index + 1) % count;
        self.viewer_scroll_offset = 0;
    }
}

/// Navigate to previous model
pub fn select_prev_model(&mut self) {
    let count = self.model_count();
    if count > 0 {
        self.selected_model_index = if self.selected_model_index == 0 {
            count - 1
        } else {
            self.selected_model_index - 1
        };
        self.viewer_scroll_offset = 0;
    }
}
```

- [ ] **Step 5: Add tests for new session methods**

```rust
#[test]
fn test_annotations() {
    let mut s = RoundhouseSession::new("anthropic".into(), "claude".into(), true, RoundhouseConfig::default());
    s.add_annotation("Use Claude's DB approach".into());
    s.add_annotation("   ".into()); // whitespace-only, should be ignored
    s.add_annotation("Ignore caching".into());
    assert_eq!(s.annotations.len(), 2);
    assert_eq!(s.annotations[0], "Use Claude's DB approach");
    assert_eq!(s.annotations[1], "Ignore caching");
}

#[test]
fn test_model_navigation() {
    let mut s = RoundhouseSession::new("anthropic".into(), "claude".into(), true, RoundhouseConfig::default());
    s.add_secondary("openai".into(), "gpt-4o".into());
    s.add_secondary("gemini".into(), "gemini-2.5".into());
    assert_eq!(s.selected_model_index, 0);
    assert_eq!(s.model_count(), 3);

    s.select_next_model();
    assert_eq!(s.selected_model_index, 1);
    s.select_next_model();
    assert_eq!(s.selected_model_index, 2);
    s.select_next_model();
    assert_eq!(s.selected_model_index, 0); // wraps

    s.select_prev_model();
    assert_eq!(s.selected_model_index, 2); // wraps back
}

#[test]
fn test_model_display_name() {
    let mut s = RoundhouseSession::new("anthropic".into(), "claude-sonnet".into(), true, RoundhouseConfig::default());
    s.add_secondary("openai".into(), "gpt-4o".into());
    assert_eq!(s.model_display_name(0), "claude-sonnet");
    assert_eq!(s.model_display_name(1), "gpt-4o");
    assert_eq!(s.model_display_name(99), "unknown");
}
```

- [ ] **Step 6: Run tests and commit**

Run: `cd tui && cargo test`
Expected: all tests pass

```bash
git add tui/src/roundhouse/types.rs tui/src/roundhouse/session.rs
git commit -m "add review gate phases, annotations, and model navigation to roundhouse"
```

---

### Task 2: Add Screen::Roundhouse and routing

**Files:**
- Modify: `tui/src/tui/dialog.rs`
- Modify: `tui/src/tui/layout.rs`
- Create: `tui/src/tui/roundhouse_screen.rs`

- [ ] **Step 1: Add Screen::Roundhouse variant**

In `dialog.rs`, add to the `Screen` enum:

```rust
pub enum Screen {
    Home,
    Chat,
    Roundhouse,
}
```

- [ ] **Step 2: Create roundhouse_screen.rs with placeholder render**

Create `tui/src/tui/roundhouse_screen.rs`:

```rust
//! Dedicated Roundhouse screen — model viewer + navigator + gate bar.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::State;
use crate::tui::theme;

/// Render the full Roundhouse screen.
pub fn render(frame: &mut Frame, state: &State) {
    let colors = theme::Colors::default();
    let area = frame.area();

    // Placeholder: just show "Roundhouse" centered
    let text = Paragraph::new("Roundhouse v2")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(colors.roundhouse)));
    frame.render_widget(text, area);
}
```

- [ ] **Step 3: Register the module**

Add `pub mod roundhouse_screen;` to `tui/src/tui/mod.rs`.

- [ ] **Step 4: Route Screen::Roundhouse in layout.rs**

In `layout.rs` `render()` function, add the new match arm:

```rust
match app.dialog_stack.base {
    Screen::Home => {
        crate::tui::home::render(frame, app);
    }
    Screen::Chat => {
        render_chat_layout(frame, app, &colors);
    }
    Screen::Roundhouse => {
        crate::tui::roundhouse_screen::render(frame, app);
    }
}
```

- [ ] **Step 5: Fix any remaining compile errors from Screen enum change**

The `Screen` enum is used in `available` closures in `command.rs` and possibly key handlers. Search for `Screen::Home` and `Screen::Chat` pattern matches that don't have a wildcard — add `Screen::Roundhouse` handling where needed. Most `available` checks for chat commands should return false for Roundhouse.

Run: `cd tui && cargo check`
Expected: compiles clean

- [ ] **Step 6: Commit**

```bash
git add tui/src/tui/dialog.rs tui/src/tui/layout.rs tui/src/tui/roundhouse_screen.rs tui/src/tui/mod.rs
git commit -m "add Screen::Roundhouse variant and placeholder renderer"
```

---

### Task 3: Build the Roundhouse screen renderer

**Files:**
- Modify: `tui/src/tui/roundhouse_screen.rs`

This is the core visual work. The screen has three zones:
- **Left panel (65%)** — selected model's streaming output with markdown/syntax highlighting
- **Right panel (35%)** — phase indicator, model list with status, cost, keybind hints
- **Bottom bar** — gate actions when in ReviewingPlans or ReviewingCritiques phase, or annotation input

- [ ] **Step 1: Implement the layout split**

Replace the placeholder `render` with the real layout:

```rust
pub fn render(frame: &mut Frame, state: &State) {
    let colors = theme::Colors::default();
    let area = frame.area();

    let rh = match &state.roundhouse_session {
        Some(rh) => rh,
        None => return, // shouldn't happen but be safe
    };

    // Reserve bottom bar for gate actions or annotation input
    let has_bottom_bar = matches!(
        rh.phase,
        RoundhousePhase::ReviewingPlans | RoundhousePhase::ReviewingCritiques
    ) || rh.annotation_input.is_some();

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_bottom_bar {
            vec![Constraint::Min(1), Constraint::Length(3)]
        } else {
            vec![Constraint::Min(1), Constraint::Length(0)]
        })
        .split(area);

    let main_area = vertical[0];
    let bottom_area = vertical[1];

    // Split main area into left (65%) and right (35%)
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(main_area);

    render_model_viewer(frame, state, rh, horizontal[0], &colors);
    render_navigator(frame, state, rh, horizontal[1], &colors);

    if has_bottom_bar {
        render_bottom_bar(frame, rh, bottom_area, &colors);
    }
}
```

- [ ] **Step 2: Implement render_model_viewer**

The left panel shows the selected model's streaming text. Use the existing markdown/syntax highlighting renderer if available, or fall back to raw Paragraph with wrapping.

```rust
fn render_model_viewer(
    frame: &mut Frame,
    _state: &State,
    rh: &RoundhouseSession,
    area: Rect,
    colors: &theme::Colors,
) {
    let model_name = rh.model_display_name(rh.selected_model_index);
    let status = if rh.selected_model_index == 0 {
        &rh.primary_status
    } else {
        &rh.secondaries[rh.selected_model_index - 1].status
    };
    let status_text = format_status(status);

    let header = format!(" {} — {}", model_name, status_text);

    // Choose content based on phase
    let content = match rh.phase {
        RoundhousePhase::Planning | RoundhousePhase::ReviewingPlans => {
            rh.selected_model_text()
        }
        RoundhousePhase::Critiquing | RoundhousePhase::ReviewingCritiques => {
            rh.selected_critique_text()
        }
        RoundhousePhase::Synthesizing => {
            &rh.synthesis_streaming_text
        }
        _ => rh.selected_model_text(),
    };

    let paragraph = Paragraph::new(content.to_string())
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((rh.viewer_scroll_offset, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors.roundhouse))
                .title(header)
                .title_style(Style::default().fg(colors.roundhouse).bold()),
        );
    frame.render_widget(paragraph, area);
}

fn format_status(status: &PlannerStatus) -> &'static str {
    match status {
        PlannerStatus::Pending => "pending",
        PlannerStatus::Thinking => "thinking",
        PlannerStatus::Streaming => "streaming",
        PlannerStatus::UsingTool(_) => "using tool",
        PlannerStatus::Done => "done",
        PlannerStatus::Failed(_) => "failed",
        PlannerStatus::TimedOut => "timed out",
    }
}
```

- [ ] **Step 3: Implement render_navigator**

The right panel shows phase, model list, cost, and hints.

```rust
fn render_navigator(
    frame: &mut Frame,
    _state: &State,
    rh: &RoundhouseSession,
    area: Rect,
    colors: &theme::Colors,
) {
    let mut lines: Vec<Line> = Vec::new();

    // Phase header
    let phase_name = match rh.phase {
        RoundhousePhase::Planning => "Planning",
        RoundhousePhase::ReviewingPlans => "Review Plans",
        RoundhousePhase::Critiquing => "Critiquing",
        RoundhousePhase::ReviewingCritiques => "Review Critiques",
        RoundhousePhase::Synthesizing => "Synthesizing",
        RoundhousePhase::Complete => "Complete",
        RoundhousePhase::Cancelled => "Cancelled",
        _ => "Roundhouse",
    };
    lines.push(Line::from(Span::styled(
        format!("  {phase_name}"),
        Style::default().fg(colors.roundhouse).bold(),
    )));
    lines.push(Line::from(""));

    // Model list
    let show_critique = matches!(
        rh.phase,
        RoundhousePhase::Critiquing | RoundhousePhase::ReviewingCritiques
    );

    // Primary
    let selected_marker = if rh.selected_model_index == 0 { "▶" } else { " " };
    let (icon, status_color) = status_icon_color(
        if show_critique { &rh.primary_critique_status } else { &rh.primary_status },
        colors,
    );
    let name = truncate(&rh.primary_model, 18);
    lines.push(Line::from(vec![
        Span::styled(format!("  {selected_marker} "), Style::default().fg(colors.roundhouse)),
        Span::styled(format!("{icon} "), Style::default().fg(status_color)),
        Span::styled(name, Style::default().fg(colors.text_secondary)),
    ]));

    // Secondaries
    for (i, sec) in rh.secondaries.iter().enumerate() {
        let marker = if rh.selected_model_index == i + 1 { "▶" } else { " " };
        let status = if show_critique { &sec.critique_status } else { &sec.status };
        let (icon, color) = status_icon_color(status, colors);
        let name = truncate(&sec.model_name, 18);
        lines.push(Line::from(vec![
            Span::styled(format!("  {marker} "), Style::default().fg(colors.roundhouse)),
            Span::styled(format!("{icon} "), Style::default().fg(color)),
            Span::styled(name, Style::default().fg(colors.text_secondary)),
        ]));
    }

    // Cost
    if rh.total_cost > 0.0 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ${:.4}", rh.total_cost),
            Style::default().fg(colors.text_muted),
        )));
    }

    // Annotations count
    if !rh.annotations.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {} annotation{}", rh.annotations.len(), if rh.annotations.len() == 1 { "" } else { "s" }),
            Style::default().fg(colors.text_muted),
        )));
    }

    // Keybind hints at bottom
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  j/k  switch model",
        Style::default().fg(colors.text_dim),
    )));
    lines.push(Line::from(Span::styled(
        "  ↑/↓  scroll output",
        Style::default().fg(colors.text_dim),
    )));

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.border)),
    );
    frame.render_widget(paragraph, area);
}

fn status_icon_color(status: &PlannerStatus, colors: &theme::Colors) -> (&'static str, Color) {
    match status {
        PlannerStatus::Pending => ("○", colors.text_dim),
        PlannerStatus::Thinking => ("◐", colors.warning),
        PlannerStatus::Streaming => ("●", colors.roundhouse),
        PlannerStatus::UsingTool(_) => ("⚙", colors.warning),
        PlannerStatus::Done => ("✓", colors.success),
        PlannerStatus::Failed(_) => ("✗", colors.error),
        PlannerStatus::TimedOut => ("⏱", colors.warning),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
```

- [ ] **Step 4: Implement render_bottom_bar**

Shows gate actions during review phases, or annotation input when active.

```rust
fn render_bottom_bar(
    frame: &mut Frame,
    rh: &RoundhouseSession,
    area: Rect,
    colors: &theme::Colors,
) {
    if let Some(ref input) = rh.annotation_input {
        // Annotation input mode
        let text = format!("  annotation: {input}█");
        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(colors.roundhouse)),
        );
        frame.render_widget(paragraph, area);
        return;
    }

    let hint_text = match rh.phase {
        RoundhousePhase::ReviewingPlans => {
            if rh.critique_enabled {
                "  [c] critique  [s] skip to synthesis  [a] annotate  [q] cancel"
            } else {
                "  [s] synthesize  [a] annotate  [q] cancel"
            }
        }
        RoundhousePhase::ReviewingCritiques => {
            "  [s] synthesize  [a] annotate  [q] cancel"
        }
        _ => "",
    };

    let paragraph = Paragraph::new(hint_text)
        .style(Style::default().fg(colors.text_secondary))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(colors.roundhouse)),
        );
    frame.render_widget(paragraph, area);
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cd tui && cargo check`
Expected: compiles (the screen won't be reachable yet but the code is valid)

- [ ] **Step 6: Commit**

```bash
git add tui/src/tui/roundhouse_screen.rs
git commit -m "build roundhouse v2 screen renderer with model viewer, navigator, and gate bar"
```

---

### Task 4: Wire screen transitions and key handling

**Files:**
- Modify: `tui/src/app.rs`

This is the integration task — connecting the new screen to the app loop.

- [ ] **Step 1: Transition to Roundhouse screen when planning starts**

Find where `start_roundhouse_planning` is called (after user enters the prompt in `AwaitingPrompt` phase). After calling it, add:

```rust
self.state.dialog_stack.base = Screen::Roundhouse;
```

- [ ] **Step 2: Transition to ReviewingPlans when all planners done**

In the `tick()` method, find the block that checks `session.all_planners_done()` and currently transitions to `Critiquing`. Change it to transition to `ReviewingPlans` instead:

```rust
if session.all_planners_done() {
    session.phase = RoundhousePhase::ReviewingPlans;
}
```

- [ ] **Step 3: Transition to ReviewingCritiques when all critiques done**

Similarly, find where `all_critiques_done()` triggers synthesis. Change to:

```rust
if session.all_critiques_done() {
    session.phase = RoundhousePhase::ReviewingCritiques;
}
```

- [ ] **Step 4: Transition back to Chat on completion**

When synthesis completes (synthesis channel closes), set:

```rust
session.phase = RoundhousePhase::Complete;
self.state.dialog_stack.base = Screen::Chat;
// Insert synthesis as assistant message in chat
self.state.chat_messages.push(ChatMessage::Assistant {
    content: session.synthesized_plan.clone().unwrap_or_default(),
    thinking: None,
});
```

- [ ] **Step 5: Add Roundhouse screen key handler**

Add a new method `handle_roundhouse_key` that handles keys when on the Roundhouse screen:

```rust
fn handle_roundhouse_key(&mut self, key: KeyEvent) -> bool {
    if key.kind != KeyEventKind::Press {
        return false;
    }

    let session = match &mut self.state.roundhouse_session {
        Some(s) => s,
        None => return false,
    };

    // Annotation input mode
    if session.annotation_input.is_some() {
        match key.code {
            KeyCode::Enter => {
                if let Some(text) = session.annotation_input.take() {
                    session.add_annotation(text);
                }
            }
            KeyCode::Esc => {
                session.annotation_input = None;
            }
            KeyCode::Backspace => {
                if let Some(ref mut input) = session.annotation_input {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(ref mut input) = session.annotation_input {
                    input.push(c);
                }
            }
            _ => {}
        }
        return true;
    }

    match key.code {
        // Model navigation
        KeyCode::Char('j') | KeyCode::Down => {
            session.select_next_model();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            session.select_prev_model();
        }

        // Scroll output
        KeyCode::PageDown => {
            session.viewer_scroll_offset = session.viewer_scroll_offset.saturating_add(10);
        }
        KeyCode::PageUp => {
            session.viewer_scroll_offset = session.viewer_scroll_offset.saturating_sub(10);
        }

        // Gate actions
        KeyCode::Char('c') if session.phase == RoundhousePhase::ReviewingPlans && session.critique_enabled => {
            session.phase = RoundhousePhase::Critiquing;
            self.start_roundhouse_critique();
        }
        KeyCode::Char('s') if matches!(session.phase, RoundhousePhase::ReviewingPlans | RoundhousePhase::ReviewingCritiques) => {
            session.phase = RoundhousePhase::Synthesizing;
            self.start_roundhouse_synthesis();
        }
        KeyCode::Char('a') if matches!(session.phase, RoundhousePhase::ReviewingPlans | RoundhousePhase::ReviewingCritiques) => {
            session.annotation_input = Some(String::new());
        }
        KeyCode::Char('q') => {
            session.phase = RoundhousePhase::Cancelled;
            self.state.dialog_stack.base = Screen::Chat;
        }

        _ => {}
    }
    true
}
```

- [ ] **Step 6: Route key events to the new handler**

In the main key event dispatch (likely in `handle_key_event` or similar), add a check before the chat handler:

```rust
if self.state.dialog_stack.base == Screen::Roundhouse {
    return self.handle_roundhouse_key(key);
}
```

- [ ] **Step 7: Verify it compiles and runs**

Run: `cd tui && cargo check`
Expected: compiles clean

Run: `cd tui && cargo test`
Expected: all tests pass

- [ ] **Step 8: Commit**

```bash
git add tui/src/app.rs
git commit -m "wire roundhouse screen transitions, key handling, and gate actions"
```

---

### Task 5: Inject annotations into prompts

**Files:**
- Modify: `tui/src/roundhouse/planner.rs`
- Modify: `tui/src/roundhouse/output.rs`

- [ ] **Step 1: Update critique_system_prompt to accept annotations**

```rust
pub fn critique_system_prompt(
    user_prompt: &str,
    own_provider: &str,
    all_plans: &[(&str, &str)],
    annotations: &[String],
) -> String {
    let mut prompt = /* existing prompt building */;

    if !annotations.is_empty() {
        prompt.push_str("--- User Guidance ---\n");
        prompt.push_str("The user has provided the following guidance. Respect these directives above your own judgment:\n\n");
        for annotation in annotations {
            prompt.push_str(&format!("- {annotation}\n"));
        }
        prompt.push('\n');
    }

    prompt
}
```

- [ ] **Step 2: Update synthesis_system_prompt to accept annotations**

Same pattern — add `annotations: &[String]` parameter, append the user guidance section.

- [ ] **Step 3: Update call sites in app.rs**

Pass `session.annotations` (as `&session.annotations`) to the prompt builders in `start_roundhouse_critique` and `start_roundhouse_synthesis`.

- [ ] **Step 4: Update tests**

Update existing tests that call `critique_system_prompt` and `synthesis_system_prompt` to pass `&[]` for annotations. Add new tests:

```rust
#[test]
fn test_critique_prompt_with_annotations() {
    let plans = vec![("openai", "Plan A")];
    let annotations = vec!["Use Claude's DB approach".to_string()];
    let prompt = critique_system_prompt("build auth", "gemini", &plans, &annotations);
    assert!(prompt.contains("User Guidance"));
    assert!(prompt.contains("Use Claude's DB approach"));
}

#[test]
fn test_synthesis_prompt_with_annotations() {
    let plans = vec![("openai", "Plan A")];
    let annotations = vec!["Ignore caching".to_string()];
    let prompt = synthesis_system_prompt("build auth", &plans, None, &annotations);
    assert!(prompt.contains("User Guidance"));
    assert!(prompt.contains("Ignore caching"));
}
```

- [ ] **Step 5: Update output.rs — include annotations in document**

Update `format_plans_document` to accept `annotations: &[String]` and include them:

```rust
if !annotations.is_empty() {
    doc.push_str("---\n\n");
    doc.push_str("## User Annotations\n\n");
    for annotation in annotations {
        doc.push_str(&format!("- {annotation}\n"));
    }
    doc.push('\n');
}
```

- [ ] **Step 6: Update output path to `.caboose/roundhouse/`**

In `write_plan_file`, change the output directory:

```rust
let dir = cwd.join(".caboose").join("roundhouse");
std::fs::create_dir_all(&dir)?;
let filename = format!("{}-{slug}.md", chrono::Local::now().format("%Y-%m-%d"));
let path = dir.join(&filename);
```

- [ ] **Step 7: Update the call site for write_plan_file in app.rs**

Pass annotations to `format_plans_document`. Update the call to `write_plan_file` if the signature changed.

- [ ] **Step 8: Run tests and commit**

Run: `cd tui && cargo test`
Expected: all tests pass

```bash
git add tui/src/roundhouse/planner.rs tui/src/roundhouse/output.rs tui/src/app.rs
git commit -m "inject user annotations into critique and synthesis prompts"
```

---

### Task 6: Integration testing and polish

**Files:**
- Various — cleanup pass

- [ ] **Step 1: Remove old chat-embedded roundhouse rendering from layout.rs**

The old roundhouse output was rendered inline in the chat area (`layout.rs` lines 633-780 approximately). With the dedicated screen, this code should be removed or guarded with a `Screen::Chat` check so it doesn't render when on the Roundhouse screen.

- [ ] **Step 2: Update sidebar roundhouse section**

The sidebar still renders roundhouse status during the Roundhouse screen. Update `sidebar.rs` to:
- Show simplified status when on `Screen::Roundhouse` (the navigator panel handles detailed status)
- Show full status when on `Screen::Chat` (after completion, the sidebar shows the result)

- [ ] **Step 3: Update `/roundhouse` subcommands**

In `handle_roundhouse_subcommand`:
- Remove `/roundhouse execute` (execution happens from chat naturally)
- Keep `/roundhouse cancel` and `/roundhouse clear`
- When `/roundhouse clear` is called, ensure we transition back to Chat if on Roundhouse screen

- [ ] **Step 4: Full test run**

Run: `cd tui && cargo test`
Expected: all tests pass

Run: `cd tui && cargo clippy`
Expected: no warnings

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "polish roundhouse v2: remove old inline rendering, update sidebar and subcommands"
```

- [ ] **Step 6: Bump version to 0.6.0**

In `tui/Cargo.toml`, update `version = "0.6.0"`.

```bash
git add tui/Cargo.toml
git commit -m "bump version to 0.6.0"
```
