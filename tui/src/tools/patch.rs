//! Apply Patch tool — applies unified diffs to one or more files.

use anyhow::Result;
use serde_json::Value;

use crate::agent::tools::ToolResult;

/// Apply a unified diff to one or more files.
pub async fn execute(input: &Value) -> Result<ToolResult> {
    let raw_diff = match input.get("diff").and_then(|v| v.as_str()) {
        Some(d) if !d.is_empty() => d,
        _ => {
            return Ok(ToolResult {
                tool_use_id: String::new(),
                output: "Missing or empty 'diff' parameter".to_string(),
                is_error: true,
                tool_name: None,
                file_path: None,
                files_modified: vec![],
                lines_added: 0,
                lines_removed: 0,
            });
        }
    };

    // Strip markdown fences if the model wrapped the diff
    let diff_text = strip_fences(raw_diff);

    let files = parse_patch(&diff_text);
    if files.is_empty() {
        // Include a preview of what we received for debugging
        let preview: String = diff_text.lines().take(5).collect::<Vec<_>>().join("\n");
        return Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!(
                "Could not parse any file entries from the diff. \
                 Expected standard unified diff format with '--- a/path' and '+++ b/path' headers.\n\
                 First lines received:\n{preview}"
            ),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        });
    }

    let mut applied: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut modified_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut total_added: usize = 0;
    let mut total_removed: usize = 0;

    for entry in &files {
        match apply_file_entry(entry).await {
            Ok(msg) => {
                applied.push(msg);
                modified_paths.push(std::path::PathBuf::from(&entry.file_path));
                // Count +/- lines from hunks
                for hunk in &entry.hunks {
                    for line in &hunk.lines {
                        if line.starts_with('+') {
                            total_added += 1;
                        } else if line.starts_with('-') {
                            total_removed += 1;
                        }
                    }
                }
            }
            Err(e) => errors.push(format!("{}: {e}", entry.file_path)),
        }
    }

    if errors.is_empty() {
        Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!(
                "Applied patch to {} file(s):\n{}",
                applied.len(),
                applied.join("\n")
            ),
            is_error: false,
            tool_name: None,
            file_path: None,
            files_modified: modified_paths,
            lines_added: total_added,
            lines_removed: total_removed,
        })
    } else if !applied.is_empty() {
        Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!(
                "Partial success — applied {} file(s), {} failed:\nApplied:\n{}\nFailed:\n{}",
                applied.len(),
                errors.len(),
                applied.join("\n"),
                errors.join("\n")
            ),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: modified_paths,
            lines_added: total_added,
            lines_removed: total_removed,
        })
    } else {
        Ok(ToolResult {
            tool_use_id: String::new(),
            output: format!("Patch failed:\n{}", errors.join("\n")),
            is_error: true,
            tool_name: None,
            file_path: None,
            files_modified: vec![],
            lines_added: 0,
            lines_removed: 0,
        })
    }
}

/// Apply a single file entry from the patch.
async fn apply_file_entry(entry: &PatchFileEntry) -> Result<String> {
    match entry.status {
        FileStatus::Deleted => {
            tokio::fs::remove_file(&entry.file_path).await?;
            Ok(format!("  deleted {}", entry.file_path))
        }
        FileStatus::Added => {
            let content = extract_added_lines(&entry.hunks);
            if let Some(parent) = std::path::Path::new(&entry.file_path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&entry.file_path, &content).await?;
            Ok(format!(
                "  created {} ({} bytes)",
                entry.file_path,
                content.len()
            ))
        }
        FileStatus::Modified => {
            let original = tokio::fs::read_to_string(&entry.file_path).await?;
            let patched = apply_hunks(&original, &entry.hunks)
                .ok_or_else(|| anyhow::anyhow!("hunks did not apply cleanly"))?;
            tokio::fs::write(&entry.file_path, &patched).await?;
            Ok(format!("  modified {}", entry.file_path))
        }
    }
}

