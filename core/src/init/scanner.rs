use std::fs;
use std::path::{Path, PathBuf};

/// Directories to skip during tree traversal.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".tox",
    ".venv",
    "venv",
    ".next",
    ".nuxt",
    "build",
    "out",
    ".caboose",
    ".claude",
];

/// Config files to detect and read.
const CONFIG_FILES: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "go.mod",
    "Makefile",
    "CMakeLists.txt",
    "Gemfile",
    "Dockerfile",
    "docker-compose.yml",
    "tsconfig.json",
];

/// Maximum number of lines to read from a config file.
const MAX_CONFIG_LINES: usize = 500;

/// Maximum directory depth for the file tree.
const MAX_TREE_DEPTH: usize = 3;

/// Maximum number of entries in the file tree.
const MAX_TREE_ENTRIES: usize = 200;

/// README file names to search for, in priority order.
const README_NAMES: &[&str] = &["README.md", "README.rst", "readme.md"];

/// Structured context gathered from scanning a repository directory.
#[derive(Debug)]
pub struct RepoContext {
    /// The root directory that was scanned.
    pub root: PathBuf,
    /// A text representation of the file tree (depth-limited).
    pub file_tree: String,
    /// Detected config files as (filename, contents) pairs.
    pub config_files: Vec<(String, String)>,
    /// Contents of the first README found, if any.
    pub readme: Option<String>,
    /// Contents of an existing CABOOSE.md, if any.
    pub existing_caboose: Option<String>,
}

/// Scan a directory and collect repo signals into a `RepoContext`.
pub fn scan(root: &Path) -> RepoContext {
    // Build file tree
    let mut entries = Vec::new();
    walk_tree(root, root, 0, &mut entries);
    let file_tree = entries.join("\n");

    // Detect config files
    let mut config_files = Vec::new();
    for name in CONFIG_FILES {
        let path = root.join(name);
        if let Some(contents) = read_optional(&path) {
            let truncated = truncate_lines(&contents, MAX_CONFIG_LINES);
            config_files.push((name.to_string(), truncated));
        }
    }

    // CI config detection: read the first .yml/.yaml from .github/workflows/
    let workflows_dir = root.join(".github").join("workflows");
    if workflows_dir.is_dir()
        && let Some((name, contents)) = read_first_workflow(&workflows_dir)
    {
        let truncated = truncate_lines(&contents, MAX_CONFIG_LINES);
        config_files.push((name, truncated));
    }

    // README detection
    let readme = README_NAMES
        .iter()
        .find_map(|name| read_optional(&root.join(name)));

    // Existing CABOOSE.md detection
    let existing_caboose = read_optional(&root.join("CABOOSE.md"));

    RepoContext {
        root: root.to_path_buf(),
        file_tree,
        config_files,
        readme,
        existing_caboose,
    }
}

/// Recursively walk a directory tree, collecting path entries up to a
/// maximum depth and entry count. Hidden files/dirs are skipped except
/// `.github`. Directories in `SKIP_DIRS` are skipped entirely.
fn walk_tree(base: &Path, dir: &Path, depth: usize, entries: &mut Vec<String>) {
    if depth > MAX_TREE_DEPTH || entries.len() >= MAX_TREE_ENTRIES {
        return;
    }

    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    // Collect and sort entries for deterministic output
    let mut dir_entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    dir_entries.sort_by_key(|e| e.file_name());

    for entry in dir_entries {
        if entries.len() >= MAX_TREE_ENTRIES {
            return;
        }

        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        // Skip hidden files/dirs except .github
        if name.starts_with('.') && name != ".github" {
            continue;
        }

        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if path.is_dir() {
            // Skip directories in the skip list
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            entries.push(format!("{}/", rel));
            walk_tree(base, &path, depth + 1, entries);
        } else {
            entries.push(rel.to_string());
        }
    }
}

