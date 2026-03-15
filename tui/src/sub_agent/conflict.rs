//! Conflict detection for parallel subagents.
//!
//! Pure functions that parse git diff output and detect overlapping edits
//! between agents. Git command execution is separate (in worktree.rs).

use uuid::Uuid;

/// How a file was changed.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
}

/// A range of lines modified in a file.
#[derive(Debug, Clone, PartialEq)]
pub struct HunkRange {
    pub start: u32,
    pub end: u32,
}

/// A file changed by an agent.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    pub hunks: Vec<HunkRange>,
}

/// Summary of all changes made by a single agent.
#[derive(Debug)]
pub struct AgentChanges {
    pub agent_id: Uuid,
    pub task: String,
    #[allow(dead_code)]
    pub branch: String,
    #[allow(dead_code)]
    pub base_sha: String,
    pub files: Vec<FileChange>,
}

/// How severe an overlap is.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlapSeverity {
    /// Same file, non-overlapping hunks. Git can auto-merge.
    Warn,
    /// Same file, overlapping hunks. Likely merge conflict.
    Block,
}

/// An agent participating in an overlap.
#[derive(Debug, Clone)]
pub struct OverlapParticipant {
    pub agent_id: Uuid,
    pub task: String,
    pub hunks: Vec<HunkRange>,
}

/// A detected overlap between agents.
#[derive(Debug)]
pub struct Overlap {
    #[allow(dead_code)]
    pub file: String,
    pub participants: Vec<OverlapParticipant>,
    pub severity: OverlapSeverity,
    pub details: String,
}

/// The full conflict analysis report.
#[derive(Debug)]
pub struct ConflictReport {
    pub overlaps: Vec<Overlap>,
}

impl ConflictReport {
    pub fn has_blocking(&self) -> bool {
        self.overlaps
            .iter()
            .any(|o| matches!(o.severity, OverlapSeverity::Block))
    }
}

/// Parse `git diff --unified=0` output into FileChange structs.
pub fn parse_diff_hunks(diff_output: &str) -> Vec<FileChange> {
    let mut files = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_kind = ChangeKind::Modified;
    let mut current_hunks: Vec<HunkRange> = Vec::new();
    let mut rename_from: Option<String> = None;

    for line in diff_output.lines() {
        if line.starts_with("diff --git ") {
            // Flush previous file
            if let Some(path) = current_path.take() {
                files.push(FileChange {
                    path,
                    kind: current_kind.clone(),
                    hunks: std::mem::take(&mut current_hunks),
                });
            }
            // Parse path from "diff --git a/path b/path"
            if let Some(b_part) = line.split(" b/").last() {
                current_path = Some(b_part.to_string());
            }
            current_kind = ChangeKind::Modified;
            rename_from = None;
        } else if line.starts_with("new file") {
            current_kind = ChangeKind::Added;
        } else if line.starts_with("deleted file") {
            current_kind = ChangeKind::Deleted;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            rename_from = Some(from.to_string());
        } else if line.starts_with("rename to ") {
            if let Some(from) = rename_from.take() {
                current_kind = ChangeKind::Renamed { from };
            }
        } else if line.starts_with("@@ ") {
            // Parse hunk header: @@ -A,B +C,D @@
            if let Some(hunk) = parse_hunk_header(line) {
                current_hunks.push(hunk);
            }
        }
    }

    // Flush last file
    if let Some(path) = current_path {
        files.push(FileChange {
            path,
            kind: current_kind,
            hunks: current_hunks,
        });
    }

    files
}

/// Parse a unified diff hunk header like "@@ -10,3 +10,5 @@" into a HunkRange.
/// Returns the range for the new file side (+C,D).
fn parse_hunk_header(line: &str) -> Option<HunkRange> {
    // Find the +C,D part
    let plus_part = line.split('+').nth(1)?;
    let nums = plus_part.split(' ').next()?;
    let parts: Vec<&str> = nums.split(',').collect();
    let start: u32 = parts.first()?.parse().ok()?;
    let count: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    if count == 0 {
        return None; // pure deletion in this hunk, no new-file lines
    }
    Some(HunkRange {
        start,
        end: start + count - 1,
    })
}

