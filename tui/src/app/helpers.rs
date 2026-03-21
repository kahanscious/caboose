use super::{ChatMessage, State, Task, TaskOutline, TaskStatus};

/// Returns true when Roundhouse is in an active running phase
/// (i.e. keys should be routed to the roundhouse handler, hover-copy disabled).
pub(crate) fn roundhouse_active(state: &State) -> bool {
    state
        .roundhouse_session
        .as_ref()
        .map(|s| {
            !matches!(
                s.phase,
                crate::roundhouse::RoundhousePhase::AwaitingPrompt
                    | crate::roundhouse::RoundhousePhase::SelectingProviders
                    | crate::roundhouse::RoundhousePhase::Cancelled
                    | crate::roundhouse::RoundhousePhase::Complete
            )
        })
        .unwrap_or(false)
}

/// Returns true if the session has real conversation content (User or Assistant messages)
/// that warrants a confirmation before starting a new session.
pub(crate) fn needs_new_session_confirm(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .any(|m| matches!(m, ChatMessage::User { .. } | ChatMessage::Assistant { .. }))
}

/// Background task for a single spawn_agent call. Runs the subagent,
/// merges on success, cleans up worktree. Returns a SpawnAgentResult
/// for the event loop to inject as a ToolResult.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_spawn_agent_task(
    agent_id: uuid::Uuid,
    tool_use_id: String,
    task: String,
    branch: String,
    worktree_path: std::path::PathBuf,
    base_sha: String,
    mut input: crate::sub_agent::executor::SubAgentInput,
    provider: std::sync::Arc<dyn caboose_core::provider::Provider + Send + Sync>,
    config: caboose_core::config::Config,
    tx: tokio::sync::mpsc::UnboundedSender<crate::sub_agent::SubAgentEvent>,
) -> crate::sub_agent::SpawnAgentResult {
    use crate::sub_agent::{SpawnAgentResult, SubAgentState};

    let requires_changes = task_likely_requires_changes(&task);
    let mut total_cost = 0.0;
    let mut attempts = 0usize;

    loop {
        attempts += 1;
        let run_result = crate::sub_agent::executor::run_subagent(
            &mut input,
            provider.clone(),
            config.clone(),
            tx.clone(),
        )
        .await;

        match run_result {
            Ok((cost, summary)) => {
                total_cost += cost;
                if !branch.is_empty() {
                    let commit_result = tokio::task::spawn_blocking({
                        let path = worktree_path.clone();
                        let task_for_commit = task.clone();
                        let message = format!("subagent: {task_for_commit}");
                        move || crate::sub_agent::worktree::commit_worktree(&path, &message)
                    })
                    .await;

                    match commit_result {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            return SpawnAgentResult {
                                agent_id,
                                tool_use_id,
                                result_text: format!(
                                    "spawn_agent: failed to capture worktree changes for '{task}': {e}"
                                ),
                                is_error: true,
                                produced_changes: false,
                                task,
                                final_state: SubAgentState::Failed,
                                cost_usd: total_cost,
                                changes: None,
                            };
                        }
                        Err(e) => {
                            return SpawnAgentResult {
                                agent_id,
                                tool_use_id,
                                result_text: format!(
                                    "spawn_agent: commit task panicked for '{task}': {e}"
                                ),
                                is_error: true,
                                produced_changes: false,
                                task,
                                final_state: SubAgentState::Failed,
                                cost_usd: total_cost,
                                changes: None,
                            };
                        }
                    }
                }

                // Collect changes via git diff for conflict detection
                let diff_output = tokio::task::spawn_blocking({
                    let base = base_sha.clone();
                    let br = branch.clone();
                    move || crate::sub_agent::worktree::run_diff(&base, &br)
                })
                .await;

                let changes = match diff_output {
                    Ok(Ok(output)) => {
                        let mut files = crate::sub_agent::conflict::parse_diff_hunks(&output);
                        let produced_changes = !files.is_empty();
                        if !produced_changes {
                            if requires_changes && attempts == 1 {
                                let _ = tx.send(crate::sub_agent::SubAgentEvent::StreamLine {
                                    id: agent_id,
                                    line: crate::sub_agent::SubAgentStreamLine {
                                        kind: crate::sub_agent::StreamLineKind::Error,
                                        text: "No tracked file changes detected. Retrying once with stricter edit instructions.".to_string(),
                                    },
                                });
                                input.task = build_noop_retry_task(&task);
                                continue;
                            }
                            return SpawnAgentResult {
                                agent_id,
                                tool_use_id,
                                result_text: if requires_changes {
                                    format!(
                                        "spawn_agent: agent produced no tracked file changes for '{task}' after {} attempt(s)\n\nLast summary: {summary}",
                                        attempts
                                    )
                                } else {
                                    format!(
                                        "Agent completed task but produced no tracked file changes: {task}\n\n{summary}"
                                    )
                                },
                                is_error: requires_changes,
                                produced_changes: false,
                                task,
                                final_state: if requires_changes {
                                    SubAgentState::Failed
                                } else {
                                    SubAgentState::Done
                                },
                                cost_usd: total_cost,
                                changes: None,
                            };
                        }
                        for file in &mut files {
                            let base_content = crate::sub_agent::worktree::read_file_at_commit(
                                &base_sha, &file.path,
                            )
                            .ok()
                            .flatten();
                            let new_content = crate::sub_agent::worktree::read_worktree_file(
                                &worktree_path,
                                &file.path,
                            )
                            .ok()
                            .flatten();
                            crate::sub_agent::conflict::enrich_file_change_semantics(
                                file,
                                base_content.as_deref(),
                                new_content.as_deref(),
                            );
                        }
                        Some(crate::sub_agent::conflict::AgentChanges {
                            agent_id,
                            task: task.clone(),
                            files,
                        })
                    }
                    _ => None,
                };

                return SpawnAgentResult {
                    agent_id,
                    tool_use_id,
                    result_text: format!("Agent completed task: {task}\n\n{summary}"),
                    is_error: false,
                    produced_changes: true,
                    task,
                    final_state: SubAgentState::Review,
                    cost_usd: total_cost,
                    changes,
                };
            }
            Err(message) => {
                tracing::error!("spawn_agent executor failed for '{task}': {message}");
                return SpawnAgentResult {
                    agent_id,
                    tool_use_id,
                    result_text: format!("spawn_agent: agent failed for '{task}': {message}"),
                    is_error: true,
                    produced_changes: false,
                    task,
                    final_state: SubAgentState::Failed,
                    cost_usd: total_cost,
                    changes: None,
                };
            }
        }
    }
}