/// Read a file's contents, returning `None` if it doesn't exist or can't be read.
fn read_optional(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// Truncate a string to at most `max_lines` lines.
fn truncate_lines(contents: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= max_lines {
        contents.to_string()
    } else {
        lines[..max_lines].join("\n")
    }
}

/// Read the first `.yml` or `.yaml` file from a workflows directory.
fn read_first_workflow(dir: &Path) -> Option<(String, String)> {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return None;
    };

    let mut yaml_files: Vec<_> = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".yml") || name.ends_with(".yaml")
        })
        .collect();

    yaml_files.sort_by_key(|e| e.file_name());

    if let Some(entry) = yaml_files.first() {
        let name = format!(".github/workflows/{}", entry.file_name().to_string_lossy());
        let contents = fs::read_to_string(entry.path()).ok()?;
        Some((name, contents))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn scan_detects_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\n",
        )
        .unwrap();

        let ctx = scan(tmp.path());
        assert!(
            ctx.config_files
                .iter()
                .any(|(name, _)| name == "Cargo.toml"),
            "Expected Cargo.toml to be detected"
        );
    }

    #[test]
    fn scan_detects_package_json() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), "{\"name\": \"test\"}\n").unwrap();

        let ctx = scan(tmp.path());
        assert!(
            ctx.config_files
                .iter()
                .any(|(name, _)| name == "package.json"),
            "Expected package.json to be detected"
        );
    }

    #[test]
    fn scan_reads_readme() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("README.md"), "# Hello World\n").unwrap();

        let ctx = scan(tmp.path());
        assert_eq!(ctx.readme, Some("# Hello World\n".to_string()));
    }

    #[test]
    fn scan_reads_existing_caboose_md() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("CABOOSE.md"), "# Existing config\n").unwrap();

        let ctx = scan(tmp.path());
        assert_eq!(
            ctx.existing_caboose,
            Some("# Existing config\n".to_string())
        );
    }

    #[test]
    fn scan_builds_file_tree() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(src.join("lib.rs"), "// lib\n").unwrap();

        let ctx = scan(tmp.path());
        assert!(
            ctx.file_tree.contains("src/"),
            "Expected tree to contain src/"
        );
        assert!(
            ctx.file_tree.contains("src/main.rs"),
            "Expected tree to contain src/main.rs"
        );
        assert!(
            ctx.file_tree.contains("src/lib.rs"),
            "Expected tree to contain src/lib.rs"
        );
    }

    #[test]
    fn scan_skips_gitignored_dirs() {
        let tmp = TempDir::new().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        fs::write(nm.join("foo.js"), "// foo\n").unwrap();

        let tgt = tmp.path().join("target");
        fs::create_dir(&tgt).unwrap();
        fs::write(tgt.join("debug"), "binary\n").unwrap();

        let ctx = scan(tmp.path());
        assert!(
            !ctx.file_tree.contains("node_modules"),
            "Expected node_modules to be skipped"
        );
        assert!(
            !ctx.file_tree.contains("target"),
            "Expected target to be skipped"
        );
    }

    #[test]
    fn scan_truncates_large_config() {
        let tmp = TempDir::new().unwrap();
        let lines: Vec<String> = (0..600).map(|i| format!("line {i}")).collect();
        fs::write(tmp.path().join("Cargo.toml"), lines.join("\n")).unwrap();

        let ctx = scan(tmp.path());
        let (_, contents) = ctx
            .config_files
            .iter()
            .find(|(name, _)| name == "Cargo.toml")
            .expect("Cargo.toml should be detected");

        let line_count = contents.lines().count();
        assert!(
            line_count <= MAX_CONFIG_LINES,
            "Expected at most {MAX_CONFIG_LINES} lines, got {line_count}"
        );
    }

    #[test]
    fn scan_empty_dir() {
        let tmp = TempDir::new().unwrap();

        let ctx = scan(tmp.path());
        assert!(ctx.file_tree.is_empty(), "Expected empty file tree");
        assert!(ctx.config_files.is_empty(), "Expected no config files");
        assert!(ctx.readme.is_none(), "Expected no README");
        assert!(ctx.existing_caboose.is_none(), "Expected no CABOOSE.md");
    }
}
