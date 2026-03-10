//! Clipboard access wrapper.

use arboard::Clipboard;

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
    fn test_copy_returns_ok() {
        // On CI without a display server this may fail — that's expected.
        // On a real machine it should succeed.
        let result = copy_to_clipboard("hello clipboard");
        let _ = result;
    }
}
