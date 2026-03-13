//! Permission modes and tool approval decisions.

use std::collections::HashSet;

/// Permission mode — controls which tools auto-execute vs. require approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    /// Read-only. Writes and commands are blocked entirely.
    Plan,
    /// Reads auto-execute. Writes and commands require approval.
    Default,
    /// Reads and writes auto-execute. Commands require approval (unless in allow-list).
    AutoEdit,
    /// Everything auto-executes. No approval prompts.
    Chug,
}

impl PermissionMode {
    /// Parse from a string (CLI flag or config value).
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "plan" => Self::Plan,
            "default" => Self::Default,
            "auto-edit" | "autoedit" | "auto_edit" => Self::AutoEdit,
            "chug" => Self::Chug,
            _ => Self::Default,
        }
    }
}

/// User-facing mode — controls tool permission guardrails.
/// Cycled via Tab key: Plan → Create → Chug → Plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Read-only exploration. Writes, commands, MCP blocked.
    Plan,
    /// Agentic coding. Reads auto, writes/commands require approval.
    Create,
    /// Everything auto-executes. No approval prompts.
    Chug,
}

impl Mode {
    /// Cycle to the next mode.
    pub fn next(self) -> Self {
        match self {
            Self::Plan => Self::Create,
            Self::Create => Self::Chug,
            Self::Chug => Self::Plan,
        }
    }

    /// Map to the underlying permission mode.
    pub fn to_permission_mode(self) -> PermissionMode {
        match self {
            Self::Plan => PermissionMode::Plan,
            Self::Create => PermissionMode::Default,
            Self::Chug => PermissionMode::Chug,
        }
    }

    /// Derive from a startup PermissionMode.
    pub fn from_permission_mode(pm: &PermissionMode) -> Self {
        match pm {
            PermissionMode::Plan => Self::Plan,
            PermissionMode::Chug => Self::Chug,
            _ => Self::Create, // Default and AutoEdit both map to Create
        }
    }

    /// Display label for the info line.
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::Create => "Create",
            Self::Chug => "Chug",
        }
    }
}

/// Decision for a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDecision {
    AutoExecute,
    RequireApproval,
    Blocked(String),
}

/// Read-only tools that are safe in all modes.
const READ_TOOLS: &[&str] = &[
    "read_file",
    "glob",
    "grep",
    "list_directory",
    "fetch",
    "lsp",
    "diagnostics",
    "web_search",
];

/// Task management tools — mutate UI-only state (no filesystem side effects).
/// Auto-execute in all modes since they only update the in-memory task outline.
const TASK_TOOLS: &[&str] = &["todo_write", "todo_read"];

/// File-write tools.
const WRITE_TOOLS: &[&str] = &["write_file", "edit_file", "apply_patch"];

/// Returns true if `path` is an absolute path that does NOT start with `primary_root`.
/// Relative paths are assumed to be in the primary workspace (return false).
pub fn is_cross_workspace_path(path: &str, primary_root: &std::path::Path) -> bool {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return false;
    }
    !p.starts_with(primary_root)
}

/// Returns true if `path` is allowed: relative, within primary root, or within a registered workspace.
pub fn is_path_allowed(
    path: &str,
    primary_root: &std::path::Path,
    workspace_paths: &[&str],
) -> bool {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return true; // relative paths resolve to primary root
    }
    if p.starts_with(primary_root) {
        return true;
    }
    workspace_paths
        .iter()
        .any(|ws| p.starts_with(std::path::Path::new(ws)))
}

/// Extract the primary path argument from a read tool's input, if any.
fn extract_read_path(_tool_name: &str, tool_input: &serde_json::Value) -> Option<String> {
    // read_file uses "file_path" or "path"
    // glob uses "path" (directory)
    // grep uses "path" (file or directory)
    let path = tool_input["file_path"]
        .as_str()
        .or_else(|| tool_input["path"].as_str());
    path.map(|s| s.to_string())
}