// ---------------------------------------------------------------------------
// Fence stripping
// ---------------------------------------------------------------------------
/// Strip markdown code fences (```diff ... ```, ```patch ... ```) that models often wrap diffs in.
fn strip_fences(text: &str) -> String {
    let trimmed = text.trim();

    // Try to find ```diff or ```patch at the start
    for prefix in &["```diff", "```patch"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(['\n', '\r']);
            if let Some(end) = rest.rfind("```") {
                return rest[..end].trim().to_string();
            }
            return rest.trim().to_string();
        }
    }

    if let Some(rest) = trimmed.strip_prefix("```") {
        let rest = rest.trim_start_matches(['\n', '\r']);
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim().to_string();
        }
        return rest.trim().to_string();
    }

    text.to_string()
}

// ---------------------------------------------------------------------------
// Unified diff parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone)]
struct PatchFileEntry {
    file_path: String,
    status: FileStatus,
    hunks: Vec<Hunk>,
}

#[derive(Debug, Clone)]
struct Hunk {
    original_start: usize,
    #[allow(dead_code)]
    original_count: usize,
    lines: Vec<String>,
}

/// Parse a unified diff string into file entries.
fn parse_patch(diff_text: &str) -> Vec<PatchFileEntry> {
    let lines: Vec<&str> = diff_text.lines().collect();
    let mut files = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].starts_with("--- ")
            && let Some((entry, next)) = parse_file_entry(&lines, i)
        {
            files.push(entry);
            i = next;
            continue;
        }
        i += 1;
    }

    files
}

fn parse_file_entry(lines: &[&str], start: usize) -> Option<(PatchFileEntry, usize)> {
    let minus_line = lines.get(start)?;
    let plus_line = lines.get(start + 1)?;

    if !minus_line.starts_with("--- ") || !plus_line.starts_with("+++ ") {
        return None;
    }

    let old_path = extract_path(&minus_line[4..]);
    let new_path = extract_path(&plus_line[4..]);

    let (status, file_path) = if old_path == "/dev/null" {
        (FileStatus::Added, new_path)
    } else if new_path == "/dev/null" {
        (FileStatus::Deleted, old_path)
    } else {
        (FileStatus::Modified, new_path)
    };

    let mut hunks = Vec::new();
    let mut i = start + 2;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("--- ") || line.starts_with("diff --git") {
            break;
        }
        if line.starts_with("@@ ")
            && let Some((hunk, next)) = parse_hunk(lines, i)
        {
            hunks.push(hunk);
            i = next;
            continue;
        }
        i += 1;
    }

    Some((
        PatchFileEntry {
            file_path,
            status,
            hunks,
        },
        i,
    ))
}

fn parse_hunk(lines: &[&str], start: usize) -> Option<(Hunk, usize)> {
    let header = lines[start];
    // @@ -oldStart,oldCount +newStart,newCount @@
    let (orig_start, orig_count, _new_start, new_count) = parse_hunk_header(header)?;

    let mut hunk_lines = Vec::new();
    let mut i = start + 1;
    let mut orig_seen = 0usize;
    let mut new_seen = 0usize;

    while i < lines.len() && (orig_seen < orig_count || new_seen < new_count) {
        let line = lines[i];
        if line.starts_with('\\') {
            // "\ No newline at end of file"
            hunk_lines.push(line.to_string());
            i += 1;
        } else if line.starts_with('-') {
            hunk_lines.push(line.to_string());
            orig_seen += 1;
            i += 1;
        } else if line.starts_with('+') {
            hunk_lines.push(line.to_string());
            new_seen += 1;
            i += 1;
        } else if line.starts_with(' ') {
            hunk_lines.push(line.to_string());
            orig_seen += 1;
            new_seen += 1;
            i += 1;
        } else {
            break;
        }
    }

    Some((
        Hunk {
            original_start: orig_start,
            original_count: orig_count,
            lines: hunk_lines,
        },
        i,
    ))
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize)> {
    // @@ -1,5 +1,7 @@
    let after_at = header.strip_prefix("@@ ")?;
    let end = after_at.find(" @@")?;
    let range_part = &after_at[..end]; // "-1,5 +1,7"

    let parts: Vec<&str> = range_part.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }

    let (orig_start, orig_count) = parse_range(parts[0].strip_prefix('-')?)?;
    let (new_start, new_count) = parse_range(parts[1].strip_prefix('+')?)?;

    Some((orig_start, orig_count, new_start, new_count))
}

