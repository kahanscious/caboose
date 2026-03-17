# Image & Screenshot Support Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add clipboard image paste and drag-and-drop image attachment so users can send images to vision-capable LLMs.

**Architecture:** Two new input paths (clipboard image read, paste-event path detection) feed into the existing `Attachment` → `ContentBlock::Image` → provider serialization pipeline. A small UI enhancement dims attachment pills when the model lacks vision support. All provider serialization and session persistence already works.

**Tech Stack:** Rust, arboard (clipboard), png (encoding), crossterm (paste events), ratatui (UI)

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `tui/Cargo.toml` | Modify | Add `png` crate dependency |
| `tui/src/clipboard.rs` | Modify | Add `read_image_from_clipboard()` |
| `tui/src/attachment.rs` | Modify | Add `attachment_from_rgba()`, `try_attach_pasted_images()` |
| `tui/src/app.rs` | Modify | Wire clipboard image paste into Ctrl+V handlers, replace single-line paste path detection with multi-line version |
| `tui/src/tui/layout.rs` | Modify | Dim attachment pills when model lacks vision |

---

## Chunk 1: Clipboard Image Reading

### Task 1: Add `png` crate dependency

**Files:**
- Modify: `tui/Cargo.toml`

- [ ] **Step 1: Add png to dependencies**

In `tui/Cargo.toml`, add under `[dependencies]`:

```toml
png = "0.17"
```

- [ ] **Step 2: Verify it compiles**

Run: `cd tui && cargo check`
Expected: compiles with no errors

- [ ] **Step 3: Commit**

```bash
git add tui/Cargo.toml
git commit -m "add png crate for clipboard image encoding"
```

---

### Task 2: Add `read_image_from_clipboard` to clipboard.rs

**Files:**
- Modify: `tui/src/clipboard.rs`
- Test: `tui/src/clipboard.rs` (inline tests)

- [ ] **Step 1: Write the failing test for clipboard image reading**

Add to the `tests` module in `tui/src/clipboard.rs`:

```rust
#[test]
fn read_image_returns_none_without_image() {
    // When clipboard has no image (or clipboard is unavailable),
    // the function should return None — not panic or error.
    let result = read_image_from_clipboard();
    // We can't control clipboard contents in CI, so just verify
    // it returns an Option without panicking.
    let _ = result;
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd tui && cargo test read_image_returns_none_without_image -- --nocapture`
Expected: FAIL — `read_image_from_clipboard` doesn't exist yet

- [ ] **Step 3: Implement `read_image_from_clipboard`**

Add to `tui/src/clipboard.rs` above the tests module:

```rust
/// Try to read an image from the system clipboard.
/// Returns `Some((rgba_bytes, width, height))` if the clipboard contains an image,
/// `None` if it contains text or is empty/inaccessible.
pub fn read_image_from_clipboard() -> Option<(Vec<u8>, usize, usize)> {
    let mut clipboard = Clipboard::new().ok()?;
    let img = clipboard.get_image().ok()?;
    // arboard::ImageData has width/height as usize and bytes as Cow<[u8]>
    Some((img.bytes.into_owned(), img.width, img.height))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd tui && cargo test read_image_returns_none_without_image -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add tui/src/clipboard.rs
git commit -m "add read_image_from_clipboard for clipboard image access"
```

---

### Task 3: Add `attachment_from_rgba` to attachment.rs

**Files:**
- Modify: `tui/src/attachment.rs`
- Test: `tui/src/attachment.rs` (inline tests)

- [ ] **Step 1: Write the failing test for RGBA → PNG encoding**

Add to the `tests` module in `tui/src/attachment.rs`:

```rust
#[test]
fn attachment_from_rgba_produces_valid_png() {
    // 2x2 red pixel image (RGBA)
    let rgba = vec![
        255, 0, 0, 255,  // red
        0, 255, 0, 255,  // green
        0, 0, 255, 255,  // blue
        255, 255, 0, 255, // yellow
    ];
    let att = attachment_from_rgba(rgba, 2, 2).unwrap();
    assert_eq!(att.media_type, "image/png");
    assert!(att.display_name.starts_with("clipboard-"));
    assert!(att.display_name.ends_with(".png"));
    // Verify PNG magic bytes
    assert_eq!(&att.data[..4], &[0x89, b'P', b'N', b'G']);
}

#[test]
fn attachment_from_rgba_rejects_oversized_dimensions() {
    // Width that would overflow u32
    let result = attachment_from_rgba(vec![], usize::MAX, 1);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("too large"));
}

#[test]
fn attachment_from_rgba_rejects_mismatched_data() {
    // 2x2 image needs 16 bytes of RGBA, but we give 4
    let result = attachment_from_rgba(vec![0, 0, 0, 0], 2, 2);
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tui && cargo test attachment_from_rgba -- --nocapture`
Expected: FAIL — `attachment_from_rgba` doesn't exist yet

