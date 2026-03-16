//! Conflict detection for parallel subagents.
//!
//! Pure functions that parse git diff output and detect overlapping edits
//! between agents. Git command execution is separate (in worktree.rs).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use regex::Regex;
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkRange {
    pub start: u32,
    pub end: u32,
}

/// Symbol kinds used by semantic conflict checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    TypeAlias,
    Const,
}

/// A symbol touched by a file change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub signature: String,
    pub range: HunkRange,
}

/// A signature change detected in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureChange {
    pub name: String,
    pub kind: SymbolKind,
    pub before: String,
    pub after: String,
}

/// A file changed by an agent.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    pub hunks: Vec<HunkRange>,
    pub added_lines: Vec<String>,
    pub removed_lines: Vec<String>,
    pub symbols: Vec<ChangedSymbol>,
    pub signature_changes: Vec<SignatureChange>,
}

/// Summary of all changes made by a single agent.
#[derive(Debug)]
pub struct AgentChanges {
    pub agent_id: Uuid,
    pub task: String,
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

/// How Caboose should handle a detected overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlapResolution {
    /// Safe to merge mechanically with no extra reasoning.
    AutoMerge,
    /// Safe to reconcile automatically because the edits appear complementary.
    AutoReconcile,
    /// Ambiguous or structurally conflicting — requires explicit review.
    RequiresReview,
}

/// A detected overlap between agents.
#[derive(Debug)]
pub struct Overlap {
    pub participants: Vec<OverlapParticipant>,
    pub resolution: OverlapResolution,
    pub details: String,
}

/// The full conflict analysis report.
#[derive(Debug)]
pub struct ConflictReport {
    pub overlaps: Vec<Overlap>,
}

#[derive(Debug, Clone)]
struct CodeSymbol {
    name: String,
    kind: SymbolKind,
    signature: String,
    start: u32,
    end: u32,
}

#[derive(Debug)]
struct SharedSymbolConflict {
    name: String,
    kind: SymbolKind,
    signatures: Vec<String>,
}

impl ConflictReport {
    pub fn requires_review(&self) -> bool {
        self.overlaps
            .iter()
            .any(|o| matches!(o.resolution, OverlapResolution::RequiresReview))
    }
}

/// Parse `git diff --unified=0` output into FileChange structs.
pub fn parse_diff_hunks(diff_output: &str) -> Vec<FileChange> {
    let mut files = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_kind = ChangeKind::Modified;
    let mut current_hunks: Vec<HunkRange> = Vec::new();
    let mut current_added_lines: Vec<String> = Vec::new();
    let mut current_removed_lines: Vec<String> = Vec::new();
    let mut rename_from: Option<String> = None;

    for line in diff_output.lines() {
        if line.starts_with("diff --git ") {
            flush_current_file(
                &mut files,
                &mut current_path,
                &mut current_kind,
                &mut current_hunks,
                &mut current_added_lines,
                &mut current_removed_lines,
            );
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
        } else if let Some(added) = line.strip_prefix('+') {
            if !line.starts_with("+++") {
                current_added_lines.push(added.to_string());
            }
        } else if let Some(removed) = line.strip_prefix('-') {
            if !line.starts_with("---") {
                current_removed_lines.push(removed.to_string());
            }
        }
    }

    flush_current_file(
        &mut files,
        &mut current_path,
        &mut current_kind,
        &mut current_hunks,
        &mut current_added_lines,
        &mut current_removed_lines,
    );

    files
}

fn flush_current_file(
    files: &mut Vec<FileChange>,
    current_path: &mut Option<String>,
    current_kind: &mut ChangeKind,
    current_hunks: &mut Vec<HunkRange>,
    current_added_lines: &mut Vec<String>,
    current_removed_lines: &mut Vec<String>,
) {
    if let Some(path) = current_path.take() {
        files.push(FileChange {
            path,
            kind: current_kind.clone(),
            hunks: std::mem::take(current_hunks),
            added_lines: std::mem::take(current_added_lines),
            removed_lines: std::mem::take(current_removed_lines),
            symbols: Vec::new(),
            signature_changes: Vec::new(),
        });
    }
}

