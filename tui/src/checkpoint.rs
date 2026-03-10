//! Checkpoint system — per-turn file snapshots for rewind.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Snapshot of a file's state before modification.
#[derive(Debug, Clone)]
pub enum FileSnapshot {
    /// File existed — original contents.
    Existed(Vec<u8>),
    /// File did not exist (created during this turn — delete on rewind).
    DidNotExist,
}

/// A single checkpoint representing the state before an agent turn.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub id: u32,
    pub prompt_preview: String,
    pub timestamp: Instant,
    pub files: HashMap<PathBuf, FileSnapshot>,
}

/// Manages checkpoints across an agent session.
#[derive(Debug)]
pub struct CheckpointManager {
    checkpoints: Vec<Checkpoint>,
    next_id: u32,
}

impl CheckpointManager {
    pub fn new() -> Self {
        Self {
            checkpoints: Vec::new(),
            next_id: 1,
        }
    }

    /// Create a new checkpoint for the current turn.
    /// `prompt_preview` is the first ~80 chars of the user prompt.
    pub fn create(&mut self, prompt_preview: &str) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.checkpoints.push(Checkpoint {
            id,
            prompt_preview: prompt_preview.chars().take(80).collect(),
            timestamp: Instant::now(),
            files: HashMap::new(),
        });
        id
    }

    /// Snapshot a file before it is modified. Only stores the first snapshot
    /// per file per checkpoint (captures state before the turn started).
    pub fn ensure_snapshotted(&mut self, path: &Path) {
        let Some(checkpoint) = self.checkpoints.last_mut() else {
            return;
        };
        let canonical = path.to_path_buf();
        if checkpoint.files.contains_key(&canonical) {
            return; // Already snapshotted in this turn
        }
        let snapshot = match std::fs::read(path) {
            Ok(bytes) => FileSnapshot::Existed(bytes),
            Err(_) => FileSnapshot::DidNotExist,
        };
        checkpoint.files.insert(canonical, snapshot);
    }

    /// List checkpoints for the picker UI.
    pub fn list(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Rewind to a checkpoint: restore all files, remove this and later checkpoints.
    /// Returns a summary of what was restored.
    pub fn rewind(&mut self, checkpoint_id: u32) -> Result<RewindSummary, String> {
        let idx = self
            .checkpoints
            .iter()
            .position(|c| c.id == checkpoint_id)
            .ok_or_else(|| format!("Checkpoint {checkpoint_id} not found"))?;

        // Collect all files from this checkpoint and later ones (rewind restores
        // to the state *before* the selected checkpoint's turn).
        let mut restored = 0u32;
        let mut deleted = 0u32;
        let mut errors: Vec<String> = Vec::new();

        for checkpoint in &self.checkpoints[idx..] {
            for (path, snapshot) in &checkpoint.files {
                match snapshot {
                    FileSnapshot::Existed(bytes) => {
                        if let Err(e) = std::fs::write(path, bytes) {
                            errors.push(format!("{}: {e}", path.display()));
                        } else {
                            restored += 1;
                        }
                    }
                    FileSnapshot::DidNotExist => {
                        if path.exists() {
                            if let Err(e) = std::fs::remove_file(path) {
                                errors.push(format!("{}: {e}", path.display()));
                            } else {
                                deleted += 1;
                            }
                        }
                    }
                }
            }
        }

        // Remove this checkpoint and all later ones
        self.checkpoints.truncate(idx);

        Ok(RewindSummary {
            restored,
            deleted,
            errors,
        })
    }
}

/// Result of a rewind operation.
pub struct RewindSummary {
    pub restored: u32,
    pub deleted: u32,
    pub errors: Vec<String>,
}

impl std::fmt::Display for RewindSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rewound: {} file(s) restored", self.restored)?;
        if self.deleted > 0 {
            write!(f, ", {} file(s) deleted", self.deleted)?;
        }
        if !self.errors.is_empty() {
            write!(f, " ({} error(s))", self.errors.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_checkpoint_increments_id() {
        let mut mgr = CheckpointManager::new();
        assert_eq!(mgr.create("first"), 1);
        assert_eq!(mgr.create("second"), 2);
        assert_eq!(mgr.checkpoints.len(), 2);
    }

    #[test]
    fn ensure_snapshotted_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "original").unwrap();

        let mut mgr = CheckpointManager::new();
        mgr.create("test prompt");
        mgr.ensure_snapshotted(&file);

        let cp = &mgr.checkpoints[0];
        assert!(cp.files.contains_key(&file));
        assert!(matches!(cp.files[&file], FileSnapshot::Existed(_)));
    }

    #[test]
    fn ensure_snapshotted_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.txt");

        let mut mgr = CheckpointManager::new();
        mgr.create("test");
        mgr.ensure_snapshotted(&file);

        let cp = &mgr.checkpoints[0];
        assert!(matches!(cp.files[&file], FileSnapshot::DidNotExist));
    }

    #[test]
    fn ensure_snapshotted_only_first_wins() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "original").unwrap();

        let mut mgr = CheckpointManager::new();
        mgr.create("test");
        mgr.ensure_snapshotted(&file);

        // Modify the file
        std::fs::write(&file, "modified").unwrap();
        // Snapshot again — should still have "original"
        mgr.ensure_snapshotted(&file);

        let cp = &mgr.checkpoints[0];
        if let FileSnapshot::Existed(bytes) = &cp.files[&file] {
            assert_eq!(bytes, b"original");
        } else {
            panic!("Expected Existed");
        }
    }

    #[test]
    fn rewind_restores_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "original").unwrap();

        let mut mgr = CheckpointManager::new();
        mgr.create("test");
        mgr.ensure_snapshotted(&file);

        // Modify file (simulating tool execution)
        std::fs::write(&file, "modified").unwrap();

        let summary = mgr.rewind(1).unwrap();
        assert_eq!(summary.restored, 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
        assert!(mgr.checkpoints.is_empty());
    }

    #[test]
    fn rewind_deletes_created_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new.txt");

        let mut mgr = CheckpointManager::new();
        mgr.create("test");
        mgr.ensure_snapshotted(&file);

        // Create file (simulating write tool)
        std::fs::write(&file, "created").unwrap();
        assert!(file.exists());

        let summary = mgr.rewind(1).unwrap();
        assert_eq!(summary.deleted, 1);
        assert!(!file.exists());
    }

    #[test]
    fn rewind_removes_later_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "v1").unwrap();

        let mut mgr = CheckpointManager::new();
        mgr.create("turn 1");
        mgr.ensure_snapshotted(&file);
        std::fs::write(&file, "v2").unwrap();

        mgr.create("turn 2");
        mgr.ensure_snapshotted(&file);
        std::fs::write(&file, "v3").unwrap();

        mgr.create("turn 3");

        // Rewind to checkpoint 2 — should restore to state before turn 2
        mgr.rewind(2).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2");
        assert_eq!(mgr.checkpoints.len(), 1); // Only checkpoint 1 remains
    }

    #[test]
    fn rewind_invalid_id() {
        let mut mgr = CheckpointManager::new();
        assert!(mgr.rewind(99).is_err());
    }

    #[test]
    fn no_checkpoint_means_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "data").unwrap();

        let mut mgr = CheckpointManager::new();
        // No create() call — ensure_snapshotted should be a no-op
        mgr.ensure_snapshotted(&file);
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn prompt_preview_truncated() {
        let mut mgr = CheckpointManager::new();
        let long = "x".repeat(200);
        mgr.create(&long);
        assert_eq!(mgr.checkpoints[0].prompt_preview.len(), 80);
    }
}