/// Check whether a tool call should auto-execute, require approval, or be blocked.
#[allow(clippy::too_many_arguments)]
pub fn check_permission(
    mode: &PermissionMode,
    tool_name: &str,
    tool_input: &serde_json::Value,
    allow_list: &[String],
    deny_list: &[String],
    session_allows: &HashSet<String>,
    tool_permission_override: Option<&str>,
    primary_root: Option<&std::path::Path>,
    allowed_workspace_paths: &[&str],
) -> ToolDecision {
    // Cross-workspace write check: write/edit targeting a secondary workspace
    // always requires approval, regardless of mode.
    // Note: apply_patch is excluded here — it has no top-level `path` field in
    // tool_input (path is embedded in the diff body). apply_patch cross-workspace
    // detection is deferred to a future enhancement.
    if let Some(root) = primary_root
        && matches!(tool_name, "write_file" | "edit_file")
    {
        let file_path = tool_input["path"]
            .as_str()
            .or_else(|| tool_input["file_path"].as_str())
            .unwrap_or("");
        if is_cross_workspace_path(file_path, root) {
            return ToolDecision::RequireApproval;
        }
    }

    // Session allow-list override (from user pressing 'a' in approval UI)
    if session_allows.contains(tool_name) {
        return ToolDecision::AutoExecute;
    }

    // Read-only tools: block access to paths outside primary root and registered workspaces.
    // This enforces that the agent only reads files the user has explicitly granted access to.
    // fetch and web_search have no file path — skip path check for them.
    if READ_TOOLS.contains(&tool_name) {
        if let Some(root) = primary_root
            && !matches!(tool_name, "fetch" | "web_search" | "lsp" | "diagnostics")
            && let Some(path) = extract_read_path(tool_name, tool_input)
            && !is_path_allowed(&path, root, allowed_workspace_paths)
        {
            return ToolDecision::Blocked(
                "path is outside the project — add it as a workspace to allow access".to_string(),
            );
        }
        return ToolDecision::AutoExecute;
    }

    // Task management tools auto-execute in all modes (UI-only state, no filesystem mutation)
    if TASK_TOOLS.contains(&tool_name) {
        return ToolDecision::AutoExecute;
    }

    // Per-tool permission override (from CLI/exec tool config)
    if let Some(override_str) = tool_permission_override {
        match override_str {
            "deny" => {
                return ToolDecision::Blocked(format!(
                    "Tool '{tool_name}' is configured as denied"
                ));
            }
            "always_approve" => return ToolDecision::RequireApproval,
            _ => {} // "auto" or unknown — fall through to normal logic
        }
    }

    // MCP tools (namespaced with ":") are external — treat as potentially dangerous
    let is_mcp_tool = tool_name.contains(':');
    let is_cli_tool = tool_name.starts_with("cli_");
    let is_exec_tool = tool_name.starts_with("exec_");

    match mode {
        PermissionMode::Chug => ToolDecision::AutoExecute,

        PermissionMode::Plan => {
            // Plan mode: block writes, commands, MCP tools, CLI tools, and exec tools
            if WRITE_TOOLS.contains(&tool_name)
                || tool_name == "run_command"
                || is_mcp_tool
                || is_cli_tool
                || is_exec_tool
            {
                ToolDecision::Blocked(format!(
                    "Tool '{tool_name}' is not allowed in plan mode (read-only)"
                ))
            } else {
                ToolDecision::AutoExecute
            }
        }

        PermissionMode::Default => {
            if WRITE_TOOLS.contains(&tool_name) || is_mcp_tool || is_cli_tool || is_exec_tool {
                ToolDecision::RequireApproval
            } else if tool_name == "run_command" {
                check_command_policy(tool_input, allow_list, deny_list)
            } else if is_cli_tool {
                ToolDecision::RequireApproval
            } else {
                ToolDecision::AutoExecute
            }
        }

        PermissionMode::AutoEdit => {
            if WRITE_TOOLS.contains(&tool_name) {
                ToolDecision::AutoExecute
            } else if tool_name == "run_command" {
                check_command_policy(tool_input, allow_list, deny_list)
            } else if is_mcp_tool || is_cli_tool || is_exec_tool {
                ToolDecision::RequireApproval
            } else {
                ToolDecision::AutoExecute
            }
        }
    }
}

