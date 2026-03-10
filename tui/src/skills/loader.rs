//! Load skills from disk — project + global directories.

use std::path::Path;

use super::builtins::builtin_skills;
use super::types::{ResponseFormat, Skill, SkillSource};

/// Maximum size of a single $INCLUDE file (50KB).
const MAX_INCLUDE_SIZE: u64 = 50 * 1024;
/// Maximum total size of all $INCLUDE files in one skill (200KB).
const MAX_TOTAL_INCLUDE_SIZE: u64 = 200 * 1024;

/// Load all skills: merge builtins + user skills, filter disabled.
pub fn load_all_skills(workspace: &Path, disabled: &[String]) -> Vec<Skill> {
    let user_skills = load_user_skills(workspace);
    load_all_skills_inner(user_skills, disabled)
}

/// Inner merge logic (testable without disk).
fn load_all_skills_inner(user_skills: Vec<Skill>, disabled: &[String]) -> Vec<Skill> {
    let user_names: std::collections::HashSet<String> =
        user_skills.iter().map(|s| s.name.clone()).collect();

    let mut all: Vec<Skill> = builtin_skills()
        .into_iter()
        .filter(|s| !user_names.contains(&s.name))
        .collect();
    all.extend(user_skills);

    // Filter disabled (case-insensitive)
    let disabled_lower: std::collections::HashSet<String> =
        disabled.iter().map(|s| s.to_lowercase()).collect();
    all.retain(|s| !disabled_lower.contains(&s.name));

    all.sort_by(|a, b| a.name.cmp(&b.name));
    all
}

/// Load user skills from project + global directories.
/// Project skills override global with the same name.
pub fn load_user_skills(workspace: &Path) -> Vec<Skill> {
    let global_dir = dirs::config_dir().map(|d| d.join("caboose"));
    load_user_skills_with_global(workspace, global_dir.as_deref().unwrap_or(Path::new("")))
}

/// Load with explicit global dir (for testing).
pub fn load_user_skills_with_global(workspace: &Path, global_dir: &Path) -> Vec<Skill> {
    let mut skills_by_name: std::collections::HashMap<String, Skill> =
        std::collections::HashMap::new();

    // Scan in reverse priority order (global first, project overrides)
    let dirs_to_scan = [
        global_dir.join("commands"),
        global_dir.join("skills"),
        workspace.join(".caboose").join("commands"),
        workspace.join(".caboose").join("skills"),
    ];

    for dir in &dirs_to_scan {
        if !dir.is_dir() {
            continue;
        }
        for skill in scan_directory(dir) {
            skills_by_name.insert(skill.name.clone(), skill);
        }
    }

    let mut skills: Vec<Skill> = skills_by_name.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Load with validation against built-in command names.
#[allow(dead_code)]
pub fn load_user_skills_validated(workspace: &Path, command_names: &[&str]) -> Vec<Skill> {
    let mut skills = load_user_skills(workspace);
    skills.retain(|s| {
        if command_names
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&s.name))
        {
            tracing::warn!(
                "Skipping skill '{}' — name conflicts with built-in command",
                s.name
            );
            false
        } else {
            true
        }
    });
    skills
}

/// Scan a single directory for .md files and folder skills.
fn scan_directory(dir: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return skills,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files/dirs
        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            // Folder skill — must contain skill.md
            let skill_md = path.join("skill.md");
            if skill_md.is_file()
                && let Some(mut skill) = load_skill_file(&skill_md)
            {
                skill.name = name.to_lowercase();
                skill.template = expand_includes(&skill.template, &path);
                skills.push(skill);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md")
            && let Some(mut skill) = load_skill_file(&path)
        {
            // Name from filename without extension
            let file_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_lowercase();
            if skill.name.is_empty() || skill.name == "unknown" {
                skill.name = file_name;
            }
            skills.push(skill);
        }
    }
    skills
}