- [ ] **Step 3: Implement `attachment_from_rgba`**

Add to `tui/src/attachment.rs` after the `read_image_attachment` function:

```rust
/// Create an Attachment from raw RGBA pixel data (e.g. from clipboard).
/// Encodes the data as PNG. Returns an error if dimensions are invalid.
pub fn attachment_from_rgba(
    rgba: Vec<u8>,
    width: usize,
    height: usize,
) -> Result<Attachment, String> {
    let w: u32 = width
        .try_into()
        .map_err(|_| format!("Image dimensions too large: {width}x{height}"))?;
    let h: u32 = height
        .try_into()
        .map_err(|_| format!("Image dimensions too large: {width}x{height}"))?;

    let expected_len = (width as u64) * (height as u64) * 4;
    if expected_len > MAX_IMAGE_SIZE {
        return Err(format!(
            "Image data too large: {} for {width}x{height}",
            format_size(expected_len as usize),
        ));
    }
    if rgba.len() != expected_len as usize {
        return Err(format!(
            "RGBA data length mismatch: expected {expected_len} bytes for {width}x{height}, got {}",
            rgba.len()
        ));
    }

    // Encode as PNG
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG header error: {e}"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|e| format!("PNG encode error: {e}"))?;
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let display_name = format!("clipboard-{timestamp}.png");

    Ok(Attachment {
        path: std::path::PathBuf::from(&display_name),
        media_type: "image/png".to_string(),
        data: png_bytes,
        display_name,
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd tui && cargo test attachment_from_rgba -- --nocapture`
Expected: all 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add tui/src/attachment.rs
git commit -m "add attachment_from_rgba for clipboard image encoding"
```

---

## Chunk 2: Paste Path Detection (Drag-and-Drop)

### Task 4: Add `try_attach_pasted_images` to attachment.rs

**Files:**
- Modify: `tui/src/attachment.rs`
- Test: `tui/src/attachment.rs` (inline tests)

- [ ] **Step 1: Write failing tests for paste path detection**

Add to the `tests` module in `tui/src/attachment.rs`:

```rust
#[test]
fn try_attach_single_image_path() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("photo.png");
    std::fs::write(&img, &[0x89, b'P', b'N', b'G']).unwrap();

    let (paths, remainder) = try_attach_pasted_images(img.to_str().unwrap());
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], img);
    assert!(remainder.is_empty());
}

#[test]
fn try_attach_multiple_image_paths() {
    let dir = tempfile::tempdir().unwrap();
    let img1 = dir.path().join("a.png");
    let img2 = dir.path().join("b.jpg");
    std::fs::write(&img1, &[0x89]).unwrap();
    std::fs::write(&img2, &[0xFF]).unwrap();

    let paste = format!("{}\n{}", img1.display(), img2.display());
    let (paths, remainder) = try_attach_pasted_images(&paste);
    assert_eq!(paths.len(), 2);
    assert!(remainder.is_empty());
}

#[test]
fn try_attach_mixed_paths_and_text() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("photo.png");
    std::fs::write(&img, &[0x89]).unwrap();

    let paste = format!("hello world\n{}\nsome other text", img.display());
    let (paths, remainder) = try_attach_pasted_images(&paste);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], img);
    assert_eq!(remainder, "hello world\nsome other text");
}

#[test]
fn try_attach_nonexistent_image_path_falls_through() {
    let (paths, remainder) = try_attach_pasted_images("/nonexistent/photo.png");
    assert!(paths.is_empty());
    assert_eq!(remainder, "/nonexistent/photo.png");
}

#[test]
fn try_attach_non_image_path_falls_through() {
    let dir = tempfile::tempdir().unwrap();
    let rs_file = dir.path().join("main.rs");
    std::fs::write(&rs_file, "fn main() {}").unwrap();

    let (paths, remainder) = try_attach_pasted_images(rs_file.to_str().unwrap());
    assert!(paths.is_empty());
    assert_eq!(remainder, rs_file.to_str().unwrap());
}

#[test]
fn try_attach_quoted_path() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("photo.png");
    std::fs::write(&img, &[0x89]).unwrap();

    let paste = format!("\"{}\"", img.display());
    let (paths, remainder) = try_attach_pasted_images(&paste);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], img);
    assert!(remainder.is_empty());
}

#[test]
fn try_attach_single_quoted_path() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("photo.png");
    std::fs::write(&img, &[0x89]).unwrap();

    let paste = format!("'{}'", img.display());
    let (paths, remainder) = try_attach_pasted_images(&paste);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], img);
    assert!(remainder.is_empty());
}

