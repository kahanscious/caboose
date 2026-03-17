# image & screenshot support — design spec

**target:** 0.4.1
**scope:** user-provided image input via clipboard paste, drag-and-drop (path detection), and existing `@file` references. no agent-initiated screenshots.

---

## summary

caboose already has full image plumbing — `ContentBlock::Image`, provider serialization for anthropic/openai/gemini/openrouter, `@file` image references, vision model detection, and session persistence. what's missing is the **input layer**: getting images from the user's clipboard or drag-and-drop into the attachment queue.

this spec covers two new input methods and a small UI enhancement:

1. clipboard image paste (Cmd+V / Ctrl+V)
2. drag-and-drop via pasted file path detection
3. non-vision model warning on existing attachment pills

---

## 1. clipboard image paste

### what changes

extend `clipboard.rs` to read images from the system clipboard via `arboard::Clipboard::get_image()`. convert raw RGBA pixel data to PNG bytes, wrap as an `Attachment`.

### flow

1. user presses Cmd+V / Ctrl+V
2. key handler in `app.rs` checks clipboard for image first, then text
3. if image found:
   - encode RGBA data as PNG via the `png` crate
   - create `Attachment` with `media_type: "image/png"`, `display_name: "clipboard-{unix_timestamp}.png"`
   - push to `self.state.attachments`
4. if no image → fall through to existing text paste behavior
5. attachment indicator appears in input area

### clipboard fallback behavior

`arboard::get_image()` returns `Err` for both "clipboard has text" and "clipboard not accessible". the handler must:

- try `get_image()` first
- on `Err`, fall through to `get_text()` (not bail)
- only fail silently if both return `Err`

this ensures text paste still works when `get_image()` fails for non-image reasons.

### new dependency

add the `png` crate for encoding raw RGBA → PNG. lightweight (~50KB), transitive deps are `miniz_oxide`, `adler`, `crc32fast` — all already pulled in via `flate2`. avoid the full `image` crate — it's heavy and we only need encoding.

### `Attachment.path` for clipboard images

`Attachment` has a required `path: PathBuf` field. clipboard images have no real file path. use a synthetic path: `PathBuf::from(format!("clipboard-{}.png", unix_timestamp))`. this keeps the struct non-optional and gives a meaningful `display_name`.

### files touched

- `tui/src/clipboard.rs` — add `read_image_from_clipboard() -> Option<(Vec<u8>, usize, usize)>` (raw RGBA + dimensions as `usize` to match `arboard::ImageData`)
- `tui/src/attachment.rs` — add `attachment_from_rgba(data: Vec<u8>, width: usize, height: usize) -> Result<Attachment>` (checked cast to `u32` for PNG encoder, PNG encode, wrap)
- `tui/src/app.rs` — modify Ctrl+V / Cmd+V handler: replace direct `get_text()` with image-first probe, then text fallback
- `Cargo.toml` — add `png` crate

---

## 2. drag-and-drop via path detection

### what changes

when a user drags a file onto the terminal, most modern terminals (iTerm2, Kitty, WezTerm, Ghostty, Alacritty) paste the file's absolute path as text via bracketed paste. detect image paths in paste events and auto-attach.

### flow

1. crossterm surfaces bracketed paste as `Event::Paste(String)`
2. on paste event, check if content is one or more lines each matching an existing file path with an image extension
3. for each image path → `read_image_attachment()` → push to `self.state.attachments`
4. any non-image lines → insert as text into input buffer
5. if all lines are images → don't insert any text

### detection heuristic

```rust
fn try_attach_pasted_images(paste: &str) -> (Vec<PathBuf>, String)
```

- split on newlines
- for each line, trim whitespace and quotes (some terminals wrap paths in quotes)
- check `is_image_path()` AND `path.exists()`
- return matched image paths + remaining text

### edge cases

- **path with spaces** — handled; we check the whole trimmed line
- **non-existent path** — falls through to text insertion
- **file too large (>20MB)** — `read_image_attachment()` already rejects with error; surface as flash message in footer
- **multiple files** — supported; newline-separated paths are split and each checked
- **user intentionally pasting a path as text** — unlikely for image file paths; they can type manually if needed