/// Check if two hunk ranges overlap (true intersection, no adjacency buffer).
fn hunks_overlap(a: &HunkRange, b: &HunkRange) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Compare all agents' changes and produce a conflict report.
pub fn cross_agent_check(all_changes: &[AgentChanges]) -> ConflictReport {
    use std::collections::HashMap;

    // Group files by path across all agents
    let mut file_map: HashMap<String, Vec<(Uuid, String, &FileChange)>> = HashMap::new();
    for agent in all_changes {
        for file in &agent.files {
            // Track by canonical path (new path for renames, path for others)
            let key = file.path.clone();
            file_map
                .entry(key)
                .or_default()
                .push((agent.agent_id, agent.task.clone(), file));

            // Also track rename source path for rename-vs-modify detection
            if let ChangeKind::Renamed { ref from } = file.kind {
                file_map.entry(from.clone()).or_default().push((
                    agent.agent_id,
                    agent.task.clone(),
                    file,
                ));
            }
        }
    }

    let mut overlaps = Vec::new();

    for (path, agents) in &file_map {
        if agents.len() < 2 {
            continue;
        }

        // Determine severity based on change kinds and hunk overlap
        let severity = determine_severity(agents);

        let participants: Vec<OverlapParticipant> = agents
            .iter()
            .map(|(id, task, fc)| OverlapParticipant {
                agent_id: *id,
                task: task.clone(),
                hunks: fc.hunks.clone(),
            })
            .collect();

        let details = format_overlap_details(path, &participants, &severity);

        overlaps.push(Overlap {
            file: path.clone(),
            participants,
            severity,
            details,
        });
    }

    ConflictReport { overlaps }
}

fn determine_severity(agents: &[(Uuid, String, &FileChange)]) -> OverlapSeverity {
    // Check for structural conflicts first
    let has_delete = agents
        .iter()
        .any(|(_, _, fc)| matches!(fc.kind, ChangeKind::Deleted));
    let has_modify = agents
        .iter()
        .any(|(_, _, fc)| matches!(fc.kind, ChangeKind::Modified));
    let has_rename = agents
        .iter()
        .any(|(_, _, fc)| matches!(fc.kind, ChangeKind::Renamed { .. }));
    let add_count = agents
        .iter()
        .filter(|(_, _, fc)| matches!(fc.kind, ChangeKind::Added))
        .count();

    // Both added same file → Block
    if add_count >= 2 {
        return OverlapSeverity::Block;
    }
    // Delete + modify → Block
    if has_delete && has_modify {
        return OverlapSeverity::Block;
    }
    // Rename + modify on old path → Block
    if has_rename && has_modify {
        return OverlapSeverity::Block;
    }

    // All modified — check hunk overlap
    for i in 0..agents.len() {
        for j in (i + 1)..agents.len() {
            for ha in &agents[i].2.hunks {
                for hb in &agents[j].2.hunks {
                    if hunks_overlap(ha, hb) {
                        return OverlapSeverity::Block;
                    }
                }
            }
        }
    }

    OverlapSeverity::Warn
}

fn format_overlap_details(
    path: &str,
    participants: &[OverlapParticipant],
    severity: &OverlapSeverity,
) -> String {
    let agent_names: Vec<&str> = participants.iter().map(|p| p.task.as_str()).collect();
    let severity_label = match severity {
        OverlapSeverity::Warn => "non-overlapping edits",
        OverlapSeverity::Block => "overlapping edits",
    };
    let hunk_details: Vec<String> = participants
        .iter()
        .filter(|p| !p.hunks.is_empty())
        .map(|p| {
            let ranges: Vec<String> = p
                .hunks
                .iter()
                .map(|h| {
                    if h.start == h.end {
                        format!("L{}", h.start)
                    } else {
                        format!("L{}-{}", h.start, h.end)
                    }
                })
                .collect();
            format!("{}: {}", p.task, ranges.join(", "))
        })
        .collect();
    format!(
        "{} — {} ({})\n  {}",
        path,
        severity_label,
        agent_names.join(" + "),
        hunk_details.join("\n  ")
    )
}