#[test]
fn try_attach_plain_text_unchanged() {
    let (paths, remainder) = try_attach_pasted_images("just some regular text");
    assert!(paths.is_empty());
    assert_eq!(remainder, "just some regular text");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd tui && cargo test try_attach -- --nocapture`
Expected: FAIL — `try_attach_pasted_images` doesn't exist yet

- [ ] **Step 3: Implement `try_attach_pasted_images`**

Add to `tui/src/attachment.rs` after `attachment_from_rgba`:

```rust
/// Detect image file paths in pasted text (e.g. from terminal drag-and-drop).
/// Returns (image_paths_found, remaining_text_to_insert).
pub fn try_attach_pasted_images(paste: &str) -> (Vec<PathBuf>, String) {
    let mut image_paths = Vec::new();
    let mut remaining_lines = Vec::new();

    for line in paste.lines() {
        let trimmed = line.trim();
        // Strip surrounding quotes (some terminals wrap dropped file paths)
        let unquoted = trimmed
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(trimmed);

        let path = Path::new(unquoted);
        if !unquoted.is_empty() && is_image_path(path) && path.exists() {
            image_paths.push(path.to_path_buf());
        } else {
            remaining_lines.push(line);
        }
    }

    let remainder = remaining_lines.join("\n");
    (image_paths, remainder)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd tui && cargo test try_attach -- --nocapture`
Expected: all 7 tests PASS

- [ ] **Step 5: Commit**

```bash
git add tui/src/attachment.rs
git commit -m "add try_attach_pasted_images for drag-and-drop path detection"
```

---

## Chunk 3: Wire Into App Event Handlers

### Task 5: Add clipboard image paste to Ctrl+V handlers

**Files:**
- Modify: `tui/src/app.rs:2948-2953` (chat Ctrl+V handler)
- Modify: `tui/src/app.rs:3627-3632` (home Ctrl+V handler)

- [ ] **Step 1: Modify the chat screen Ctrl+V handler (around line 2948)**

Replace the existing Ctrl+V handler:

```rust
// BEFORE (around line 2948):
(KeyCode::Char('v'), m) if m.contains(KeyModifiers::CONTROL) => {
    if let Ok(mut clipboard) = arboard::Clipboard::new()
        && let Ok(text) = clipboard.get_text()
    {
        self.handle_paste(&text);
    }
}
```

With:

```rust
(KeyCode::Char('v'), m) if m.contains(KeyModifiers::CONTROL) => {
    // Try clipboard image first, then fall back to text
    if let Some(att) = crate::clipboard::read_image_from_clipboard()
        .and_then(|(rgba, w, h)| crate::attachment::attachment_from_rgba(rgba, w, h).ok())
    {
        self.state.attachments.push(att);
    } else if let Ok(mut clipboard) = arboard::Clipboard::new()
        && let Ok(text) = clipboard.get_text()
    {
        self.handle_paste(&text);
    }
}
```

- [ ] **Step 2: Modify the home screen Ctrl+V handler (around line 3627)**

Apply the same change — replace the home screen's Ctrl+V handler with the identical image-first logic:

```rust
(KeyCode::Char('v'), m) if m.contains(KeyModifiers::CONTROL) => {
    // Try clipboard image first, then fall back to text
    if let Some(att) = crate::clipboard::read_image_from_clipboard()
        .and_then(|(rgba, w, h)| crate::attachment::attachment_from_rgba(rgba, w, h).ok())
    {
        self.state.attachments.push(att);
    } else if let Ok(mut clipboard) = arboard::Clipboard::new()
        && let Ok(text) = clipboard.get_text()
    {
        self.handle_paste(&text);
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd tui && cargo check`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add tui/src/app.rs
git commit -m "wire clipboard image paste into Ctrl+V handlers"
```

---

### Task 6: Replace single-line paste path detection with multi-line version

**Files:**
- Modify: `tui/src/app.rs:6134-6153` (handle_paste None branch)

- [ ] **Step 1: Replace the existing single-line image path detection in handle_paste**

In `handle_paste`, find the `None =>` branch (around line 6134). Replace the single-line image check:

```rust
// BEFORE (around line 6134-6153):
None => {
    // Check if paste is a single file path to an image — auto-attach
    let trimmed = text.trim();
    if !trimmed.is_empty()
        && !trimmed.contains('\n')
        && crate::attachment::is_image_path(std::path::Path::new(trimmed))
        && std::path::Path::new(trimmed).exists()
    {
        match crate::attachment::read_image_attachment(std::path::Path::new(trimmed)) {
            Ok(att) => {
                self.state.attachments.push(att);
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to attach: {e}"),
                });
            }
        }
        return;
    }

    // Base screen (Home or Chat) — paste into input with threshold check
```

With:

```rust
None => {
    // Check if paste contains image file paths (drag-and-drop or pasted paths)
    let (image_paths, remainder) =
        crate::attachment::try_attach_pasted_images(text);

    for path in &image_paths {
        match crate::attachment::read_image_attachment(path) {
            Ok(att) => {
                self.state.attachments.push(att);
            }
            Err(e) => {
                self.state.chat_messages.push(ChatMessage::Error {
                    content: format!("Failed to attach: {e}"),
                });
            }
        }
    }

    // If everything was image paths, we're done
    if remainder.is_empty() && !image_paths.is_empty() {
        return;
    }

    // Use remainder (non-image lines) as the paste text, avoiding variable shadowing
    let effective_text: &str = if image_paths.is_empty() {
        text // no images found, use original text
    } else {
        remainder.as_str()
    };

    // Base screen (Home or Chat) — paste into input with threshold check
    // NOTE: replace all uses of `text` below this point with `effective_text`
```

**Important:** The rest of the `None =>` branch (threshold check, `PasteConfirm` dialog, `input.push_str`) stays unchanged — it just operates on `text` which is now potentially the remainder.

- [ ] **Step 2: Verify it compiles**

Run: `cd tui && cargo check`
Expected: compiles with no errors

- [ ] **Step 3: Run all existing tests to check for regressions**

Run: `cd tui && cargo test`
Expected: all ~514+ tests PASS

- [ ] **Step 4: Commit**

```bash
git add tui/src/app.rs
git commit -m "replace single-line paste image detection with multi-line try_attach_pasted_images"
```

---

## Chunk 4: Non-Vision Model Warning on Attachment Pills

### Task 7: Dim attachment pills when model lacks vision

**Files:**
- Modify: `tui/src/tui/layout.rs:1297-1313`

- [ ] **Step 1: Modify the attachment chip rendering to check vision support**

The `render_input_area` function in `layout.rs` receives `app` which has `model_supports_vision`. Find the attachment chip rendering block (around line 1297):

```rust
// BEFORE:
// Render attachment chips
if let Some(att_area) = attach_area {
    let chips: Vec<Span> = app
        .attachments
        .iter()
        .flat_map(|att| {
            vec![
                Span::styled(
                    format!(" [image: {}] ", att.display_name),
                    Style::default().fg(colors.text).bg(colors.bg_elevated),
                ),
                Span::raw(" "),
            ]
        })
        .collect();
    let chip_line = Line::from(chips);
    frame.render_widget(Paragraph::new(chip_line), att_area);
}
```

Replace with:

```rust
// Render attachment chips (dimmed when model lacks vision)
if let Some(att_area) = attach_area {
    let style = if app.model_supports_vision {
        Style::default().fg(colors.text).bg(colors.bg_elevated)
    } else {
        Style::default()
            .fg(colors.text_muted)
            .bg(colors.bg_elevated)
            .add_modifier(ratatui::style::Modifier::DIM)
    };

    let mut chips: Vec<Span> = app
        .attachments
        .iter()
        .flat_map(|att| {
            vec![
                Span::styled(format!(" [image: {}] ", att.display_name), style),
                Span::raw(" "),
            ]
        })
        .collect();

    if !app.model_supports_vision && !app.attachments.is_empty() {
        chips.push(Span::styled(
            " (model doesn't support images) ",
            Style::default().fg(colors.error),
        ));
    }

    let chip_line = Line::from(chips);
    frame.render_widget(Paragraph::new(chip_line), att_area);
}
```

**Note:** `colors.text_muted` and `colors.error` are fields on the railroad theme's `Colors` struct (see `tui/src/tui/theme.rs`).

- [ ] **Step 2: Verify it compiles**

Run: `cd tui && cargo check`
Expected: compiles with no errors

- [ ] **Step 3: Run all tests**

Run: `cd tui && cargo test`
Expected: all tests PASS

- [ ] **Step 4: Commit**

```bash
git add tui/src/tui/layout.rs
git commit -m "dim attachment pills when model lacks vision support"
```

---

## Chunk 5: Final Verification

### Task 8: Full build and test sweep

- [ ] **Step 1: Run clippy**

Run: `cd tui && cargo clippy`
Expected: no warnings related to new code

- [ ] **Step 2: Run full test suite**

Run: `cd tui && cargo test`
Expected: all tests PASS (should be ~520+ now with the new tests)

- [ ] **Step 3: Run release build**

Run: `cd tui && cargo build --release`
Expected: compiles successfully

- [ ] **Step 4: Final commit if any cleanup was needed**

If clippy or tests required fixes, commit those fixes.