### migration note

`app.rs` already has a single-line paste path handler (around line 6136). the new multi-line `try_attach_pasted_images()` replaces it — the old single-line logic must be removed or it will short-circuit before the new multi-line logic runs.

### edge cases (additional)

- **file exists but not readable (permissions)** — `read_image_attachment()` returns an error; surface as flash message in footer, do not silently fall through to text insertion

### files touched

- `tui/src/app.rs` — replace existing `Event::Paste` single-line handler with new multi-line detection heuristic
- `tui/src/attachment.rs` — add `try_attach_pasted_images()` function

---

## 3. non-vision model warning on attachment pills

### what already exists

attachment pill rendering is already implemented in `tui/src/tui/layout.rs` (lines 1190–1313). pills show `[image: {display_name}]` in accent color. backspace-to-remove works. send-time rejection for non-vision models exists in `app.rs` (lines 3927–3932).

### what changes (additive only)

- when `model_supports_vision` is false and attachments are present, dim/strikethrough the pills and show a footer hint: "current model doesn't support images"
- this is purely visual — the rejection logic at send time already exists

### files touched

- `tui/src/tui/layout.rs` — add conditional dim styling when `model_supports_vision` is false

---

## 4. existing plumbing (no changes needed)

already implemented and tested:

| component | status | location |
|-----------|--------|----------|
| `ContentBlock::Image` | done | `agent/conversation.rs` |
| anthropic image serialization | done | `provider/anthropic/` |
| openai image serialization | done | `provider/openai/` |
| gemini image serialization | done | `provider/gemini/` |
| openrouter (delegates to openai) | done | `provider/openrouter/` |
| `@file` image references | done | `attachment.rs`, `app.rs` |
| `is_image_path()` / `media_type_from_ext()` | done | `attachment.rs` |
| `read_image_attachment()` | done | `attachment.rs` |
| `extract_at_image_paths()` | done | `attachment.rs` |
| vision model detection (`supports_vision`) | done | `provider/mod.rs` |
| session persistence (sqlite) | done | `agent/conversation.rs` |
| transcript rendering (`[image: file]`) | done | `agent/conversation.rs` |
| file autocomplete (finds image files) | done | `tui/src/tui/file_auto.rs` |
| backspace-to-remove attachment | done | `app.rs` |
| attachment pill rendering | done | `tui/src/tui/layout.rs` (lines 1190–1313) |
| non-vision model send-time rejection | done | `app.rs` (lines 3927–3932) |

providers without vision (deepseek, groq, mistral) — the existing `model_supports_vision` check rejects gracefully. no changes needed.

---

## testing

### unit tests

- `clipboard.rs` — test `read_image_from_clipboard()` returns `None` when clipboard has text (mock arboard if possible, or test the PNG encoding path separately)
- `attachment.rs` — test `attachment_from_rgba()` produces valid PNG with correct media type
- `attachment.rs` — test `try_attach_pasted_images()`:
  - single image path → returns path, empty remainder
  - multiple image paths → returns all paths, empty remainder
  - mixed paths and text → returns image paths + remaining text
  - non-existent paths → falls through to text
  - quoted paths (`"/path/to/image.png"`) → trimmed and matched
  - non-image paths (`/path/to/file.rs`) → treated as text
  - existing file but not readable (permissions error) → flash error, not silent fall-through

### manual testing

- paste screenshot from clipboard → appears as attachment pill → sends to LLM → response references the image
- drag image file onto terminal → auto-attaches → sends correctly
- paste text normally → still works as before
- paste image path as text → auto-attaches (verify this is desirable behavior)
- attach image with non-vision model → warning shown, image rejected on send
- attach multiple images → all show as pills → backspace removes last one

---

## out of scope

- agent-initiated screenshots (tool that captures screen)
- image preview/thumbnails in chat (sixel, kitty graphics protocol)
- image compression/resizing before send
- browser automation for visual verification