pub(super) fn task_likely_requires_changes(task: &str) -> bool {
    let task = task.to_ascii_lowercase();
    [
        "edit_file",
        "write_file",
        "apply_patch",
        "replace this exact line",
        "do not edit any other line",
        "do not edit any other file",
        "modify the file",
        "edit the file",
    ]
    .iter()
    .any(|needle| task.contains(needle))
}

pub(super) fn build_noop_retry_task(task: &str) -> String {
    format!(
        "{task}\n\nRetry requirement:\n- You must actually modify the target file in this attempt.\n- Use a file-editing tool (`edit_file`, `write_file`, or `apply_patch`) rather than only reading.\n- If the exact match fails, read the smallest needed window and then immediately call the edit tool.\n- Do not stop after inspection. The task is incomplete unless a file diff is produced."
    )
}

/// Parse "5m" → 300, "30s" → 30, "1h" → 3600
pub(super) fn parse_interval(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('s') {
        n.parse().ok()
    } else if let Some(n) = s.strip_suffix('m') {
        n.parse::<u64>().ok().map(|n| n * 60)
    } else if let Some(n) = s.strip_suffix('h') {
        n.parse::<u64>().ok().map(|n| n * 3600)
    } else {
        None
    }
}

/// Format seconds back to human-readable: 300 → "5m", 3600 → "1h", 90 → "1m 30s"
pub(super) fn format_duration(secs: u64) -> String {
    if secs >= 3600 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs >= 60 && secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else if secs >= 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}s", secs)
    }
}