/// Add semantic metadata to a parsed FileChange using pre- and post-change content.
pub fn enrich_file_change_semantics(
    file: &mut FileChange,
    base_content: Option<&str>,
    new_content: Option<&str>,
) {
    file.signature_changes =
        detect_signature_changes(&file.path, &file.added_lines, &file.removed_lines);
    file.symbols = detect_changed_symbols(file, base_content, new_content);
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
    Some(HunkRange {
        start,
        end: if count == 0 {
            start.max(1)
        } else {
            start + count - 1
        },
    })
}

/// Check if two hunk ranges overlap (true intersection, no adjacency buffer).
fn hunks_overlap(a: &HunkRange, b: &HunkRange) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// Compare all agents' changes and produce a conflict report.
pub fn cross_agent_check(all_changes: &[AgentChanges]) -> ConflictReport {
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

        let hunk_severity = determine_hunk_severity(agents);
        let symbol_conflicts = detect_shared_symbol_conflicts(agents);
        let participants: Vec<OverlapParticipant> = agents
            .iter()
            .map(|(id, task, fc)| OverlapParticipant {
                agent_id: *id,
                task: task.clone(),
                hunks: fc.hunks.clone(),
            })
            .collect();

        let resolution = classify_file_overlap_resolution(&hunk_severity, &symbol_conflicts);
        let details = format_file_overlap_details(
            path,
            agents,
            &participants,
            &hunk_severity,
            &symbol_conflicts,
            &resolution,
        );

        overlaps.push(Overlap {
            participants,
            resolution,
            details,
        });
    }

    overlaps.extend(detect_interface_conflicts(all_changes));

    ConflictReport { overlaps }
}