/// Load and parse a single skill .md file.
fn load_skill_file(path: &Path) -> Option<Skill> {
    let content = std::fs::read_to_string(path).ok()?;
    let (frontmatter, body) = strip_frontmatter(&content);

    let name = frontmatter.get("name").cloned().unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_lowercase()
    });

    let description = frontmatter
        .get("description")
        .cloned()
        .unwrap_or_else(|| first_line_description(body));

    let response_format = match frontmatter
        .get("responseFormat")
        .or(frontmatter.get("response_format"))
    {
        Some(f) if f == "plan" => ResponseFormat::Plan,
        _ => ResponseFormat::Prose,
    };

    Some(Skill {
        name,
        description,
        template: body.to_string(),
        source: SkillSource::File(path.to_path_buf()),
        response_format,
    })
}

/// Strip YAML frontmatter between `---` markers.
/// Returns (parsed key-value pairs, remaining body).
fn strip_frontmatter(content: &str) -> (std::collections::HashMap<String, String>, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (std::collections::HashMap::new(), content);
    }

    let after_first = &trimmed[3..];
    if let Some(end) = after_first.find("\n---") {
        let fm_text = &after_first[..end];
        let body = &after_first[end + 4..]; // skip \n---

        let mut map = std::collections::HashMap::new();
        for line in fm_text.lines() {
            let line = line.trim();
            if let Some((key, val)) = line.split_once(':') {
                let val = val.trim().trim_matches('"').trim_matches('\'');
                map.insert(key.trim().to_string(), val.to_string());
            }
        }
        (map, body.trim_start_matches('\n'))
    } else {
        (std::collections::HashMap::new(), content)
    }
}

/// Extract description from first non-empty line, stripping markdown headers.
fn first_line_description(body: &str) -> String {
    body.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim().to_string())
        .unwrap_or_default()
}

/// Expand `$INCLUDE(filename)` directives in a skill template.
pub fn expand_includes(content: &str, folder: &Path) -> String {
    let re = regex::Regex::new(r"\$INCLUDE\(([^)]+)\)").unwrap();
    let mut result = content.to_string();
    let mut total_size: u64 = 0;

    // Collect matches first to avoid borrow issues
    let matches: Vec<(String, String)> = re
        .captures_iter(content)
        .map(|cap| (cap[0].to_string(), cap[1].to_string()))
        .collect();

    for (full_match, filename) in matches {
        let replacement = match resolve_include(&filename, folder, &mut total_size) {
            Ok(content) => content,
            Err(msg) => format!("[ERROR: Could not include {filename}: {msg}]"),
        };
        result = result.replacen(&full_match, &replacement, 1);
    }
    result
}

