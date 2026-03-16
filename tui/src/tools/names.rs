//! Canonical tool name constants — single source of truth for built-in tool names.
//!
//! Other modules should use these constants instead of hardcoding string literals
//! to avoid drift between tool registration, permission checks, and dispatch.

// Read-only tools
pub const READ_FILE: &str = "read_file";
pub const GLOB: &str = "glob";
pub const GREP: &str = "grep";
pub const LIST_DIRECTORY: &str = "list_directory";
pub const FETCH: &str = "fetch";
pub const LSP: &str = "lsp";
pub const DIAGNOSTICS: &str = "diagnostics";
pub const WEB_SEARCH: &str = "web_search";

// Write tools
pub const WRITE_FILE: &str = "write_file";
pub const EDIT_FILE: &str = "edit_file";
pub const APPLY_PATCH: &str = "apply_patch";

// Command execution
pub const RUN_COMMAND: &str = "run_command";

// Task management
pub const TODO_WRITE: &str = "todo_write";
pub const TODO_READ: &str = "todo_read";

/// Read-only tools that auto-execute in all permission modes.
pub const READ_TOOLS: &[&str] = &[
    READ_FILE,
    GLOB,
    GREP,
    LIST_DIRECTORY,
    FETCH,
    LSP,
    DIAGNOSTICS,
    WEB_SEARCH,
];

/// File-write tools that require approval in default mode.
pub const WRITE_TOOLS: &[&str] = &[WRITE_FILE, EDIT_FILE, APPLY_PATCH];

/// Task management tools — UI-only state, auto-execute in all modes.
pub const TASK_TOOLS: &[&str] = &[TODO_WRITE, TODO_READ];

/// Read-only subset used by roundhouse planning phase.
pub const PLANNING_TOOLS: &[&str] = &[READ_FILE, GLOB, GREP, LIST_DIRECTORY];