fn parse_range(s: &str) -> Option<(usize, usize)> {
    if let Some((start, count)) = s.split_once(',') {
        Some((start.parse().ok()?, count.parse().ok()?))
    } else {
        Some((s.parse().ok()?, 1))
    }
}

/// Strip a/ or b/ prefix from paths, handle quoted paths.
fn extract_path(raw: &str) -> String {
    let mut p = raw.trim().to_string();
    if p.starts_with('"') && p.ends_with('"') {
        p = p[1..p.len() - 1].to_string();
    }
    if p.starts_with("a/") || p.starts_with("b/") {
        p = p[2..].to_string();
    }
    p
}

// ---------------------------------------------------------------------------
// Patch application
// ---------------------------------------------------------------------------

/// Extract content for new files from '+' lines.
fn extract_added_lines(hunks: &[Hunk]) -> String {
    let mut lines = Vec::new();
    for hunk in hunks {
        for line in &hunk.lines {
            if let Some(stripped) = line.strip_prefix('+') {
                lines.push(stripped);
            }
        }
    }
    lines.join("\n")
}

/// Apply hunks to original content, producing new content.
/// Returns None if hunks don't apply cleanly.
fn apply_hunks(original: &str, hunks: &[Hunk]) -> Option<String> {
    let mut result: Vec<String> = original.lines().map(|s| s.to_string()).collect();

    // Apply hunks in reverse order so line numbers stay valid
    let mut sorted_hunks: Vec<&Hunk> = hunks.iter().collect();
    sorted_hunks.sort_by(|a, b| b.original_start.cmp(&a.original_start));

    for hunk in sorted_hunks {
        // Extract the expected original lines (context + removed) from the hunk
        let expected_old: Vec<&str> = hunk
            .lines
            .iter()
            .filter_map(|line| line.strip_prefix(' ').or_else(|| line.strip_prefix('-')))
            .collect();

        let new_lines: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|line| {
                if line.starts_with('\\') || line.starts_with('-') {
                    None
                } else {
                    line.strip_prefix('+')
                        .or_else(|| line.strip_prefix(' '))
                        .map(ToString::to_string)
                }
            })
            .collect();

        let declared_idx = hunk.original_start.saturating_sub(1);
        let old_count = expected_old.len();

        // Find the correct position: try declared position first, then search nearby
        let start_idx = if context_matches(&result, declared_idx, &expected_old) {
            declared_idx
        } else {
            find_context_match(&result, declared_idx, &expected_old)?
        };

        if start_idx + old_count > result.len() {
            return None;
        }

        result.splice(start_idx..start_idx + old_count, new_lines);
    }

    Some(result.join("\n"))
}

/// Check if expected context/removed lines match the file at the given position.
fn context_matches(lines: &[String], start: usize, expected: &[&str]) -> bool {
    if start + expected.len() > lines.len() {
        return false;
    }
    for (i, exp) in expected.iter().enumerate() {
        if lines[start + i] != *exp {
            return false;
        }
    }
    true
}