/// Format a conflict report as a human-readable text block for the chat.
pub fn format_conflict_report_text(report: &ConflictReport) -> String {
    let mut lines = vec!["Conflict Analysis".to_string(), String::new()];
    for overlap in &report.overlaps {
        let icon = match overlap.severity {
            OverlapSeverity::Warn => "  ⚠",
            OverlapSeverity::Block => "  ✗",
        };
        lines.push(format!("{icon} {}", overlap.details));
    }
    if report.has_blocking() {
        lines.push(String::new());
        lines.push("Approve merge? [y/n]".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_modified_file_single_hunk() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,3 +10,5 @@ fn main() {
";
        let files = parse_diff_hunks(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].kind, ChangeKind::Modified);
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[0].hunks[0], HunkRange { start: 10, end: 14 });
    }

    #[test]
    fn parse_modified_file_multiple_hunks() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -5,2 +5,4 @@ use std::io;
@@ -20,0 +22,3 @@ fn helper() {
";
        let files = parse_diff_hunks(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[0], HunkRange { start: 5, end: 8 });
        assert_eq!(files[0].hunks[1], HunkRange { start: 22, end: 24 });
    }

    #[test]
    fn parse_added_file() {
        let diff = "\
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,10 @@
";
        let files = parse_diff_hunks(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/new.rs");
        assert_eq!(files[0].kind, ChangeKind::Added);
        assert_eq!(files[0].hunks[0], HunkRange { start: 1, end: 10 });
    }

    #[test]
    fn parse_deleted_file() {
        let diff = "\
diff --git a/src/old.rs b/src/old.rs
deleted file mode 100644
index abc1234..0000000
--- a/src/old.rs
+++ /dev/null
@@ -1,5 +0,0 @@
";
        let files = parse_diff_hunks(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/old.rs");
        assert_eq!(files[0].kind, ChangeKind::Deleted);
    }

    #[test]
    fn parse_renamed_file() {
        let diff = "\
diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 90%
rename from src/old_name.rs
rename to src/new_name.rs
index abc1234..def5678 100644
--- a/src/old_name.rs
+++ b/src/new_name.rs
@@ -3,2 +3,4 @@ fn foo() {
";
        let files = parse_diff_hunks(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/new_name.rs");
        assert_eq!(
            files[0].kind,
            ChangeKind::Renamed {
                from: "src/old_name.rs".to_string()
            }
        );
    }

    #[test]
    fn parse_multiple_files() {
        let diff = "\
diff --git a/src/a.rs b/src/a.rs
index abc..def 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,1 +1,2 @@
diff --git a/src/b.rs b/src/b.rs
index abc..def 100644
--- a/src/b.rs
+++ b/src/b.rs
@@ -5,1 +5,3 @@
";
        let files = parse_diff_hunks(diff);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[1].path, "src/b.rs");
    }

    #[test]
    fn parse_empty_diff() {
        let files = parse_diff_hunks("");
        assert!(files.is_empty());
    }

    #[test]
    fn hunks_no_overlap() {
        let a = HunkRange { start: 1, end: 5 };
        let b = HunkRange { start: 10, end: 15 };
        assert!(!hunks_overlap(&a, &b));
    }

    #[test]
    fn hunks_overlap_partial() {
        let a = HunkRange { start: 5, end: 10 };
        let b = HunkRange { start: 8, end: 15 };
        assert!(hunks_overlap(&a, &b));
    }

    #[test]
    fn hunks_overlap_identical() {
        let a = HunkRange { start: 5, end: 10 };
        assert!(hunks_overlap(&a, &a));
    }

    #[test]
    fn hunks_adjacent_no_overlap() {
        let a = HunkRange { start: 1, end: 5 };
        let b = HunkRange { start: 6, end: 10 };
        assert!(!hunks_overlap(&a, &b));
    }

    #[test]
    fn hunks_single_line_overlap() {
        let a = HunkRange { start: 5, end: 5 };
        let b = HunkRange { start: 5, end: 5 };
        assert!(hunks_overlap(&a, &b));
    }

    #[test]
    fn cross_agent_no_overlap() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                branch: "agent/a".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/a.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 1, end: 10 }],
                }],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                branch: "agent/b".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/b.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 1, end: 10 }],
                }],
            },
        ];
        let report = cross_agent_check(&changes);
        assert!(report.overlaps.is_empty());
        assert!(!report.has_blocking());
    }

    #[test]
    fn cross_agent_same_file_no_hunk_overlap() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                branch: "agent/a".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/shared.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 1, end: 5 }],
                }],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                branch: "agent/b".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/shared.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 50, end: 60 }],
                }],
            },
        ];
        let report = cross_agent_check(&changes);
        assert_eq!(report.overlaps.len(), 1);
        assert_eq!(report.overlaps[0].severity, OverlapSeverity::Warn);
        assert!(!report.has_blocking());
    }

    #[test]
    fn cross_agent_blocking_overlap() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let changes = vec![
            AgentChanges {
                agent_id: id_a,
                task: "task-a".into(),
                branch: "agent/a".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/shared.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 10, end: 20 }],
                }],
            },
            AgentChanges {
                agent_id: id_b,
                task: "task-b".into(),
                branch: "agent/b".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/shared.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 15, end: 25 }],
                }],
            },
        ];
        let report = cross_agent_check(&changes);
        assert_eq!(report.overlaps.len(), 1);
        assert_eq!(report.overlaps[0].severity, OverlapSeverity::Block);
        assert!(report.has_blocking());
        assert_eq!(report.overlaps[0].participants.len(), 2);
    }

    #[test]
    fn cross_agent_both_add_same_file() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                branch: "agent/a".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/new.rs".into(),
                    kind: ChangeKind::Added,
                    hunks: vec![HunkRange { start: 1, end: 10 }],
                }],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                branch: "agent/b".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/new.rs".into(),
                    kind: ChangeKind::Added,
                    hunks: vec![HunkRange { start: 1, end: 5 }],
                }],
            },
        ];
        let report = cross_agent_check(&changes);
        assert!(report.has_blocking());
    }

    #[test]
    fn cross_agent_delete_vs_modify() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                branch: "agent/a".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/doomed.rs".into(),
                    kind: ChangeKind::Deleted,
                    hunks: vec![],
                }],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                branch: "agent/b".into(),
                base_sha: "abc".into(),
                files: vec![FileChange {
                    path: "src/doomed.rs".into(),
                    kind: ChangeKind::Modified,
                    hunks: vec![HunkRange { start: 1, end: 10 }],
                }],
            },
        ];
        let report = cross_agent_check(&changes);
        assert!(report.has_blocking());
    }
}
