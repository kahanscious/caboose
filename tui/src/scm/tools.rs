use crate::provider::ToolDefinition;
use serde_json::json;

/// Generate tool definitions for GitHub CLI tools
pub fn github_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "create_pr".into(),
            description: "Create a GitHub pull request using the gh CLI".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "PR title" },
                    "body": { "type": "string", "description": "PR description" },
                    "base": { "type": "string", "description": "Base branch (default: main)" },
                    "draft": { "type": "boolean", "description": "Create as draft PR" }
                },
                "required": ["title"]
            }),
        },
        ToolDefinition {
            name: "list_prs".into(),
            description: "List GitHub pull requests using the gh CLI".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "state": { "type": "string", "enum": ["open", "closed", "merged", "all"], "description": "Filter by state" },
                    "limit": { "type": "integer", "description": "Max results (default 10)" }
                }
            }),
        },
        ToolDefinition {
            name: "list_issues".into(),
            description: "List GitHub issues using the gh CLI".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "state": { "type": "string", "enum": ["open", "closed", "all"] },
                    "label": { "type": "string", "description": "Filter by label" },
                    "limit": { "type": "integer", "description": "Max results (default 10)" }
                }
            }),
        },
        ToolDefinition {
            name: "check_ci".into(),
            description: "Check CI/GitHub Actions status for current branch or a PR".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pr": { "type": "integer", "description": "PR number (default: current branch)" }
                }
            }),
        },
        ToolDefinition {
            name: "review_pr".into(),
            description: "View PR details, diff, and review comments".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pr": { "type": "integer", "description": "PR number" }
                },
                "required": ["pr"]
            }),
        },
        ToolDefinition {
            name: "merge_pr".into(),
            description: "Merge a GitHub pull request".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pr": { "type": "integer", "description": "PR number" },
                    "method": { "type": "string", "enum": ["merge", "squash", "rebase"], "description": "Merge method (default: merge)" }
                },
                "required": ["pr"]
            }),
        },
    ]
}

/// Execute an SCM tool by shelling out to gh/glab
pub fn execute_scm_tool(
    name: &str,
    args: &serde_json::Value,
    cwd: &std::path::Path,
) -> Result<String, String> {
    match name {
        "create_pr" => {
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let base = args.get("base").and_then(|v| v.as_str()).unwrap_or("main");
            let draft = args.get("draft").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut cmd = std::process::Command::new("gh");
            cmd.args(["pr", "create", "--title", title, "--body", body, "--base", base]);
            if draft {
                cmd.arg("--draft");
            }
            cmd.arg("--json").arg("number,url,title");
            cmd.current_dir(cwd);
            run_command(cmd)
        }
        "list_prs" => {
            let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("open");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

            let mut cmd = std::process::Command::new("gh");
            cmd.args([
                "pr", "list",
                "--state", state,
                "--limit", &limit.to_string(),
                "--json", "number,title,state,author,updatedAt",
            ]);
            cmd.current_dir(cwd);
            run_command(cmd)
        }
        "list_issues" => {
            let state = args.get("state").and_then(|v| v.as_str()).unwrap_or("open");
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

            let mut cmd = std::process::Command::new("gh");
            cmd.args([
                "issue", "list",
                "--state", state,
                "--limit", &limit.to_string(),
                "--json", "number,title,state,labels,assignees",
            ]);
            if let Some(label) = args.get("label").and_then(|v| v.as_str()) {
                cmd.args(["--label", label]);
            }
            cmd.current_dir(cwd);
            run_command(cmd)
        }
        "check_ci" => {
            let mut cmd = std::process::Command::new("gh");
            if let Some(pr) = args.get("pr").and_then(|v| v.as_u64()) {
                cmd.args(["pr", "checks", &pr.to_string(), "--json", "name,state,conclusion"]);
            } else {
                cmd.args(["run", "list", "--limit", "5", "--json", "name,status,conclusion,headBranch"]);
            }
            cmd.current_dir(cwd);
            run_command(cmd)
        }
        "review_pr" => {
            let pr = args.get("pr").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut cmd = std::process::Command::new("gh");
            cmd.args([
                "pr", "view", &pr.to_string(),
                "--json", "number,title,body,state,reviews,comments,additions,deletions,files",
            ]);
            cmd.current_dir(cwd);
            run_command(cmd)
        }
        "merge_pr" => {
            let pr = args.get("pr").and_then(|v| v.as_u64()).unwrap_or(0);
            let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("merge");
            let merge_flag = match method {
                "squash" => "--squash",
                "rebase" => "--rebase",
                _ => "--merge",
            };
            let mut cmd = std::process::Command::new("gh");
            cmd.args(["pr", "merge", &pr.to_string(), merge_flag, "--json", "number,url"]);
            cmd.current_dir(cwd);
            run_command(cmd)
        }
        _ => Err(format!("unknown SCM tool: {name}")),
    }
}

fn run_command(mut cmd: std::process::Command) -> Result<String, String> {
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("command failed: {stderr}"))
            }
        }
        Err(e) => Err(format!("failed to execute: {e}. Is `gh` installed? Run `gh auth login` to authenticate.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_tool_definitions_count() {
        let tools = github_tool_definitions();
        assert_eq!(tools.len(), 6);
    }

    #[test]
    fn test_tool_names() {
        let tools = github_tool_definitions();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"create_pr"));
        assert!(names.contains(&"list_prs"));
        assert!(names.contains(&"list_issues"));
        assert!(names.contains(&"check_ci"));
        assert!(names.contains(&"review_pr"));
        assert!(names.contains(&"merge_pr"));
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let result = execute_scm_tool(
            "nonexistent",
            &serde_json::json!({}),
            std::path::Path::new("."),
        );
        assert!(result.is_err());
    }
}