/// Parse circuit command args: "<interval> <prompt>"
pub(super) fn parse_circuit_args(args: &str) -> Option<(u64, String)> {
    let args = args.trim();

    // First token is interval
    let space = args.find(' ')?;
    let interval_str = &args[..space];
    let interval = parse_interval(interval_str)?;

    // Rest is prompt (strip quotes if present)
    let prompt = args[space..].trim();
    let prompt = prompt.trim_matches('"').trim_matches('\'').trim();
    if prompt.is_empty() {
        return None;
    }

    Some((interval, prompt.to_string()))
}

/// Parse task-like patterns from assistant text output.
/// Recognizes markdown checklists and numbered lists with status markers.
pub(super) fn parse_tasks_from_text(text: &str) -> Option<TaskOutline> {
    let mut tasks = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Markdown checklist: - [x] Done, - [ ] Pending
        if let Some(rest) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
        {
            tasks.push(Task {
                content: rest.trim().to_string(),
                active_form: rest.trim().to_string(),
                status: TaskStatus::Completed,
            });
        } else if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
            tasks.push(Task {
                content: rest.trim().to_string(),
                active_form: rest.trim().to_string(),
                status: TaskStatus::Pending,
            });
        }
        // Numbered list: 1. [DONE] Task, 2. [IN PROGRESS] Task, 3. Task
        else if let Some(after_dot) = trimmed.split_once(". ").and_then(|(num, rest)| {
            if num.chars().all(|c| c.is_ascii_digit()) && !num.is_empty() {
                Some(rest)
            } else {
                None
            }
        }) {
            if let Some(rest) = after_dot
                .strip_prefix("[DONE] ")
                .or_else(|| after_dot.strip_prefix("[done] "))
            {
                tasks.push(Task {
                    content: rest.trim().to_string(),
                    active_form: rest.trim().to_string(),
                    status: TaskStatus::Completed,
                });
            } else if let Some(rest) = after_dot
                .strip_prefix("[IN PROGRESS] ")
                .or_else(|| after_dot.strip_prefix("[in progress] "))
            {
                tasks.push(Task {
                    content: rest.trim().to_string(),
                    active_form: rest.trim().to_string(),
                    status: TaskStatus::InProgress,
                });
            } else if let Some(rest) = after_dot
                .strip_prefix("[CANCELLED] ")
                .or_else(|| after_dot.strip_prefix("[cancelled] "))
            {
                tasks.push(Task {
                    content: rest.trim().to_string(),
                    active_form: rest.trim().to_string(),
                    status: TaskStatus::Cancelled,
                });
            } else {
                tasks.push(Task {
                    content: after_dot.trim().to_string(),
                    active_form: after_dot.trim().to_string(),
                    status: TaskStatus::Pending,
                });
            }
        }
    }

    if tasks.len() >= 2 {
        Some(TaskOutline { tasks })
    } else {
        None
    }
}

/// Returns all filesystem roots to search when no explicit path is typed.
/// On Windows: all mounted drive roots (A:\ through Z:\).
/// On Unix: ["/"].
pub(super) fn scan_roots() -> Vec<String> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .filter_map(|c| {
                let root = format!("{}:\\", c as char);
                if std::path::Path::new(&root).exists() {
                    Some(root)
                } else {
                    None
                }
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec!["/".to_string()]
    }
}

/// Spawn a background tokio task to walk directories under the given roots.
/// Results are sent via the returned mpsc receiver.
/// Constraints: max depth 5, max 100 results, 1s timeout.
pub(super) fn spawn_dir_scan(
    roots: Vec<String>,
    query: String,
) -> tokio::sync::mpsc::Receiver<Vec<String>> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            tokio::task::spawn_blocking(move || walk_dirs_fuzzy(&roots, &query)),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();

        let _ = tx.send(result).await;
    });
    rx
}

