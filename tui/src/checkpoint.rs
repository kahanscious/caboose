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
    pub name: Option<String>,
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
            name: None,
            timestamp: Instant::now(),
            files: HashMap::new(),
        });
        id
    }

    /// Create a user-named checkpoint that snapshots all files modified across
    /// the entire session (union of all file paths from every checkpoint).
    pub fn create_named(&mut self, name: &str) -> u32 {
        // Collect all file paths that have been modified in the session.
        let all_paths: Vec<PathBuf> = self
            .checkpoints
            .iter()
            .flat_map(|cp| cp.files.keys().cloned())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let id = self.next_id;
        self.next_id += 1;

        let mut files = HashMap::new();
        for path in &all_paths {
            let snapshot = match std::fs::read(path) {
                Ok(bytes) => FileSnapshot::Existed(bytes),
                Err(_) => FileSnapshot::DidNotExist,
            };
            files.insert(path.clone(), snapshot);
        }

        self.checkpoints.push(Checkpoint {
            id,
            prompt_preview: name.chars().take(80).collect(),
            name: Some(name.to_string()),
            timestamp: Instant::now(),
            files,
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

    /// Preview what a rewind to the given checkpoint would change, without
    /// actually modifying any files.
    pub fn preview(&self, checkpoint_id: u32) -> Result<Vec<PreviewEntry>, String> {
        let idx = self
            .checkpoints
            .iter()
            .position(|c| c.id == checkpoint_id)
            .ok_or_else(|| format!("Checkpoint {checkpoint_id} not found"))?;

        // De-duplicate: first snapshot for a path wins (earliest checkpoint).
        let mut seen = std::collections::HashSet::new();
        let mut entries = Vec::new();

        for checkpoint in &self.checkpoints[idx..] {
            for (path, snapshot) in &checkpoint.files {
                if !seen.insert(path.clone()) {
                    continue;
                }
                let action = match snapshot {
                    FileSnapshot::Existed(bytes) => {
                        let current = std::fs::read(path).unwrap_or_default();
                        if current == *bytes {
                            PreviewAction::NoChange
                        } else {
                            let old_lines = String::from_utf8_lossy(bytes);
                            let new_lines = String::from_utf8_lossy(&current);
                            let old: Vec<&str> = old_lines.lines().collect();
                            let cur: Vec<&str> = new_lines.lines().collect();
                            let mut added = 0usize;
                            let mut removed = 0usize;
                            // Simple line-count diff: lines in current but not in snapshot = removed on rewind,
                            // lines in snapshot but not in current = added on rewind.
                            let max_len = old.len().max(cur.len());
                            for i in 0..max_len {
                                let o = old.get(i).copied();
                                let c = cur.get(i).copied();
                                if o != c {
                                    if o.is_some() {
                                        added += 1;
                                    }
                                    if c.is_some() {
                                        removed += 1;
                                    }
                                }
                            }
                            PreviewAction::Restore {
                                lines_added: added,
                                lines_removed: removed,
                            }
                        }
                    }
                    FileSnapshot::DidNotExist => {
                        if path.exists() {
                            PreviewAction::Delete
                        } else {
                            PreviewAction::NoChange
                        }
                    }
                };
                if !matches!(action, PreviewAction::NoChange) {
                    entries.push(PreviewEntry {
                        path: path.clone(),
                        action,
                    });
                }
            }
        }
        Ok(entries)
    }
}

/// An entry describing what would change if a rewind were performed.
pub struct PreviewEntry {
    pub path: PathBuf,
    pub action: PreviewAction,
}

/// The action that would be taken on a file during rewind.
pub enum PreviewAction {
    Restore {
        lines_added: usize,
        lines_removed: usize,
    },
    Delete,
    NoChange,
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

    #[test]
    fn create_named_sets_name_and_snapshots_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        std::fs::write(&file_a, "aaa").unwrap();

        let mut mgr = CheckpointManager::new();

        // Turn 1 modifies file_a
        mgr.create("turn 1");
        mgr.ensure_snapshotted(&file_a);
        std::fs::write(&file_a, "aaa-modified").unwrap();

        // Turn 2 creates file_b
        mgr.create("turn 2");
        mgr.ensure_snapshotted(&file_b);
        std::fs::write(&file_b, "bbb").unwrap();

        // Named checkpoint should snapshot both files
        let id = mgr.create_named("my save");
        let cp = mgr.checkpoints.last().unwrap();
        assert_eq!(cp.id, id);
        assert_eq!(cp.name.as_deref(), Some("my save"));
        assert_eq!(cp.prompt_preview, "my save");
        assert!(cp.files.contains_key(&file_a));
        assert!(cp.files.contains_key(&file_b));
    }

    #[test]
    fn preview_returns_restore_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("exist.txt");
        let created = dir.path().join("new.txt");
        std::fs::write(&existing, "line1\nline2\n").unwrap();

        let mut mgr = CheckpointManager::new();
        mgr.create("turn 1");
        mgr.ensure_snapshotted(&existing);
        mgr.ensure_snapshotted(&created);

        // Modify existing file & create the new one
        std::fs::write(&existing, "line1\nchanged\nextra\n").unwrap();
        std::fs::write(&created, "brand new").unwrap();

        let entries = mgr.preview(1).unwrap();
        assert_eq!(entries.len(), 2);

        let restore = entries.iter().find(|e| e.path == existing).unwrap();
        assert!(matches!(restore.action, PreviewAction::Restore { .. }));

        let delete = entries.iter().find(|e| e.path == created).unwrap();
        assert!(matches!(delete.action, PreviewAction::Delete));
    }

    #[test]
    fn preview_filters_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("stable.txt");
        std::fs::write(&file, "unchanged").unwrap();

        let mut mgr = CheckpointManager::new();
        mgr.create("turn 1");
        mgr.ensure_snapshotted(&file);
        // File not modified — preview should return empty

        let entries = mgr.preview(1).unwrap();
        assert!(entries.is_empty());
    }
}