fn determine_hunk_severity(agents: &[(Uuid, String, &FileChange)]) -> OverlapSeverity {
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

fn classify_file_overlap_resolution(
    hunk_severity: &OverlapSeverity,
    symbol_conflicts: &[SharedSymbolConflict],
) -> OverlapResolution {
    if matches!(hunk_severity, OverlapSeverity::Block) {
        OverlapResolution::RequiresReview
    } else if !symbol_conflicts.is_empty() {
        OverlapResolution::AutoReconcile
    } else {
        OverlapResolution::AutoMerge
    }
}

fn detect_shared_symbol_conflicts(
    agents: &[(Uuid, String, &FileChange)],
) -> Vec<SharedSymbolConflict> {
    let mut symbol_map: BTreeMap<(String, SymbolKind), Vec<String>> = BTreeMap::new();

    for (_, task, file) in agents {
        let mut seen = HashSet::new();
        for symbol in &file.symbols {
            if seen.insert((symbol.name.clone(), symbol.kind.clone())) {
                symbol_map
                    .entry((symbol.name.clone(), symbol.kind.clone()))
                    .or_default()
                    .push(format!("{task}: {}", symbol.signature));
            }
        }
    }

    symbol_map
        .into_iter()
        .filter_map(|((name, kind), signatures)| {
            if signatures.len() >= 2 {
                Some(SharedSymbolConflict {
                    name,
                    kind,
                    signatures,
                })
            } else {
                None
            }
        })
        .collect()
}

fn format_file_overlap_details(
    path: &str,
    agents: &[(Uuid, String, &FileChange)],
    participants: &[OverlapParticipant],
    hunk_severity: &OverlapSeverity,
    symbol_conflicts: &[SharedSymbolConflict],
    resolution: &OverlapResolution,
) -> String {
    let agent_names: Vec<&str> = participants.iter().map(|p| p.task.as_str()).collect();
    let mut labels = vec![match resolution {
        OverlapResolution::AutoMerge => "auto-mergeable edits",
        OverlapResolution::AutoReconcile => "auto-reconcilable edits",
        OverlapResolution::RequiresReview => match hunk_severity {
            OverlapSeverity::Warn => "same-file edits",
            OverlapSeverity::Block => "overlapping edits",
        },
    }];
    if !symbol_conflicts.is_empty() {
        labels.push("shared symbols");
    }
    let mut lines = vec![format!(
        "{} — {} ({})",
        path,
        labels.join(", "),
        agent_names.join(" + ")
    )];
    let hunk_details: Vec<String> = participants
        .iter()
        .filter(|p| !p.hunks.is_empty())
        .map(|p| {
            format!("{}: {}", p.task, format_hunk_ranges(&p.hunks))
        })
        .collect();
    if !hunk_details.is_empty() {
        lines.push(format!("  lines: {}", hunk_details.join(" | ")));
    }
    let snippets: Vec<String> = agents
        .iter()
        .filter_map(|(_, task, file)| {
            first_changed_snippet(file).map(|snippet| format!("{task}: {snippet}"))
        })
        .collect();
    if !snippets.is_empty() {
        lines.push(format!("  snippets: {}", snippets.join(" | ")));
    }
    if !symbol_conflicts.is_empty() {
        for conflict in symbol_conflicts {
            lines.push(format!(
                "  symbol {} `{}`: {}",
                symbol_kind_label(&conflict.kind),
                conflict.name,
                conflict.signatures.join(" | ")
            ));
        }
    } else if matches!(hunk_severity, OverlapSeverity::Warn) {
        let changed_by: Vec<String> = agents
            .iter()
            .map(|(_, task, file)| format!("{task}: {}", format_hunk_ranges(&file.hunks)))
            .collect();
        lines.push(format!("  non-overlapping hunks: {}", changed_by.join(" | ")));
    }
    lines.join("\n")
}

fn detect_interface_conflicts(all_changes: &[AgentChanges]) -> Vec<Overlap> {
    let mut overlaps = Vec::new();
    let mut seen = HashSet::new();

    for source_agent in all_changes {
        for source_file in &source_agent.files {
            for sig_change in &source_file.signature_changes {
                if sig_change.name.len() < 3 {
                    continue;
                }

                for other_agent in all_changes {
                    if other_agent.agent_id == source_agent.agent_id {
                        continue;
                    }

                    let mut reference_hits = Vec::new();
                    let mut other_hunks = Vec::new();
                    for other_file in &other_agent.files {
                        if other_file.path == source_file.path {
                            continue;
                        }
                        if let Some(snippet) = find_reference_snippet(other_file, sig_change) {
                            reference_hits.push(format!("{}: {}", other_file.path, snippet));
                            other_hunks.extend(other_file.hunks.clone());
                        }
                    }

                    if reference_hits.is_empty() {
                        continue;
                    }

                    let key = (
                        source_agent.agent_id,
                        source_file.path.clone(),
                        sig_change.name.clone(),
                        other_agent.agent_id,
                    );
                    if !seen.insert(key) {
                        continue;
                    }

                    overlaps.push(Overlap {
                        participants: vec![
                            OverlapParticipant {
                                agent_id: source_agent.agent_id,
                                task: source_agent.task.clone(),
                                hunks: source_file.hunks.clone(),
                            },
                            OverlapParticipant {
                                agent_id: other_agent.agent_id,
                                task: other_agent.task.clone(),
                                hunks: other_hunks,
                            },
                        ],
                        resolution: OverlapResolution::RequiresReview,
                        details: format!(
                            "{} — interface drift on {} `{}`\n  {} changed signature: {} -> {}\n  {} touched likely dependents: {}",
                            source_file.path,
                            symbol_kind_label(&sig_change.kind),
                            sig_change.name,
                            source_agent.task,
                            sig_change.before,
                            sig_change.after,
                            other_agent.task,
                            reference_hits.join(" | ")
                        ),
                    });
                }
            }
        }
    }

    overlaps
}

fn find_reference_snippet(file: &FileChange, sig_change: &SignatureChange) -> Option<String> {
    let matcher = reference_regex(&sig_change.name, &sig_change.kind)?;
    file.added_lines
        .iter()
        .chain(file.removed_lines.iter())
        .find_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || parse_symbol_declaration(&file.path, trimmed).is_some() {
                return None;
            }
            if matcher.is_match(trimmed) {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
}

fn reference_regex(name: &str, kind: &SymbolKind) -> Option<Regex> {
    let escaped = regex::escape(name);
    let pattern = match kind {
        SymbolKind::Function | SymbolKind::Method => format!(r"\b{escaped}\s*\("),
        _ => format!(r"\b{escaped}\b"),
    };
    Regex::new(&pattern).ok()
}

fn detect_changed_symbols(
    file: &FileChange,
    base_content: Option<&str>,
    new_content: Option<&str>,
) -> Vec<ChangedSymbol> {
    let content = match file.kind {
        ChangeKind::Deleted => base_content,
        _ => new_content.or(base_content),
    };

    let mut symbols: Vec<ChangedSymbol> = content
        .map(|text| extract_symbols(&file.path, text))
        .unwrap_or_default()
        .into_iter()
        .filter(|symbol| file.hunks.iter().any(|h| h.start <= symbol.end && symbol.start <= h.end))
        .map(|symbol| ChangedSymbol {
            name: symbol.name,
            kind: symbol.kind,
            signature: symbol.signature,
            range: HunkRange {
                start: symbol.start,
                end: symbol.end,
            },
        })
        .collect();

    if symbols.is_empty() {
        let fallback_range = file
            .hunks
            .first()
            .cloned()
            .unwrap_or(HunkRange { start: 1, end: 1 });
        for change in &file.signature_changes {
            symbols.push(ChangedSymbol {
                name: change.name.clone(),
                kind: change.kind.clone(),
                signature: if change.after.is_empty() {
                    change.before.clone()
                } else {
                    change.after.clone()
                },
                range: fallback_range.clone(),
            });
        }
    }

    let mut seen = HashSet::new();
    symbols.retain(|symbol| {
        seen.insert((
            symbol.name.clone(),
            symbol.kind.clone(),
            symbol.range.clone(),
        ))
    });
    symbols
}

fn detect_signature_changes(
    path: &str,
    added_lines: &[String],
    removed_lines: &[String],
) -> Vec<SignatureChange> {
    let mut before_map = BTreeMap::new();
    let mut after_map = BTreeMap::new();

    for line in removed_lines {
        if let Some((kind, name, signature)) = parse_symbol_declaration(path, line.trim()) {
            before_map.insert((name, kind), normalize_signature(&signature));
        }
    }
    for line in added_lines {
        if let Some((kind, name, signature)) = parse_symbol_declaration(path, line.trim()) {
            after_map.insert((name, kind), normalize_signature(&signature));
        }
    }

    let mut changes = Vec::new();
    for ((name, kind), before) in before_map {
        if let Some(after) = after_map.get(&(name.clone(), kind.clone()))
            && before != *after
        {
            changes.push(SignatureChange {
                name,
                kind,
                before,
                after: after.clone(),
            });
        }
    }
    changes
}

fn extract_symbols(path: &str, content: &str) -> Vec<CodeSymbol> {
    let mut starts = Vec::new();
    let total_lines = content.lines().count() as u32;

    for (idx, line) in content.lines().enumerate() {
        if let Some((kind, name, signature)) = parse_symbol_declaration(path, line.trim()) {
            starts.push((idx as u32 + 1, kind, name, normalize_signature(&signature)));
        }
    }

    let mut symbols = Vec::new();
    for i in 0..starts.len() {
        let (start, kind, name, signature) = &starts[i];
        let end = starts
            .get(i + 1)
            .map(|(next_start, _, _, _)| next_start.saturating_sub(1))
            .unwrap_or(total_lines.max(*start));
        symbols.push(CodeSymbol {
            name: name.clone(),
            kind: kind.clone(),
            signature: signature.clone(),
            start: *start,
            end,
        });
    }

    symbols
}

fn parse_symbol_declaration(path: &str, line: &str) -> Option<(SymbolKind, String, String)> {
    if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
        return None;
    }

    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    parse_rust_symbol(line)
        .or_else(|| parse_python_symbol(line).filter(|_| ext == "py"))
        .or_else(|| parse_go_symbol(line).filter(|_| ext == "go"))
        .or_else(|| {
            parse_js_like_symbol(line).filter(|_| {
                matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
            })
        })
        .or_else(|| parse_generic_symbol(line))
}

fn parse_rust_symbol(line: &str) -> Option<(SymbolKind, String, String)> {
    capture_decl(
        line,
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)\s*(?:<[^>]+>\s*)?\(",
        SymbolKind::Function,
    )
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:pub\s+)?struct\s+([A-Za-z_]\w*)\b",
            SymbolKind::Struct,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:pub\s+)?enum\s+([A-Za-z_]\w*)\b",
            SymbolKind::Enum,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:pub\s+)?trait\s+([A-Za-z_]\w*)\b",
            SymbolKind::Trait,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:pub\s+)?type\s+([A-Za-z_]\w*)\b",
            SymbolKind::TypeAlias,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:pub\s+)?const\s+([A-Za-z_]\w*)\b",
            SymbolKind::Const,
        )
    })
}