/// Walk directories under `roots`, returning up to 100 fuzzy-matched **absolute** paths.
pub(super) fn walk_dirs_fuzzy(roots: &[String], query: &str) -> Vec<String> {
    // When the query is a partial path like "a:/projects/cabo", match only on
    // the last component ("cabo") so directory names score correctly.
    let match_term = if query.contains('/') || query.contains('\\') {
        std::path::Path::new(query)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(query)
    } else {
        query
    };
    let query_lower = match_term.to_lowercase();
    let mut candidates: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();

    // BFS so shallow paths surface before deep ones — avoids hitting the 200
    // cap with deeply-nested entries before reaching the user's target.
    let mut queue: std::collections::VecDeque<(std::path::PathBuf, usize)> =
        std::collections::VecDeque::new();
    for root in roots {
        // Strip Windows extended-length path prefix (\\?\) that canonicalize() adds —
        // it bleeds into display strings and causes false duplicates.
        let root = root.strip_prefix(r"\\?\").unwrap_or(root);
        let p = std::path::PathBuf::from(root);
        if p.exists() {
            queue.push_back((p, 0));
        }
    }
    while let Some((dir, depth)) = queue.pop_front() {
        if depth >= 5 || candidates.len() >= 200 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || is_ignored_dir(name) {
                continue;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            if let Some(s) = path.to_str() {
                candidates.push(s.to_string());
            }
            if depth + 1 < 5 {
                queue.push_back((path, depth + 1));
            }
        }
    }

    // Fuzzy score against the last path component (dirname) for relevance
    let mut scored: Vec<(u32, String)> = candidates
        .into_iter()
        .filter_map(|abs_path| {
            let component = std::path::Path::new(&abs_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&abs_path);
            crate::tui::file_auto::score_path_for_dir(component, &query_lower)
                .map(|score| (score, abs_path))
        })
        .collect();

    scored.sort_by_key(|(s, _)| *s);
    scored.truncate(100);
    scored.into_iter().map(|(_, p)| p).collect()
}

/// Directories to skip during workspace scanning.
pub(super) fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | ".git"
            | ".svn"
            | ".hg"
            | "target"
            | "dist"
            | "build"
            | "out"
            | ".next"
            | ".nuxt"
            | ".cache"
            | "cache"
            | "__pycache__"
            | ".tox"
            | "venv"
            | ".venv"
            | "env"
            | ".env"
            | "vendor"
            | ".idea"
            | ".vscode"
            | "Temp"
            | "temp"
            | "tmp"
            | "$Recycle.Bin"
            | "System Volume Information"
            | "Windows"
            | "Program Files"
            | "Program Files (x86)"
    )
}

pub(super) fn build_workspace_list_state(
    config: &caboose_core::config::Config,
) -> crate::tui::dialog::WorkspaceListState {
    use crate::tui::dialog::WorkspaceListState;
    let workspaces = config
        .workspaces
        .iter()
        .map(|(name, cfg)| {
            let available = std::path::Path::new(&cfg.path).exists();
            (name.clone(), cfg.clone(), available)
        })
        .collect::<Vec<_>>();
    WorkspaceListState {
        workspaces,
        selected: 0,
    }
}