/// Search ±30 lines around the declared position for a context match.
fn find_context_match(lines: &[String], declared: usize, expected: &[&str]) -> Option<usize> {
    if expected.is_empty() {
        return Some(declared.min(lines.len()));
    }
    let max_offset = 30;
    for offset in 1..=max_offset {
        // Search forward
        let fwd = declared + offset;
        if fwd + expected.len() <= lines.len() && context_matches(lines, fwd, expected) {
            return Some(fwd);
        }
        // Search backward
        if offset <= declared {
            let bwd = declared - offset;
            if bwd + expected.len() <= lines.len() && context_matches(lines, bwd, expected) {
                return Some(bwd);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_modify() {
        let diff = "\
--- a/hello.txt
+++ b/hello.txt
@@ -1,3 +1,3 @@
 line1
-line2
+LINE2
 line3";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "hello.txt");
        assert_eq!(files[0].status, FileStatus::Modified);
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn parse_new_file() {
        let diff = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "new.txt");
        assert_eq!(files[0].status, FileStatus::Added);
    }

    #[test]
    fn parse_deleted_file() {
        let diff = "\
--- a/old.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-goodbye
-world";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "old.txt");
        assert_eq!(files[0].status, FileStatus::Deleted);
    }

    #[test]
    fn parse_multi_file() {
        let diff = "\
--- a/a.txt
+++ b/a.txt
@@ -1,1 +1,1 @@
-old
+new
--- a/b.txt
+++ b/b.txt
@@ -1,1 +1,1 @@
-foo
+bar";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_path, "a.txt");
        assert_eq!(files[1].file_path, "b.txt");
    }

    #[test]
    fn apply_simple_hunk() {
        let original = "line1\nline2\nline3";
        let hunks = vec![Hunk {
            original_start: 1,
            original_count: 3,
            lines: vec![
                " line1".to_string(),
                "-line2".to_string(),
                "+LINE2".to_string(),
                " line3".to_string(),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line1\nLINE2\nline3");
    }

    #[test]
    fn extract_added_content() {
        let hunks = vec![Hunk {
            original_start: 0,
            original_count: 0,
            lines: vec!["+hello".to_string(), "+world".to_string()],
        }];
        let content = extract_added_lines(&hunks);
        assert_eq!(content, "hello\nworld");
    }

    #[test]
    fn extract_path_strips_prefix() {
        assert_eq!(extract_path("a/src/main.rs"), "src/main.rs");
        assert_eq!(extract_path("b/src/main.rs"), "src/main.rs");
        assert_eq!(extract_path("/dev/null"), "/dev/null");
        assert_eq!(extract_path("\"a/quoted.rs\""), "quoted.rs");
    }

    #[test]
    fn parse_hunk_header_works() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -1,5 +1,7 @@").unwrap();
        assert_eq!((os, oc, ns, nc), (1, 5, 1, 7));
    }

    #[test]
    fn parse_hunk_header_single_line() {
        let (os, oc, ns, nc) = parse_hunk_header("@@ -1 +1 @@").unwrap();
        assert_eq!((os, oc, ns, nc), (1, 1, 1, 1));
    }

    #[tokio::test]
    async fn execute_missing_diff_param() {
        let result = execute(&serde_json::json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("diff"));
    }

    #[tokio::test]
    async fn execute_unparseable_diff() {
        let result = execute(&serde_json::json!({"diff": "not a diff"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("parse"));
    }

    #[test]
    fn strip_fences_removes_diff_fence() {
        let input = "```diff\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new\n```";
        let result = strip_fences(input);
        assert!(result.starts_with("--- a/f.txt"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn strip_fences_removes_plain_fence() {
        let input = "```\n--- a/f.txt\n+++ b/f.txt\n```";
        let result = strip_fences(input);
        assert!(result.starts_with("--- a/f.txt"));
    }

    #[test]
    fn strip_fences_noop_for_bare_diff() {
        let input = "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new";
        let result = strip_fences(input);
        assert_eq!(result, input);
    }

    #[test]
    fn parse_bare_paths_without_ab_prefix() {
        let diff = "\
--- /tmp/patch-test.txt
+++ /tmp/patch-test.txt
@@ -1,3 +1,3 @@
 line1
-line2
+LINE2
 line3";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "/tmp/patch-test.txt");
    }

    #[test]
    fn parse_fenced_diff() {
        let raw = "```diff\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-old\n+new\n```";
        let cleaned = strip_fences(raw);
        let files = parse_patch(&cleaned);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "f.txt");
    }

    #[test]
    fn apply_hunk_rejects_wrong_context() {
        // Hunk says line 1 is "aaa" but file has "xxx" — should fail
        let original = "xxx\nyyy\nzzz";
        let hunks = vec![Hunk {
            original_start: 1,
            original_count: 3,
            lines: vec![
                " aaa".to_string(),
                "-bbb".to_string(),
                "+BBB".to_string(),
                " ccc".to_string(),
            ],
        }];
        assert!(apply_hunks(original, &hunks).is_none());
    }

    #[test]
    fn apply_hunk_fuzzy_finds_shifted_context() {
        // Hunk declares start at line 1 but content is actually at line 3
        let original = "header1\nheader2\nline1\nline2\nline3";
        let hunks = vec![Hunk {
            original_start: 1, // wrong — actual content starts at line 3
            original_count: 3,
            lines: vec![
                " line1".to_string(),
                "-line2".to_string(),
                "+LINE2".to_string(),
                " line3".to_string(),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "header1\nheader2\nline1\nLINE2\nline3");
    }

    #[tokio::test]
    async fn apply_patch_populates_files_modified() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "line1\nline2\nline3").unwrap();
        let file_str = file.to_str().unwrap();
        let diff = format!(
            "--- a/{file_str}\n+++ b/{file_str}\n@@ -1,3 +1,3 @@\n line1\n-line2\n+LINE2\n line3"
        );
        let result = execute(&serde_json::json!({"diff": diff})).await.unwrap();
        assert!(!result.is_error);
        assert!(!result.files_modified.is_empty());
    }

    #[test]
    fn strip_fences_removes_patch_fence() {
        let input = "```patch\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-old\n+new\n```";
        let result = strip_fences(input);
        assert!(result.starts_with("--- a/f.txt"));
        assert!(!result.contains("```"));
    }

    // -----------------------------------------------------------------------
    // LLM quirk tests — patterns commonly emitted by real models
    // -----------------------------------------------------------------------

    #[test]
    fn parse_diff_with_git_prefix_lines() {
        // Models often emit `diff --git` lines before the --- / +++ headers
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 fn main() {
-    println!(\"hello\");
+    println!(\"goodbye\");
 }";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "src/main.rs");
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn parse_multi_file_with_git_prefix() {
        let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-old_a
+new_a
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,1 @@
-old_b
+new_b";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_path, "a.rs");
        assert_eq!(files[1].file_path, "b.rs");
    }

    #[test]
    fn parse_hunk_header_with_section_label() {
        // `@@ -10,5 +10,7 @@ fn some_function()` — git adds function context
        let (os, oc, ns, nc) = parse_hunk_header("@@ -10,5 +10,7 @@ fn some_function()").unwrap();
        assert_eq!((os, oc, ns, nc), (10, 5, 10, 7));
    }

    #[test]
    fn apply_multiple_hunks_same_file() {
        // Two non-overlapping hunks in the same file
        let original = "a\nb\nc\nd\ne\nf\ng\nh";
        let hunks = vec![
            Hunk {
                original_start: 2,
                original_count: 1,
                lines: vec!["-b".to_string(), "+B".to_string()],
            },
            Hunk {
                original_start: 7,
                original_count: 1,
                lines: vec!["-g".to_string(), "+G".to_string()],
            },
        ];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "a\nB\nc\nd\ne\nf\nG\nh");
    }

    #[test]
    fn apply_hunk_pure_addition() {
        // Adding lines without removing any (common LLM pattern)
        let original = "line1\nline2\nline3";
        let hunks = vec![Hunk {
            original_start: 2,
            original_count: 1,
            lines: vec![
                " line2".to_string(),
                "+inserted_a".to_string(),
                "+inserted_b".to_string(),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line1\nline2\ninserted_a\ninserted_b\nline3");
    }

    #[test]
    fn apply_hunk_pure_deletion() {
        let original = "line1\nline2\nline3\nline4";
        let hunks = vec![Hunk {
            original_start: 2,
            original_count: 2,
            lines: vec!["-line2".to_string(), "-line3".to_string()],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line1\nline4");
    }

    #[test]
    fn apply_hunk_at_end_of_file() {
        let original = "a\nb\nc";
        let hunks = vec![Hunk {
            original_start: 3,
            original_count: 1,
            lines: vec!["-c".to_string(), "+C".to_string(), "+d".to_string()],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "a\nb\nC\nd");
    }

    #[test]
    fn apply_hunk_at_start_of_file() {
        let original = "a\nb\nc";
        let hunks = vec![Hunk {
            original_start: 1,
            original_count: 1,
            lines: vec!["-a".to_string(), "+header".to_string(), "+a".to_string()],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "header\na\nb\nc");
    }

    #[test]
    fn parse_no_newline_at_eof_marker() {
        // Models sometimes include the `\ No newline at end of file` marker
        let diff = "\
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,2 @@
 line1
-line2
\\ No newline at end of file
+LINE2
\\ No newline at end of file";
        let files = parse_patch(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hunks.len(), 1);
        // Parser should still extract the correct +/- lines
        let hunk = &files[0].hunks[0];
        assert!(hunk.lines.iter().any(|l| l == "-line2"));
        assert!(hunk.lines.iter().any(|l| l == "+LINE2"));
    }

    #[tokio::test]
    async fn end_to_end_create_and_modify() {
        // Full flow: create a file via patch, then modify it via a second patch
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("new_file.txt");
        let file_str = file.to_str().unwrap();

        // Create
        let create_diff =
            format!("--- /dev/null\n+++ b/{file_str}\n@@ -0,0 +1,3 @@\n+alpha\n+beta\n+gamma");
        let r = execute(&serde_json::json!({"diff": create_diff}))
            .await
            .unwrap();
        assert!(!r.is_error, "create failed: {}", r.output);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "alpha\nbeta\ngamma"
        );

        // Modify
        let modify_diff = format!(
            "--- a/{file_str}\n+++ b/{file_str}\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma"
        );
        let r = execute(&serde_json::json!({"diff": modify_diff}))
            .await
            .unwrap();
        assert!(!r.is_error, "modify failed: {}", r.output);
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "alpha\nBETA\ngamma"
        );
    }

    #[tokio::test]
    async fn end_to_end_delete_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doomed.txt");
        std::fs::write(&file, "goodbye\nworld").unwrap();
        let file_str = file.to_str().unwrap();

        let diff = format!("--- a/{file_str}\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-goodbye\n-world");
        let r = execute(&serde_json::json!({"diff": diff})).await.unwrap();
        assert!(!r.is_error, "delete failed: {}", r.output);
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn end_to_end_multi_file_patch() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("one.txt");
        let f2 = dir.path().join("two.txt");
        std::fs::write(&f1, "aaa\nbbb").unwrap();
        std::fs::write(&f2, "xxx\nyyy").unwrap();
        let s1 = f1.to_str().unwrap();
        let s2 = f2.to_str().unwrap();

        let diff = format!(
            "--- a/{s1}\n+++ b/{s1}\n@@ -1,2 +1,2 @@\n aaa\n-bbb\n+BBB\n\
             --- a/{s2}\n+++ b/{s2}\n@@ -1,2 +1,2 @@\n xxx\n-yyy\n+YYY"
        );
        let r = execute(&serde_json::json!({"diff": diff})).await.unwrap();
        assert!(!r.is_error, "multi-file failed: {}", r.output);
        assert_eq!(std::fs::read_to_string(&f1).unwrap(), "aaa\nBBB");
        assert_eq!(std::fs::read_to_string(&f2).unwrap(), "xxx\nYYY");
        assert_eq!(r.files_modified.len(), 2);
    }

    #[tokio::test]
    async fn fenced_diff_applies_end_to_end() {
        // LLMs commonly wrap diffs in markdown fences
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fenced.txt");
        std::fs::write(&file, "old_line").unwrap();
        let file_str = file.to_str().unwrap();

        let raw = format!(
            "```diff\n--- a/{file_str}\n+++ b/{file_str}\n@@ -1,1 +1,1 @@\n-old_line\n+new_line\n```"
        );
        let r = execute(&serde_json::json!({"diff": raw})).await.unwrap();
        assert!(!r.is_error, "fenced failed: {}", r.output);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new_line");
    }
}
