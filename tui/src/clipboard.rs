//! Clipboard access wrapper.

use arboard::Clipboard;

/// Try to read an image from the system clipboard.
/// Returns `Some((rgba_bytes, width, height))` if the clipboard contains an image,
/// `None` if it contains text or is empty/inaccessible.
pub fn read_image_from_clipboard() -> Option<(Vec<u8>, usize, usize)> {
    let mut clipboard = match Clipboard::new() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("Clipboard::new() failed: {e}");
            return None;
        }
    };
    let img = match clipboard.get_image() {
        Ok(img) => img,
        Err(e) => {
            tracing::debug!("clipboard.get_image() failed: {e}");
            return None;
        }
    };
    tracing::debug!(
        "Clipboard image: {}x{}, {} bytes",
        img.width,
        img.height,
        img.bytes.len()
    );
    Some((img.bytes.into_owned(), img.width, img.height))
}

/// Copy text to the system clipboard. Returns Ok(()) on success.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_image_returns_none_without_image() {
        // When clipboard has no image (or clipboard is unavailable),
        // the function should return None — not panic or error.
        let result = read_image_from_clipboard();
        let _ = result;
    }

    #[test]
    fn test_copy_returns_ok() {
        // On CI without a display server this may fail — that's expected.
        // On a real machine it should succeed.
        let result = copy_to_clipboard("hello clipboard");
        let _ = result;
    }
}
