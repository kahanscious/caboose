//! TUI layer — Ratatui setup, terminal management, and UI widgets.

pub mod approval;
pub mod ask_user;
pub mod chat;
pub mod command;
pub mod command_palette;
pub mod dialog;
pub mod file_auto;
pub mod file_browser;
pub mod footer;
pub mod header;
pub mod highlight;
pub mod home;
pub mod input;
pub mod input_buffer;
pub mod input_history;
pub mod key_input;
pub mod layout;
pub mod mcp_input;
pub mod model_picker;
pub mod provider_picker;
pub mod roundhouse_picker;
pub mod session_picker;
pub mod sidebar;
pub mod slash_auto;
pub mod theme;
pub mod tools;
pub mod workspace_list;

use anyhow::Result;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io;

/// Wrapper around the ratatui terminal.
pub struct Terminal {
    inner: ratatui::Terminal<CrosstermBackend<io::Stdout>>,
    keyboard_enhanced: bool,
}

impl Terminal {
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let inner = ratatui::Terminal::new(backend)?;
        Ok(Self {
            inner,
            keyboard_enhanced: false,
        })
    }

    /// Enter the alternate screen and enable raw mode.
    pub fn enter(&mut self) -> Result<()> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        // Enable keyboard enhancement (Kitty protocol) so Shift+Enter is
        // distinguishable from plain Enter. Silently ignored by terminals
        // that don't support it.
        if execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
        .is_ok()
        {
            self.keyboard_enhanced = true;
        }
        self.inner.clear()?;
        Ok(())
    }

    /// Exit the alternate screen and restore terminal state.
    pub fn exit(&mut self) -> Result<()> {
        if self.keyboard_enhanced {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        disable_raw_mode()?;
        execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
        Ok(())
    }

    /// Draw a frame using the provided rendering function.
    pub fn draw<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.inner.draw(f)?;
        Ok(())
    }
}