/// Build the workspace context block for injection into the system prompt.
/// Omits workspaces whose path no longer exists.
/// Returns an empty string if no workspaces are configured or available.
pub(super) fn workspace_system_prompt_block(
    workspaces: &std::collections::HashMap<String, caboose_core::config::schema::WorkspaceConfig>,
) -> String {
    use caboose_core::config::schema::{WorkspaceAccess, WorkspaceMode};

    if workspaces.is_empty() {
        return String::new();
    }

    let available: Vec<_> = workspaces
        .iter()
        .filter(|(_, cfg)| std::path::Path::new(&cfg.path).exists())
        .collect();

    if available.is_empty() {
        return String::new();
    }

    let proactive: Vec<_> = available
        .iter()
        .filter(|(_, c)| c.mode == WorkspaceMode::Proactive)
        .collect();
    let explicit: Vec<_> = available
        .iter()
        .filter(|(_, c)| c.mode == WorkspaceMode::Explicit)
        .collect();

    let mut block = String::new();
    block.push_str("\n\n<workspaces>\n");
    block.push_str("The following additional repositories are registered. Use your file tools (read, glob, grep) to access them by their absolute paths.\n\n");

    if !proactive.is_empty() {
        block.push_str("Proactive — search these automatically when relevant:\n");
        for (name, cfg) in &proactive {
            let access = if cfg.access == WorkspaceAccess::ReadOnly {
                "read-only"
            } else {
                "read-write"
            };
            block.push_str(&format!("- {name} ({access}): {}\n", cfg.path));
        }
        block.push('\n');
    }

    if !explicit.is_empty() {
        block.push_str("Explicit — only use when the user directly references by name:\n");
        for (name, cfg) in &explicit {
            let access = if cfg.access == WorkspaceAccess::ReadOnly {
                "read-only"
            } else {
                "read-write"
            };
            block.push_str(&format!("- {name} ({access}): {}\n", cfg.path));
        }
        block.push('\n');
    }

    block.push_str("</workspaces>");
    block
}

pub(super) fn has_meaningful_model_switch_context(
    chat_messages: &[ChatMessage],
    has_modified_files: bool,
    has_tool_counts: bool,
) -> bool {
    if has_modified_files || has_tool_counts {
        return true;
    }

    let has_user_message = chat_messages
        .iter()
        .any(|msg| matches!(msg, ChatMessage::User { .. }));
    let has_open_tasks = chat_messages.iter().rev().any(|msg| {
        matches!(
            msg,
            ChatMessage::TaskOutline(outline)
                if outline.tasks.iter().any(|t| !matches!(t.status, TaskStatus::Completed | TaskStatus::Cancelled))
        )
    });

    has_user_message || has_open_tasks
}

#[cfg(test)]
mod task_text_parse_tests {
    use super::*;

    #[test]
    fn parse_markdown_checklist() {
        let text =
            "Here's what I'll do:\n- [x] Read the file\n- [ ] Edit the code\n- [ ] Run tests";
        let outline = parse_tasks_from_text(text).unwrap();
        assert_eq!(outline.tasks.len(), 3);
        assert_eq!(outline.tasks[0].status, TaskStatus::Completed);
        assert_eq!(outline.tasks[1].status, TaskStatus::Pending);
    }

    #[test]
    fn parse_numbered_list_with_status() {
        let text = "Tasks:\n1. [DONE] Setup project\n2. [IN PROGRESS] Write code\n3. Run tests";
        let outline = parse_tasks_from_text(text).unwrap();
        assert_eq!(outline.tasks.len(), 3);
        assert_eq!(outline.tasks[0].status, TaskStatus::Completed);
        assert_eq!(outline.tasks[1].status, TaskStatus::InProgress);
        assert_eq!(outline.tasks[2].status, TaskStatus::Pending);
    }

    #[test]
    fn single_item_returns_none() {
        let text = "- [ ] Only one task";
        assert!(parse_tasks_from_text(text).is_none());
    }

    #[test]
    fn no_tasks_returns_none() {
        let text = "Just some regular text with no task patterns.";
        assert!(parse_tasks_from_text(text).is_none());
    }
}

#[cfg(test)]
mod model_switch_handoff_tests {
    use super::*;

    #[test]
    fn meaningful_model_switch_context_when_user_message_present() {
        let messages = vec![ChatMessage::User {
            content: "debug this".into(),
            images: vec![],
        }];
        assert!(has_meaningful_model_switch_context(&messages, false, false));
    }

    #[test]
    fn meaningful_model_switch_context_when_open_tasks_present() {
        let messages = vec![ChatMessage::TaskOutline(TaskOutline {
            tasks: vec![Task {
                content: "Fix bug".into(),
                active_form: "Fixing bug".into(),
                status: TaskStatus::InProgress,
            }],
        })];
        assert!(has_meaningful_model_switch_context(&messages, false, false));
    }