fn parse_python_symbol(line: &str) -> Option<(SymbolKind, String, String)> {
    capture_decl(
        line,
        r"^\s*(?:async\s+def|def)\s+([A-Za-z_]\w*)\s*\(",
        SymbolKind::Function,
    )
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*class\s+([A-Za-z_]\w*)\b",
            SymbolKind::Class,
        )
    })
}

fn parse_go_symbol(line: &str) -> Option<(SymbolKind, String, String)> {
    capture_decl(
        line,
        r"^\s*func\s+(?:\([^)]+\)\s*)?([A-Za-z_]\w*)\s*\(",
        SymbolKind::Function,
    )
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*type\s+([A-Za-z_]\w*)\s+struct\b",
            SymbolKind::Struct,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*type\s+([A-Za-z_]\w*)\s+interface\b",
            SymbolKind::Interface,
        )
    })
}

fn parse_js_like_symbol(line: &str) -> Option<(SymbolKind, String, String)> {
    capture_decl(
        line,
        r"^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(",
        SymbolKind::Function,
    )
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*=>",
            SymbolKind::Function,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:export\s+)?class\s+([A-Za-z_$][\w$]*)\b",
            SymbolKind::Class,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)\b",
            SymbolKind::Interface,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)\b",
            SymbolKind::TypeAlias,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:public\s+|private\s+|protected\s+|static\s+|async\s+)*([A-Za-z_$][\w$]*)\s*\([^;]*\)\s*(?::\s*[^={]+)?\s*\{?$",
            SymbolKind::Method,
        )
        .filter(|(_, name, _)| !is_reserved_symbol_name(name))
    })
}