/// Check a run_command tool call against allow/deny policy.
fn check_command_policy(
    tool_input: &serde_json::Value,
    allow_list: &[String],
    deny_list: &[String],
) -> ToolDecision {
    let command = tool_input
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let decision = crate::safety::command_policy::check(command, allow_list, deny_list);

    match decision {
        crate::safety::command_policy::Decision::Allow => ToolDecision::AutoExecute,
        crate::safety::command_policy::Decision::Deny(reason) => ToolDecision::Blocked(reason),
        crate::safety::command_policy::Decision::RequireApproval => ToolDecision::RequireApproval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_allows_reads() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "read_file",
            &serde_json::Value::Null,
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn plan_mode_blocks_writes() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "write_file",
            &serde_json::Value::Null,
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn plan_mode_blocks_commands() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "run_command",
            &serde_json::json!({"command": "ls"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn default_mode_approves_writes() {
        let decision = check_permission(
            &PermissionMode::Default,
            "write_file",
            &serde_json::Value::Null,
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn default_mode_auto_reads() {
        let decision = check_permission(
            &PermissionMode::Default,
            "glob",
            &serde_json::Value::Null,
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn auto_edit_mode_auto_writes() {
        let decision = check_permission(
            &PermissionMode::AutoEdit,
            "edit_file",
            &serde_json::Value::Null,
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn auto_edit_mode_approves_commands() {
        let decision = check_permission(
            &PermissionMode::AutoEdit,
            "run_command",
            &serde_json::json!({"command": "cargo test"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn chug_mode_auto_everything() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "run_command",
            &serde_json::json!({"command": "rm -rf /"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn command_allow_list_overrides() {
        let decision = check_permission(
            &PermissionMode::Default,
            "run_command",
            &serde_json::json!({"command": "cargo test"}),
            &["cargo".to_string()],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn command_deny_list_blocks() {
        let decision = check_permission(
            &PermissionMode::AutoEdit,
            "run_command",
            &serde_json::json!({"command": "rm -rf /"}),
            &[],
            &["rm".to_string()],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn session_allow_list_auto_executes() {
        let mut session_allows = std::collections::HashSet::new();
        session_allows.insert("write_file".to_string());
        let decision = check_permission(
            &PermissionMode::Default,
            "write_file",
            &serde_json::Value::Null,
            &[],
            &[],
            &session_allows,
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn mcp_tools_require_approval_in_default_mode() {
        let decision = check_permission(
            &PermissionMode::Default,
            "github:create_issue",
            &serde_json::json!({"title": "test"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn mcp_tools_auto_in_chug_mode() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "github:create_issue",
            &serde_json::json!({"title": "test"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn mcp_tools_blocked_in_plan_mode() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "github:create_issue",
            &serde_json::json!({"title": "test"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn mcp_tools_require_approval_in_auto_edit_mode() {
        let decision = check_permission(
            &PermissionMode::AutoEdit,
            "github:create_issue",
            &serde_json::json!({"title": "test"}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn mcp_tools_session_allow_overrides() {
        let mut allows = std::collections::HashSet::new();
        allows.insert("github:create_issue".to_string());
        let decision = check_permission(
            &PermissionMode::Default,
            "github:create_issue",
            &serde_json::json!({}),
            &[],
            &[],
            &allows,
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn mode_next_cycles_plan_create_chug() {
        assert_eq!(Mode::Plan.next(), Mode::Create);
        assert_eq!(Mode::Create.next(), Mode::Chug);
        assert_eq!(Mode::Chug.next(), Mode::Plan);
    }

    #[test]
    fn mode_to_permission_mode_mapping() {
        assert_eq!(Mode::Plan.to_permission_mode(), PermissionMode::Plan);
        assert_eq!(Mode::Create.to_permission_mode(), PermissionMode::Default);
        assert_eq!(Mode::Chug.to_permission_mode(), PermissionMode::Chug);
    }

    #[test]
    fn mode_from_permission_mode_mapping() {
        assert_eq!(
            Mode::from_permission_mode(&PermissionMode::Plan),
            Mode::Plan
        );
        assert_eq!(
            Mode::from_permission_mode(&PermissionMode::Default),
            Mode::Create
        );
        assert_eq!(
            Mode::from_permission_mode(&PermissionMode::AutoEdit),
            Mode::Create
        );
        assert_eq!(
            Mode::from_permission_mode(&PermissionMode::Chug),
            Mode::Chug
        );
    }

    #[test]
    fn mode_label_returns_expected_strings() {
        assert_eq!(Mode::Plan.label(), "Plan");
        assert_eq!(Mode::Create.label(), "Create");
        assert_eq!(Mode::Chug.label(), "Chug");
    }

    #[test]
    fn todo_read_auto_executes_in_all_modes() {
        let result = check_permission(
            &PermissionMode::Plan,
            "todo_read",
            &serde_json::json!({}),
            &[],
            &[],
            &HashSet::new(),
            None,
            None,
            &[],
        );
        assert_eq!(result, ToolDecision::AutoExecute);
    }

    #[test]
    fn todo_write_auto_executes_in_plan_mode() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "todo_write",
            &serde_json::json!({"todos": []}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn web_search_auto_executes_in_default_mode() {
        let result = check_permission(
            &PermissionMode::Default,
            "web_search",
            &serde_json::json!({"query": "test"}),
            &[],
            &[],
            &HashSet::new(),
            None,
            None,
            &[],
        );
        assert!(matches!(result, ToolDecision::AutoExecute));
    }

    #[test]
    fn todo_write_auto_executes_in_default_mode() {
        let decision = check_permission(
            &PermissionMode::Default,
            "todo_write",
            &serde_json::json!({"todos": []}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn cli_tool_blocked_in_plan_mode() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "cli_test",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn cli_tool_requires_approval_in_default_mode() {
        let decision = check_permission(
            &PermissionMode::Default,
            "cli_test",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn cli_tool_auto_in_chug_mode() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "cli_test",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn cli_tool_always_approve_overrides_chug() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "cli_deploy",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            Some("always_approve"),
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn cli_tool_deny_override_blocks() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "cli_danger",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            Some("deny"),
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn exec_tool_blocked_in_plan_mode() {
        let decision = check_permission(
            &PermissionMode::Plan,
            "exec_my_tool",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn exec_tool_requires_approval_in_default_mode() {
        let decision = check_permission(
            &PermissionMode::Default,
            "exec_my_tool",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn exec_tool_auto_in_chug_mode() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "exec_my_tool",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::AutoExecute);
    }

    #[test]
    fn exec_tool_requires_approval_in_auto_edit_mode() {
        let decision = check_permission(
            &PermissionMode::AutoEdit,
            "exec_my_tool",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            None,
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn exec_tool_deny_override_blocks() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "exec_my_tool",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            Some("deny"),
            None,
            &[],
        );
        assert!(matches!(decision, ToolDecision::Blocked(_)));
    }

    #[test]
    fn exec_tool_always_approve_overrides_chug() {
        let decision = check_permission(
            &PermissionMode::Chug,
            "exec_my_tool",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            Some("always_approve"),
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }

    #[test]
    fn cli_tool_auto_override_follows_mode() {
        let decision = check_permission(
            &PermissionMode::Default,
            "cli_test",
            &serde_json::json!({}),
            &[],
            &[],
            &Default::default(),
            Some("auto"),
            None,
            &[],
        );
        assert_eq!(decision, ToolDecision::RequireApproval);
    }
}

#[cfg(test)]
mod cross_workspace_tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn path_in_primary_is_not_cross_workspace() {
        let primary = std::path::PathBuf::from("/home/alex/caboose");
        let target = "/home/alex/caboose/src/main.rs";
        assert!(!is_cross_workspace_path(target, &primary));
    }

    #[test]
    #[cfg(windows)]
    fn path_in_primary_is_not_cross_workspace() {
        let primary = std::path::PathBuf::from(r"C:\Users\alex\caboose");
        let target = r"C:\Users\alex\caboose\src\main.rs";
        assert!(!is_cross_workspace_path(target, &primary));
    }

    #[test]
    #[cfg(unix)]
    fn path_in_secondary_is_cross_workspace() {
        let primary = std::path::PathBuf::from("/home/alex/caboose");
        let target = "/home/alex/caboose-web/src/index.ts";
        assert!(is_cross_workspace_path(target, &primary));
    }

    #[test]
    #[cfg(windows)]
    fn path_in_secondary_is_cross_workspace() {
        let primary = std::path::PathBuf::from(r"C:\Users\alex\caboose");
        let target = r"C:\Users\alex\caboose-web\src\index.ts";
        assert!(is_cross_workspace_path(target, &primary));
    }

    #[test]
    fn relative_path_is_not_cross_workspace() {
        let primary = std::path::PathBuf::from("/home/alex/caboose");
        let target = "src/main.rs"; // relative — assumed primary
        assert!(!is_cross_workspace_path(target, &primary));
    }
}