    #[test]
    fn no_model_switch_context_for_empty_idle_state() {
        assert!(!has_meaningful_model_switch_context(&[], false, false));
    }
}

#[cfg(test)]
mod circuit_parse_tests {
    use super::*;

    #[test]
    fn parse_interval_seconds() {
        assert_eq!(parse_interval("30s"), Some(30));
    }

    #[test]
    fn parse_interval_minutes() {
        assert_eq!(parse_interval("5m"), Some(300));
    }

    #[test]
    fn parse_interval_hours() {
        assert_eq!(parse_interval("1h"), Some(3600));
    }

    #[test]
    fn parse_interval_invalid() {
        assert_eq!(parse_interval("abc"), None);
        assert_eq!(parse_interval(""), None);
    }

    #[test]
    fn parse_circuit_args_basic() {
        let (interval, prompt) = parse_circuit_args("5m \"check build\"").unwrap();
        assert_eq!(interval, 300);
        assert_eq!(prompt, "check build");
    }

    #[test]
    fn parse_circuit_args_no_quotes() {
        let (_, prompt) = parse_circuit_args("5m check build status").unwrap();
        assert_eq!(prompt, "check build status");
    }

    #[test]
    fn parse_circuit_args_missing_prompt() {
        assert!(parse_circuit_args("5m").is_none());
        assert!(parse_circuit_args("5m \"\"").is_none());
    }

    #[test]
    fn parse_circuit_args_bad_interval() {
        assert!(parse_circuit_args("abc \"prompt\"").is_none());
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(300), "5m");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3600), "1h");
    }

    #[test]
    fn format_duration_mixed() {
        assert_eq!(format_duration(90), "1m 30s");
    }
}

#[cfg(test)]
mod workspace_add_validation_tests {
    /// Test the path validation helper logic (extracted for testability).
    /// These test the same conditions checked in handle_workspace_add_confirm_path.

    #[test]
    fn name_from_dirname() {
        let path = std::path::Path::new("/home/alex/caboose-web");
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        assert_eq!(name, "caboose-web");
    }

    #[test]
    fn nested_path_detection() {
        let primary = std::path::PathBuf::from("/home/alex/caboose");
        let child = std::path::PathBuf::from("/home/alex/caboose/sub");
        let parent = std::path::PathBuf::from("/home/alex");
        // child starts_with primary → nested
        assert!(child.starts_with(&primary));
        // primary starts_with parent → primary is nested inside parent
        assert!(primary.starts_with(&parent));
        // sibling does not start_with primary
        let sibling = std::path::PathBuf::from("/home/alex/caboose-web");
        assert!(!sibling.starts_with(&primary));
        assert!(!primary.starts_with(&sibling));
    }

    #[test]
    fn name_validation_no_spaces() {
        let bad = "my workspace";
        let good = "my-workspace";
        assert!(bad.contains(' '));
        assert!(!good.contains(' '));
    }
}

#[cfg(test)]
mod workspace_list_handler_tests {
    use crate::tui::dialog::WorkspaceListState;
    use caboose_core::config::schema::{WorkspaceConfig, WorkspaceMode};

    fn make_state(n: usize) -> WorkspaceListState {
        WorkspaceListState {
            workspaces: (0..n)
                .map(|i| {
                    (
                        format!("ws-{i}"),
                        WorkspaceConfig {
                            path: format!("/tmp/ws{i}"),
                            mode: WorkspaceMode::Proactive,
                            access: caboose_core::config::schema::WorkspaceAccess::ReadWrite,
                        },
                        true,
                    )
                })
                .collect(),
            selected: 0,
        }
    }