fn parse_generic_symbol(line: &str) -> Option<(SymbolKind, String, String)> {
    capture_decl(
        line,
        r"^\s*(?:public\s+)?class\s+([A-Za-z_]\w*)\b",
        SymbolKind::Class,
    )
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:public\s+)?interface\s+([A-Za-z_]\w*)\b",
            SymbolKind::Interface,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:public\s+)?struct\s+([A-Za-z_]\w*)\b",
            SymbolKind::Struct,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*(?:public\s+)?enum\s+([A-Za-z_]\w*)\b",
            SymbolKind::Enum,
        )
    })
    .or_else(|| {
        capture_decl(
            line,
            r"^\s*[\w:<>\[\],&*\s]+\s+([A-Za-z_]\w*)\s*\([^;{}]*\)\s*\{?$",
            SymbolKind::Function,
        )
        .filter(|(_, name, _)| !is_reserved_symbol_name(name))
    })
}

fn capture_decl(line: &str, pattern: &str, kind: SymbolKind) -> Option<(SymbolKind, String, String)> {
    let regex = Regex::new(pattern).ok()?;
    let caps = regex.captures(line)?;
    let name = caps.get(1)?.as_str().to_string();
    Some((kind, name, line.trim().to_string()))
}

fn is_reserved_symbol_name(name: &str) -> bool {
    matches!(name, "if" | "for" | "while" | "loop" | "match" | "switch" | "catch")
}