/// Resolve a single $INCLUDE, enforcing safety limits.
fn resolve_include(filename: &str, folder: &Path, total_size: &mut u64) -> Result<String, String> {
    let path = Path::new(filename);

    // Reject absolute paths
    if path.is_absolute() {
        return Err("absolute paths not allowed".into());
    }
    // Reject path traversal
    if filename.contains("..") {
        return Err("path traversal not allowed".into());
    }

    let full_path = folder.join(filename);

    // Check file size
    let metadata = std::fs::metadata(&full_path).map_err(|e| format!("file not found: {e}"))?;
    if metadata.len() > MAX_INCLUDE_SIZE {
        return Err(format!(
            "file too large ({} bytes, max {})",
            metadata.len(),
            MAX_INCLUDE_SIZE
        ));
    }
    *total_size += metadata.len();
    if *total_size > MAX_TOTAL_INCLUDE_SIZE {
        return Err(format!(
            "total includes too large ({total_size} bytes, max {MAX_TOTAL_INCLUDE_SIZE})"
        ));
    }

    std::fs::read_to_string(&full_path).map_err(|e| format!("read error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_single_md_file_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("deploy.md"),
            "---\nname: deploy\ndescription: Deploy to staging\n---\nDeploy $ARGS to staging.",
        )
        .unwrap();

        let skills = load_user_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "deploy");
        assert_eq!(skills[0].description, "Deploy to staging");
        assert!(skills[0].template.contains("Deploy $ARGS"));
        assert!(!skills[0].template.contains("---")); // frontmatter stripped
    }

    #[test]
    fn load_file_without_frontmatter_uses_first_line() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("quick.md"),
            "# Quick check\nRun a quick check on $ARGS.",
        )
        .unwrap();

        let skills = load_user_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "quick");
        assert_eq!(skills[0].description, "Quick check"); // # stripped
    }

    #[test]
    fn load_folder_skill() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join(".caboose").join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.md"),
            "My skill\nDo $ARGS.\n$INCLUDE(ref.md)",
        )
        .unwrap();
        std::fs::write(skill_dir.join("ref.md"), "Reference content here.").unwrap();

        let skills = load_user_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
        assert!(skills[0].template.contains("Reference content here."));
    }

    #[test]
    fn include_rejects_path_traversal() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join(".caboose").join("skills").join("evil");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.md"),
            "Evil\n$INCLUDE(../../../etc/passwd)",
        )
        .unwrap();

        let skills = load_user_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert!(skills[0].template.contains("[ERROR: Could not include"));
    }

    #[test]
    fn include_rejects_absolute_path() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join(".caboose").join("skills").join("abs");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("skill.md"), "Abs\n$INCLUDE(/etc/passwd)").unwrap();

        let skills = load_user_skills(dir.path());
        assert!(skills[0].template.contains("[ERROR: Could not include"));
    }

    #[test]
    fn project_skills_override_global() {
        let global = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();

        let global_skills = global.path().join("skills");
        std::fs::create_dir_all(&global_skills).unwrap();
        std::fs::write(
            global_skills.join("review.md"),
            "Global review\nGlobal version.",
        )
        .unwrap();

        let project_skills = project.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&project_skills).unwrap();
        std::fs::write(
            project_skills.join("review.md"),
            "Project review\nProject version.",
        )
        .unwrap();

        let skills = load_user_skills_with_global(project.path(), global.path());
        let review: Vec<_> = skills.iter().filter(|s| s.name == "review").collect();
        assert_eq!(review.len(), 1);
        assert!(review[0].template.contains("Project version"));
    }

    #[test]
    fn rejects_skill_with_builtin_command_name() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("model.md"), "Bad\nThis shadows /model.").unwrap();

        let commands = vec![
            "model", "connect", "compact", "new", "sessions", "title", "memories", "forget", "mcp",
            "settings", "init", "quit", "skills",
        ];
        let skills = load_user_skills_validated(dir.path(), &commands);
        assert!(skills.is_empty(), "Should reject skill named 'model'");
    }

    #[test]
    fn load_all_merges_builtins_and_user() {
        let dir = TempDir::new().unwrap();
        // No user skills — should get all 11 builtins
        let all = load_all_skills(dir.path(), &[]);
        assert_eq!(all.len(), 11);
    }

    #[test]
    fn disabled_skills_are_filtered() {
        let dir = TempDir::new().unwrap();
        let all = load_all_skills(dir.path(), &["review".to_string(), "DEBUG".to_string()]);
        assert!(!all.iter().any(|s| s.name == "review"));
        assert!(!all.iter().any(|s| s.name == "debug"));
        assert_eq!(all.len(), 9);
    }

    #[test]
    fn hidden_files_ignored() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join(".hidden.md"), "Hidden\nShould not load.").unwrap();

        let skills = load_user_skills(dir.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn skills_sorted_by_name() {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join(".caboose").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("zebra.md"), "Zebra\nZ skill.").unwrap();
        std::fs::write(skills_dir.join("alpha.md"), "Alpha\nA skill.").unwrap();

        let skills = load_user_skills(dir.path());
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[1].name, "zebra");
    }
}