    #[test]
    fn remove_last_item_clamps_selected() {
        let mut state = make_state(1);
        state.selected = 0;
        state.workspaces.remove(0);
        state.clamp_selected();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn remove_item_mid_list_clamps_selected() {
        let mut state = make_state(3);
        state.selected = 2; // last item
        state.workspaces.remove(2);
        state.clamp_selected();
        assert_eq!(state.selected, 1);
    }
}

#[cfg(test)]
mod execute_command_tests {
    #[test]
    fn extract_tasks_from_assistant_message() {
        let text = "Here's what I'll do:\n- auth refactor\n- add session tests\n- update readme";
        let tasks = crate::sub_agent::pipeline::extract_tasks(text).unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0], "auth refactor");
    }

    #[test]
    fn no_task_list_returns_none() {
        let text = "Just explaining some code...";
        assert!(crate::sub_agent::pipeline::extract_tasks(text).is_none());
    }
}

#[cfg(test)]
mod workspace_prompt_tests {
    use caboose_core::config::schema::{WorkspaceConfig, WorkspaceMode};
    use std::collections::HashMap;

    fn build_workspace_block(workspaces: &HashMap<String, WorkspaceConfig>) -> String {
        super::workspace_system_prompt_block(workspaces)
    }

    #[test]
    fn empty_workspaces_returns_empty_string() {
        let ws: HashMap<String, WorkspaceConfig> = HashMap::new();
        assert_eq!(build_workspace_block(&ws), "");
    }

    #[test]
    fn proactive_workspace_in_prompt() {
        let path = std::env::temp_dir();
        let path_str = path.to_string_lossy().into_owned();
        let mut ws = HashMap::new();
        ws.insert(
            "caboose-web".to_string(),
            WorkspaceConfig {
                path: path_str.clone(),
                mode: WorkspaceMode::Proactive,
                access: caboose_core::config::schema::WorkspaceAccess::ReadWrite,
            },
        );
        let block = build_workspace_block(&ws);
        assert!(block.contains("caboose-web"));
        assert!(block.contains(&path_str));
        assert!(block.contains("Proactive"));
    }

    #[test]
    fn explicit_workspace_in_prompt() {
        let path = std::env::temp_dir();
        let path_str = path.to_string_lossy().into_owned();
        let mut ws = HashMap::new();
        ws.insert(
            "docs".to_string(),
            WorkspaceConfig {
                path: path_str.clone(),
                mode: WorkspaceMode::Explicit,
                access: caboose_core::config::schema::WorkspaceAccess::ReadWrite,
            },
        );
        let block = build_workspace_block(&ws);
        assert!(block.contains("docs"));
        assert!(block.contains("Explicit"));
    }

    #[test]
    fn unavailable_workspace_omitted() {
        let mut ws = HashMap::new();
        ws.insert(
            "gone".to_string(),
            WorkspaceConfig {
                path: "/nonexistent/path/xyz123".to_string(),
                mode: WorkspaceMode::Proactive,
                access: caboose_core::config::schema::WorkspaceAccess::ReadWrite,
            },
        );
        let block = build_workspace_block(&ws);
        // Path doesn't exist — should be omitted
        assert!(block.is_empty());
    }
}

#[cfg(test)]
mod new_session_confirm_tests {
    use super::*;

    fn user_msg() -> ChatMessage {
        ChatMessage::User {
            content: "hello".into(),
            images: vec![],
        }
    }

    fn assistant_msg() -> ChatMessage {
        ChatMessage::Assistant {
            content: "hi".into(),
            thinking: None,
        }
    }

    fn system_msg() -> ChatMessage {
        ChatMessage::System {
            content: "connected".into(),
        }
    }

    #[test]
    fn empty_messages_no_confirm() {
        assert!(!needs_new_session_confirm(&[]));
    }

    #[test]
    fn only_system_messages_no_confirm() {
        assert!(!needs_new_session_confirm(&[system_msg(), system_msg()]));
    }

    #[test]
    fn user_message_requires_confirm() {
        assert!(needs_new_session_confirm(&[system_msg(), user_msg()]));
    }

    #[test]
    fn assistant_message_requires_confirm() {
        assert!(needs_new_session_confirm(&[assistant_msg()]));
    }
}