fn normalize_signature(signature: &str) -> String {
    signature.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_changed_snippet(file: &FileChange) -> Option<String> {
    file.added_lines
        .iter()
        .map(|line| format!("+ {}", line.trim()))
        .find(|line| !line.trim().eq("+"))
        .or_else(|| {
            file.removed_lines
                .iter()
                .map(|line| format!("- {}", line.trim()))
                .find(|line| !line.trim().eq("-"))
        })
}

fn format_hunk_ranges(hunks: &[HunkRange]) -> String {
    if hunks.is_empty() {
        return "none".to_string();
    }
    hunks
        .iter()
        .map(|h| {
            if h.start == h.end {
                format!("L{}", h.start)
            } else {
                format!("L{}-{}", h.start, h.end)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn symbol_kind_label(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "method",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Class => "class",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Const => "const",
    }
}

/// Format a conflict report as a human-readable text block for the chat.
pub fn format_conflict_report_text(report: &ConflictReport) -> String {
    let mut lines = vec!["Conflict Analysis".to_string(), String::new()];
    for overlap in &report.overlaps {
        let icon = match overlap.resolution {
            OverlapResolution::AutoMerge => "  ✓",
            OverlapResolution::AutoReconcile => "  ↺",
            OverlapResolution::RequiresReview => "  ✗",
        };
        lines.push(format!("{icon} {}", overlap.details));
    }
    if report.requires_review() {
        lines.push(String::new());
        lines.push("Approve merge? [y/n]".to_string());
    } else if report
        .overlaps
        .iter()
        .any(|o| matches!(o.resolution, OverlapResolution::AutoReconcile))
    {
        lines.push(String::new());
        lines.push("Complementary conflicts were auto-reconciled.".to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(path: &str, kind: ChangeKind, hunks: Vec<HunkRange>) -> FileChange {
        FileChange {
            path: path.into(),
            kind,
            hunks,
            added_lines: Vec::new(),
            removed_lines: Vec::new(),
            symbols: Vec::new(),
            signature_changes: Vec::new(),
        }
    }

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
        assert!(files[0].added_lines.is_empty());
        assert!(files[0].removed_lines.is_empty());
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
        assert_eq!(files[0].hunks[0], HunkRange { start: 1, end: 1 });
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
    fn enrich_symbols_finds_rust_function() {
        let mut file = make_file(
            "src/lib.rs",
            ChangeKind::Modified,
            vec![HunkRange { start: 2, end: 4 }],
        );
        let new = "fn untouched() {}\nfn shared(user: &User) {\n    let x = 1;\n}\n";
        enrich_file_change_semantics(&mut file, None, Some(new));
        assert_eq!(file.symbols.len(), 1);
        assert_eq!(file.symbols[0].name, "shared");
        assert_eq!(file.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn detect_signature_change_from_diff_lines() {
        let mut file = make_file(
            "src/lib.rs",
            ChangeKind::Modified,
            vec![HunkRange { start: 1, end: 1 }],
        );
        file.removed_lines = vec!["pub fn shared(user: &User) -> Result<()> {".into()];
        file.added_lines = vec!["pub fn shared(user: &User, ctx: &Ctx) -> Result<()> {".into()];
        enrich_file_change_semantics(&mut file, None, None);
        assert_eq!(file.signature_changes.len(), 1);
        assert_eq!(file.signature_changes[0].name, "shared");
    }

    #[test]
    fn cross_agent_no_overlap() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                files: vec![make_file(
                    "src/a.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 1, end: 10 }],
                )],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                files: vec![make_file(
                    "src/b.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 1, end: 10 }],
                )],
            },
        ];
        let report = cross_agent_check(&changes);
        assert!(report.overlaps.is_empty());
        assert!(!report.requires_review());
    }

    #[test]
    fn cross_agent_same_file_no_hunk_overlap() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                files: vec![make_file(
                    "src/shared.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 1, end: 5 }],
                )],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                files: vec![make_file(
                    "src/shared.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 50, end: 60 }],
                )],
            },
        ];
        let report = cross_agent_check(&changes);
        assert_eq!(report.overlaps.len(), 1);
        assert_eq!(report.overlaps[0].resolution, OverlapResolution::AutoMerge);
        assert!(!report.requires_review());
    }

    #[test]
    fn cross_agent_same_symbol_non_overlapping_hunks_auto_reconcile() {
        let file_a = FileChange {
            symbols: vec![ChangedSymbol {
                name: "shared".into(),
                kind: SymbolKind::Function,
                signature: "fn shared(user: &User) {".into(),
                range: HunkRange { start: 10, end: 60 },
            }],
            ..make_file(
                "src/shared.rs",
                ChangeKind::Modified,
                vec![HunkRange { start: 12, end: 14 }],
            )
        };
        let file_b = FileChange {
            symbols: vec![ChangedSymbol {
                name: "shared".into(),
                kind: SymbolKind::Function,
                signature: "fn shared(user: &User) {".into(),
                range: HunkRange { start: 10, end: 60 },
            }],
            ..make_file(
                "src/shared.rs",
                ChangeKind::Modified,
                vec![HunkRange { start: 40, end: 42 }],
            )
        };
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                files: vec![file_a],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                files: vec![file_b],
            },
        ];
        let report = cross_agent_check(&changes);
        assert_eq!(
            report.overlaps[0].resolution,
            OverlapResolution::AutoReconcile
        );
        assert!(!report.requires_review());
        assert!(report.overlaps[0].details.contains("shared symbols"));
    }

    #[test]
    fn cross_agent_blocking_overlap() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let changes = vec![
            AgentChanges {
                agent_id: id_a,
                task: "task-a".into(),
                files: vec![make_file(
                    "src/shared.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 10, end: 20 }],
                )],
            },
            AgentChanges {
                agent_id: id_b,
                task: "task-b".into(),
                files: vec![make_file(
                    "src/shared.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 15, end: 25 }],
                )],
            },
        ];
        let report = cross_agent_check(&changes);
        assert_eq!(report.overlaps.len(), 1);
        assert_eq!(
            report.overlaps[0].resolution,
            OverlapResolution::RequiresReview
        );
        assert!(report.requires_review());
        assert_eq!(report.overlaps[0].participants.len(), 2);
    }

    #[test]
    fn cross_agent_both_add_same_file() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                files: vec![make_file(
                    "src/new.rs",
                    ChangeKind::Added,
                    vec![HunkRange { start: 1, end: 10 }],
                )],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                files: vec![make_file(
                    "src/new.rs",
                    ChangeKind::Added,
                    vec![HunkRange { start: 1, end: 5 }],
                )],
            },
        ];
        let report = cross_agent_check(&changes);
        assert!(report.requires_review());
    }

    #[test]
    fn cross_agent_interface_drift_blocks_cross_file() {
        let mut api_file = make_file(
            "src/api.rs",
            ChangeKind::Modified,
            vec![HunkRange { start: 1, end: 2 }],
        );
        api_file.removed_lines = vec!["pub fn shared(user: &User) -> Result<()> {".into()];
        api_file.added_lines = vec!["pub fn shared(user: &User, ctx: &Ctx) -> Result<()> {".into()];
        enrich_file_change_semantics(&mut api_file, None, None);

        let mut consumer_file = make_file(
            "src/consumer.rs",
            ChangeKind::Modified,
            vec![HunkRange { start: 20, end: 21 }],
        );
        consumer_file.added_lines = vec!["let result = shared(user);".into()];

        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                files: vec![api_file],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                files: vec![consumer_file],
            },
        ];

        let report = cross_agent_check(&changes);
        assert!(report.overlaps.iter().any(|o| o.details.contains("interface drift")));
        assert!(report.requires_review());
    }

    #[test]
    fn cross_agent_delete_vs_modify() {
        let changes = vec![
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-a".into(),
                files: vec![make_file("src/doomed.rs", ChangeKind::Deleted, vec![])],
            },
            AgentChanges {
                agent_id: Uuid::new_v4(),
                task: "task-b".into(),
                files: vec![make_file(
                    "src/doomed.rs",
                    ChangeKind::Modified,
                    vec![HunkRange { start: 1, end: 10 }],
                )],
            },
        ];
        let report = cross_agent_check(&changes);
        assert!(report.requires_review());
    }

    #[test]
    fn auto_reconcile_report_does_not_prompt_for_approval() {
        let report = ConflictReport {
            overlaps: vec![Overlap {
                participants: vec![],
                resolution: OverlapResolution::AutoReconcile,
                details: "src/shared.rs — shared symbols".into(),
            }],
        };

        let text = format_conflict_report_text(&report);
        assert!(text.contains("Complementary conflicts were auto-reconciled."));
        assert!(!text.contains("Approve merge? [y/n]"));
    }
}
