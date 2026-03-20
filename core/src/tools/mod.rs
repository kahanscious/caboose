//! Tool definitions, execution implementations, and result types.

pub mod executable;
pub mod fetch;
pub mod glob;
pub mod grep;
pub mod names;
pub mod patch;
pub mod read;
pub mod shell;
pub mod web_search;
pub mod write;

/// Result of executing a tool.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub output: String,
    pub is_error: bool,
    /// Which tool produced this result (e.g. "read_file", "write_file").
    pub tool_name: Option<String>,
    /// The file path the tool operated on, if applicable.
    pub file_path: Option<String>,
    /// Files this tool modified on disk (used by post-tool hooks).
    pub files_modified: Vec<std::path::PathBuf>,
    /// Lines added by this tool invocation.
    pub lines_added: usize,
    /// Lines removed by this tool invocation.
    pub lines_removed: usize,
}
