//! Interactive terminal panel backed by a PTY + vt100 parser.
//!
//! Uses `portable-pty` to spawn a shell and `vt100::Parser` to parse ANSI
//! escape sequences into a renderable screen grid. A background thread reads
//! from the PTY master in a loop and sends chunks over an `mpsc` channel so
//! that the TUI event loop is never blocked on I/O.

use std::sync::mpsc;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::safety::env_filter;

/// Default scrollback buffer size in lines.
const SCROLLBACK_LINES: usize = 1000;

/// An interactive terminal panel.
///
/// Owns a PTY child process, a vt100 parser for screen state, and a
/// background reader thread that forwards PTY output over a channel.
pub struct TerminalPanel {
    /// vt100 parser that maintains the virtual screen.
    parser: vt100::Parser,

    /// Writer handle to the PTY master — sends keystrokes to the shell.
    writer: Box<dyn std::io::Write + Send>,

    /// The spawned child process.
    child: Box<dyn Child + Send + Sync>,

    /// Master PTY handle (kept alive for resize).
    master: Box<dyn MasterPty + Send>,

    /// Receiving end of the non-blocking reader channel.
    rx: mpsc::Receiver<Vec<u8>>,

    /// Whether the panel is currently visible in the UI.
    pub visible: bool,

    /// Whether the panel currently has keyboard focus.
    pub focused: bool,

    /// Current scrollback offset from the bottom (0 = latest output visible).
    pub scroll_offset: usize,
}

impl TerminalPanel {
    /// Spawn a new terminal panel.
    ///
    /// Opens a PTY of the given size, spawns the default shell with the
    /// working directory set to `cwd`, and starts a background reader thread.
    /// Environment variables are filtered through the safety module to strip
    /// secrets.
    pub fn new(cols: u16, rows: u16, cwd: &str) -> anyhow::Result<Self> {
        let pty_system = native_pty_system();
        let pty_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = pty_system.openpty(pty_size)?;

        let mut cmd = CommandBuilder::new_default_prog();
        cmd.cwd(cwd);

        // Remove secret env vars from the PTY environment.
        // We cannot use env_clear() on Windows (ConPTY needs the base env),
        // so instead we selectively remove secrets.
        let filtered = env_filter::filtered_env(&[]);
        let all_keys: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
        for key in &all_keys {
            if !filtered.iter().any(|(k, _)| k == key) {
                cmd.env_remove(key);
            }
        }

        let child = pair.slave.spawn_command(cmd)?;
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let parser = vt100::Parser::new(rows, cols, SCROLLBACK_LINES);

        // Non-blocking reader: background thread reads from PTY and sends
        // chunks over an mpsc channel.
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            Self::reader_loop(reader, tx);
        });

        Ok(Self {
            parser,
            writer,
            child,
            master: pair.master,
            rx,
            visible: false,
            focused: false,
            scroll_offset: 0,
        })
    }

    /// Background reader loop. Reads from the PTY master and sends chunks
    /// to the channel. Exits when the read returns 0 bytes (EOF) or errors.
    fn reader_loop(mut reader: Box<dyn std::io::Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        // Receiver dropped — panel was dropped.
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    /// Drain all pending output from the reader thread and feed it to the
    /// vt100 parser. Returns `true` if any new output was processed.
    pub fn poll_output(&mut self) -> bool {
        let mut got_data = false;
        while let Ok(chunk) = self.rx.try_recv() {
            self.parser.process(&chunk);
            got_data = true;
        }
        got_data
    }

    /// Send raw bytes (keystrokes) to the PTY.
    pub fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Resize the PTY and the vt100 parser.
    pub fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        self.parser.set_size(rows, cols);
        Ok(())
    }

    /// Returns a reference to the vt100 screen for rendering.
    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Check whether the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Scroll up by the given number of lines.
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        // Clamp to scrollback length — the parser tracks how much scrollback
        // is actually available via screen().scrollback().len().
        let max = self.parser.screen().scrollback();
        if self.scroll_offset > max {
            self.scroll_offset = max;
        }
        self.parser.set_scrollback(self.scroll_offset);
    }

    /// Scroll down by the given number of lines.
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.parser.set_scrollback(self.scroll_offset);
    }

    /// Kill the child process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

impl Drop for TerminalPanel {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> String {
        std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn spawn_and_check_alive() {
        let mut panel = TerminalPanel::new(80, 24, &cwd()).expect("failed to spawn");
        assert!(panel.is_alive(), "child should be alive right after spawn");
        panel.kill();
        // Give the process a moment to exit.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!panel.is_alive(), "child should be dead after kill");
    }

    #[test]
    fn write_and_read_output() {
        let mut panel = TerminalPanel::new(120, 24, &cwd()).expect("failed to spawn");

        // Give the shell a moment to initialize.
        std::thread::sleep(std::time::Duration::from_millis(500));
        panel.poll_output();

        // Write a command that echoes a unique marker.
        panel
            .write_input(b"echo hello_terminal_test\r\n")
            .expect("write_input failed");

        // Wait for the output to arrive.
        std::thread::sleep(std::time::Duration::from_millis(1000));
        panel.poll_output();

        // The screen should contain our marker string.
        let screen = panel.screen();
        let contents = screen.contents();
        assert!(
            contents.contains("hello_terminal_test"),
            "expected screen to contain 'hello_terminal_test', got:\n{}",
            contents
        );
    }

    #[test]
    fn scroll_offset_clamps() {
        let mut panel = TerminalPanel::new(80, 24, &cwd()).expect("failed to spawn");
        // Scroll up way past any available scrollback.
        panel.scroll_up(100);
        // Scroll down way past zero.
        panel.scroll_down(200);
        assert_eq!(panel.scroll_offset, 0, "scroll offset should clamp to 0");
    }

    #[test]
    fn resize_updates_parser() {
        let mut panel = TerminalPanel::new(80, 24, &cwd()).expect("failed to spawn");
        panel.resize(120, 40).expect("resize failed");
        let (rows, cols) = panel.screen().size();
        assert_eq!((rows, cols), (40, 120), "screen size should be (40, 120)");
    }
}
