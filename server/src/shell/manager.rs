use std::collections::HashMap;
use std::io::{Read, Write};
use tokio::sync::mpsc;
use portable_pty::{native_pty_system, CommandBuilder, PtySize, MasterPty, Child};

const MAX_SHELLS: usize = 3;
const READ_BUF_SIZE: usize = 4096;

#[derive(Debug, Clone)]
pub enum ShellEvent {
    Output { shell_id: String, data: String },
    Exited { shell_id: String, exit_code: i32 },
}

struct ManagedShell {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _reader_abort: tokio::sync::oneshot::Sender<()>,
}

pub struct ShellManager {
    shells: HashMap<String, ManagedShell>,
    event_tx: mpsc::UnboundedSender<ShellEvent>,
}

impl ShellManager {
    pub fn new(event_tx: mpsc::UnboundedSender<ShellEvent>) -> Self {
        Self {
            shells: HashMap::new(),
            event_tx,
        }
    }

    pub fn shell_count(&self) -> usize {
        self.shells.len()
    }

    pub fn spawn(&mut self, cols: u16, rows: u16) -> Result<String, String> {
        if self.shells.len() >= MAX_SHELLS {
            return Err(format!("Max shells ({MAX_SHELLS}) reached"));
        }

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {e}"))?;

        let shell_cmd = if cfg!(windows) { "cmd.exe" } else { "/bin/bash" };
        let cmd = CommandBuilder::new(shell_cmd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell: {e}"))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone reader: {e}"))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to take writer: {e}"))?;

        let shell_id = uuid::Uuid::new_v4().to_string();

        // Spawn a blocking reader task
        let tx = self.event_tx.clone();
        let sid = shell_id.clone();
        let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();

        tokio::task::spawn_blocking(move || {
            reader_loop(reader, tx, sid, abort_rx);
        });

        self.shells.insert(
            shell_id.clone(),
            ManagedShell {
                master: pair.master,
                writer,
                child,
                _reader_abort: abort_tx,
            },
        );

        Ok(shell_id)
    }

    pub fn write(&mut self, shell_id: &str, data: &str) -> Result<(), String> {
        let shell = self
            .shells
            .get_mut(shell_id)
            .ok_or_else(|| format!("Shell {shell_id} not found"))?;

        shell
            .writer
            .write_all(data.as_bytes())
            .map_err(|e| format!("Write failed: {e}"))?;

        shell
            .writer
            .flush()
            .map_err(|e| format!("Flush failed: {e}"))?;

        Ok(())
    }

    pub fn resize(&mut self, shell_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let shell = self
            .shells
            .get_mut(shell_id)
            .ok_or_else(|| format!("Shell {shell_id} not found"))?;

        shell
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Resize failed: {e}"))?;

        Ok(())
    }

    pub fn kill(&mut self, shell_id: &str) -> Result<(), String> {
        let mut shell = self
            .shells
            .remove(shell_id)
            .ok_or_else(|| format!("Shell {shell_id} not found"))?;

        let _ = shell.child.kill();
        // Dropping the ManagedShell will drop master/writer/abort_tx,
        // which closes the PTY and signals the reader task to stop.
        Ok(())
    }

    pub fn kill_all(&mut self) {
        let ids: Vec<String> = self.shells.keys().cloned().collect();
        for id in ids {
            let _ = self.kill(&id);
        }
    }
}

impl Drop for ShellManager {
    fn drop(&mut self) {
        self.kill_all();
    }
}

fn reader_loop(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::UnboundedSender<ShellEvent>,
    shell_id: String,
    mut abort_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut buf = [0u8; READ_BUF_SIZE];
    loop {
        // Check if we've been asked to stop
        match abort_rx.try_recv() {
            Ok(()) | Err(tokio::sync::oneshot::error::TryRecvError::Closed) => break,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }

        match reader.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(ShellEvent::Exited {
                    shell_id,
                    exit_code: 0,
                });
                break;
            }
            Ok(n) => {
                let data = String::from_utf8_lossy(&buf[..n]).to_string();
                if tx
                    .send(ShellEvent::Output {
                        shell_id: shell_id.clone(),
                        data,
                    })
                    .is_err()
                {
                    break; // receiver dropped
                }
            }
            Err(_) => {
                let _ = tx.send(ShellEvent::Exited {
                    shell_id,
                    exit_code: -1,
                });
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_and_write() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut mgr = ShellManager::new(tx);

        let shell_id = mgr.spawn(80, 24).unwrap();
        assert_eq!(mgr.shell_count(), 1);

        // Write a command
        let cmd = if cfg!(windows) {
            "echo hello\r\n"
        } else {
            "echo hello\n"
        };
        mgr.write(&shell_id, cmd).unwrap();

        // Should get some output
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();

        match event {
            ShellEvent::Output { shell_id: id, data } => {
                assert_eq!(id, shell_id);
                assert!(!data.is_empty());
            }
            _ => panic!("expected Output event"),
        }

        mgr.kill(&shell_id).unwrap();
        assert_eq!(mgr.shell_count(), 0);
    }

    #[tokio::test]
    async fn max_shells_enforced() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut mgr = ShellManager::new(tx);

        for _ in 0..3 {
            mgr.spawn(80, 24).unwrap();
        }
        assert_eq!(mgr.shell_count(), 3);

        let result = mgr.spawn(80, 24);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Max"));
    }
}
