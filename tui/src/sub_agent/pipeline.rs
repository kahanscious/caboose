use std::sync::Arc;

use futures::StreamExt;

use crate::provider::{Message, Provider, StreamEvent};
use crate::sub_agent::{TaskPipeline, TaskStage};

/// Extract the last top-level markdown list (≥2 items) from `text`.
/// Handles both `- item` and `1. item` styles. Returns None if not found.
#[allow(dead_code)]
pub fn extract_tasks(text: &str) -> Option<Vec<String>> {
    let mut current: Vec<String> = Vec::new();
    let mut last_valid: Option<Vec<String>> = None;

    for line in text.lines() {
        if let Some(item) = strip_list_marker(line) {
            current.push(item.trim().to_string());
        } else {
            // Indented lines (nested bullets, continuation) are silently skipped
            // so they don't interrupt the current top-level list.
            if line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            let t = line.trim();
            if !t.is_empty() && !current.is_empty() {
                if current.len() >= 2 {
                    last_valid = Some(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() >= 2 {
        last_valid = Some(current);
    }
    last_valid
}

/// Returns the list item text if `line` is a top-level list marker (not indented).
/// Indented lines (nested bullets) return None.
fn strip_list_marker(line: &str) -> Option<&str> {
    // Reject indented lines — they are nested bullets, not top-level tasks
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    let t = line.trim();
    // Unordered
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    // Ordered: "1. " or "1) "
    let digits = t.trim_start_matches(|c: char| c.is_ascii_digit());
    if let Some(rest) = digits.strip_prefix(". ") {
        return Some(rest);
    }
    if let Some(rest) = digits.strip_prefix(") ") {
        return Some(rest);
    }
    None
}

/// Try to parse a TaskPipeline from an LLM response.
/// Tries ```json ... ``` block first, then bare JSON object.
#[allow(dead_code)]
pub fn parse_pipeline_response(response: &str) -> Option<TaskPipeline> {
    // Try fenced JSON
    if let Some(start) = response.find("```json") {
        let rest = &response[start + 7..];
        if let Some(end) = rest.find("```")
            && let Ok(p) = serde_json::from_str::<TaskPipeline>(rest[..end].trim())
        {
            return Some(p);
        }
    }
    // Try bare JSON object
    if let Some(start) = response.find('{')
        && let Some(end) = response.rfind('}')
        && let Ok(p) = serde_json::from_str::<TaskPipeline>(&response[start..=end])
    {
        return Some(p);
    }
    None
}

/// Fallback: put all tasks in a single sequential stage.
#[allow(dead_code)]
pub fn single_stage_fallback(tasks: Vec<String>) -> TaskPipeline {
    TaskPipeline { stages: vec![TaskStage { tasks }] }
}

/// Build the coordinator prompt for dependency analysis.
#[allow(dead_code)]
pub fn coordinator_prompt(tasks: &[String]) -> String {
    let list = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. {t}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "You are a task dependency analyzer for a software project.\n\
         Given a list of tasks, determine which can run in parallel and which must be sequential.\n\n\
         Return ONLY a JSON object:\n\
         ```json\n\
         {{\"stages\":[{{\"tasks\":[\"task A\",\"task B\"]}},{{\"tasks\":[\"task C\"]}}]}}\n\
         ```\n\n\
         Tasks within a stage run concurrently. Stages run in order.\n\n\
         Rules:\n\
         - Tasks likely touching the same files → separate stages\n\
         - Tasks with explicit ordering signals → later stage\n\
         - Test/build tasks → after their paired implementation task\n\
         - When uncertain → separate stages (conservative)\n\n\
         Tasks:\n{list}"
    )
}

/// Call the LLM coordinator to produce a TaskPipeline from a list of tasks.
/// Uses `provider.stream()`, collects all TextDelta events, then parses the result.
/// Falls back to `single_stage_fallback` if the call fails or output is unparseable.
#[allow(dead_code)]
pub async fn analyze_dependencies(
    tasks: &[String],
    provider: Arc<dyn Provider + Send + Sync>,
) -> TaskPipeline {
    let prompt = coordinator_prompt(tasks);

    let messages = vec![Message {
        role: "user".to_string(),
        content: serde_json::json!(prompt),
    }];

    let mut stream = provider.stream(&messages, &[]);
    let mut response = String::new();

    loop {
        match stream.next().await {
            Some(Ok(StreamEvent::TextDelta(text))) => {
                response.push_str(&text);
            }
            Some(Ok(StreamEvent::Done { .. })) | None => break,
            Some(Ok(StreamEvent::Error(e))) => {
                tracing::warn!("coordinator LLM error: {e}");
                return single_stage_fallback(tasks.to_vec());
            }
            Some(Ok(StreamEvent::ProviderError { message, .. })) => {
                tracing::warn!("coordinator provider error: {message}");
                return single_stage_fallback(tasks.to_vec());
            }
            Some(Ok(_)) => {} // ThinkingDelta, ToolCall etc — ignore for coordinator
            Some(Err(e)) => {
                tracing::warn!("coordinator stream error: {e}");
                return single_stage_fallback(tasks.to_vec());
            }
        }
    }

    parse_pipeline_response(&response)
        .unwrap_or_else(|| single_stage_fallback(tasks.to_vec()))
}

/// Returned by start_pipeline so State can pre-populate sub_agents.
#[allow(dead_code)]
pub struct PipelineSetup {
    pub agents: Vec<crate::sub_agent::SubAgent>,
    pub rx: tokio::sync::mpsc::UnboundedReceiver<crate::sub_agent::SubAgentEvent>,
}

/// Set up and spawn the pipeline driver for a list of tasks.
/// Returns PipelineSetup with the initial agent list and the event receiver.
#[allow(dead_code)]
pub async fn start_pipeline(
    tasks: Vec<String>,
    system_prompt: String,
    provider: Arc<dyn crate::provider::Provider + Send + Sync>,
    config: crate::config::Config,
) -> Result<PipelineSetup, String> {
    use crate::sub_agent::worktree;
    use crate::sub_agent::SubAgent;

    // Pre-check gitignore
    worktree::check_worktrees_ignored().map_err(|e| e.to_string())?;

    let base_branch = worktree::current_branch().map_err(|e| e.to_string())?;

    // Pre-allocate SubAgent instances — all Pending
    let agents: Vec<SubAgent> = tasks
        .iter()
        .map(|t| {
            let slug = worktree::slug(t);
            let branch = worktree::branch_name(&slug);
            let path = worktree::worktree_path(&slug);
            SubAgent::new(t.clone(), branch, path)
        })
        .collect();

    let agent_ids: Vec<(String, uuid::Uuid)> = agents
        .iter()
        .map(|a| (a.task.clone(), a.id))
        .collect();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    let tx2 = tx.clone();
    tokio::spawn(async move {
        drive_pipeline(tasks, agent_ids, base_branch, system_prompt, provider, config, tx2).await;
    });

    Ok(PipelineSetup { agents, rx })
}

async fn drive_pipeline(
    tasks: Vec<String>,
    agent_ids: Vec<(String, uuid::Uuid)>,
    _base_branch: String,
    system_prompt: String,
    provider: Arc<dyn crate::provider::Provider + Send + Sync>,
    config: crate::config::Config,
    tx: tokio::sync::mpsc::UnboundedSender<crate::sub_agent::SubAgentEvent>,
) {
    use crate::sub_agent::{SubAgentEvent, SubAgentState};
    use crate::sub_agent::executor::{SubAgentInput, run_subagent};
    use crate::sub_agent::worktree;

    // Step 1: coordinator analysis
    let pipeline = analyze_dependencies(&tasks, provider.clone()).await;

    // Helper: find id for a task name
    let id_for = |task: &str| -> uuid::Uuid {
        agent_ids
            .iter()
            .find(|(t, _)| t == task)
            .map(|(_, id)| *id)
            .unwrap_or_else(uuid::Uuid::new_v4)
    };

    let mut used_slugs: Vec<String> = Vec::new();

    for stage in &pipeline.stages {
        // Create worktrees for this stage
        let mut stage_work: Vec<(uuid::Uuid, String, std::path::PathBuf, String)> = Vec::new();

        for task in &stage.tasks {
            let slug = worktree::unique_slug(task, &used_slugs);
            used_slugs.push(slug.clone());
            let branch = worktree::branch_name(&slug);
            let path = worktree::worktree_path(&slug);
            let id = id_for(task);

            // Wrap blocking git call in spawn_blocking
            let (path_c, branch_c) = (path.clone(), branch.clone());
            let create_result = tokio::task::spawn_blocking(move || {
                worktree::create_worktree(&path_c, &branch_c)
            })
            .await
            .unwrap_or_else(|e| Err(worktree::WorktreeError::GitFailed(e.to_string())));

            if let Err(e) = create_result {
                let _ = tx.send(SubAgentEvent::StateChange {
                    id,
                    state: SubAgentState::Failed { message: e.to_string() },
                });
                let _ = tx.send(SubAgentEvent::PipelineHalted {
                    message: format!("failed to create worktree for \"{task}\": {e}"),
                });
                return;
            }
            stage_work.push((id, task.clone(), path, branch));
        }

        // Dispatch executors, capturing start time per task
        let mut handles = Vec::new();
        for (id, task, path, _branch) in &stage_work {
            let _ = tx.send(SubAgentEvent::StateChange {
                id: *id,
                state: SubAgentState::Running,
            });
            let started_at = std::time::Instant::now();
            let input = SubAgentInput {
                id: *id,
                task: task.clone(),
                worktree_path: path.clone(),
                system_prompt: system_prompt.clone(),
            };
            let handle = tokio::spawn(run_subagent(
                input,
                provider.clone(),
                config.clone(),
                tx.clone(),
            ));
            handles.push((*id, task.clone(), path.clone(), started_at, handle));
        }

        // Wait for all executors in this stage
        let mut stage_ok = true;
        let mut completed: Vec<(uuid::Uuid, f64, u64)> = Vec::new();

        for (id, task, _path, started_at, handle) in handles {
            match handle.await {
                Ok(Ok(cost)) => {
                    let elapsed = started_at.elapsed().as_secs();
                    completed.push((id, cost, elapsed));
                    let _ = tx.send(SubAgentEvent::StateChange {
                        id,
                        state: SubAgentState::Done,
                    });
                }
                Ok(Err(msg)) => {
                    let _ = tx.send(SubAgentEvent::AgentFailed {
                        id,
                        task: task.clone(),
                        message: msg.clone(),
                    });
                    let _ = tx.send(SubAgentEvent::PipelineHalted {
                        message: format!(
                            "pipeline halted — agent \"{task}\" failed. resolve before continuing."
                        ),
                    });
                    stage_ok = false;
                    break;
                }
                Err(e) => {
                    let _ = tx.send(SubAgentEvent::AgentFailed {
                        id,
                        task: task.clone(),
                        message: e.to_string(),
                    });
                    let _ = tx.send(SubAgentEvent::PipelineHalted {
                        message: format!(
                            "pipeline halted — agent \"{task}\" panicked: {e}"
                        ),
                    });
                    stage_ok = false;
                    break;
                }
            }
        }

        if !stage_ok {
            return;
        }

        // Merge completed tasks
        for (id, task, path, branch) in &stage_work {
            let branch_clone = branch.clone();
            let merge_result = tokio::task::spawn_blocking(move || {
                worktree::merge_branch(&branch_clone)
            })
            .await
            .unwrap_or_else(|e| Err(worktree::WorktreeError::GitFailed(e.to_string())));

            match merge_result {
                Ok(()) => {
                    let (cost, elapsed) = completed
                        .iter()
                        .find(|(i, _, _)| i == id)
                        .map(|(_, c, e)| (*c, *e))
                        .unwrap_or((0.0, 0));
                    let _ = tx.send(SubAgentEvent::AgentMerged {
                        id: *id,
                        task: task.clone(),
                        elapsed_secs: elapsed,
                        cost_usd: cost,
                    });
                    let _ = worktree::remove_worktree(path, branch);
                }
                Err(e) => {
                    let _ = tx.send(SubAgentEvent::StateChange {
                        id: *id,
                        state: SubAgentState::Conflict { report: e.to_string() },
                    });
                    let _ = tx.send(SubAgentEvent::AgentConflict {
                        id: *id,
                        task: task.clone(),
                        worktree_path: path.clone(),
                    });
                    let _ = tx.send(SubAgentEvent::PipelineHalted {
                        message: format!(
                            "pipeline halted — conflict in \"{task}\". worktree preserved at {}",
                            path.display()
                        ),
                    });
                    return;
                }
            }
        }
    }

    let _ = tx.send(SubAgentEvent::PipelineDone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sub_agent::TaskPipeline;

    #[test]
    fn extract_unordered_list() {
        let text = "I'll do these:\n- auth refactor\n- add tests\n- update readme";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks, vec!["auth refactor", "add tests", "update readme"]);
    }

    #[test]
    fn extract_ordered_list() {
        let text = "Plan:\n1. implement feature\n2. write tests\n3. update docs";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks[0], "implement feature");
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn extract_returns_last_list_when_multiple() {
        let text = "First:\n- a\n- b\n\nSecond:\n- c\n- d\n- e";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks, vec!["c", "d", "e"]);
    }

    #[test]
    fn extract_requires_min_two_items() {
        assert!(extract_tasks("- just one thing").is_none());
    }

    #[test]
    fn extract_none_for_prose() {
        assert!(extract_tasks("just prose, no list").is_none());
    }

    #[test]
    fn extract_ignores_nested_bullets() {
        // Nested bullets are NOT separate tasks — only top-level items count
        let text = "Tasks:\n- auth refactor\n  - sub-step A\n  - sub-step B\n- update readme";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks.len(), 2, "nested bullets must not be treated as top-level tasks");
        assert_eq!(tasks[0], "auth refactor");
        assert_eq!(tasks[1], "update readme");
    }

    #[test]
    fn parse_pipeline_from_json() {
        let json = r#"{"stages":[{"tasks":["a","b"]},{"tasks":["c"]}]}"#;
        let p: TaskPipeline = serde_json::from_str(json).unwrap();
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[0].tasks, vec!["a", "b"]);
    }

    #[test]
    fn extract_pipeline_from_code_fence() {
        let response = "analysis:\n```json\n{\"stages\":[{\"tasks\":[\"task a\"]}]}\n```";
        let p = parse_pipeline_response(response).unwrap();
        assert_eq!(p.stages[0].tasks[0], "task a");
    }

    #[test]
    fn parse_pipeline_falls_back_to_none_on_bad_json() {
        assert!(parse_pipeline_response("no json here").is_none());
    }

    #[test]
    fn single_stage_fallback_wraps_all_tasks() {
        let tasks = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let p = single_stage_fallback(tasks);
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].tasks.len(), 3);
    }

    #[test]
    fn coordinator_prompt_contains_tasks() {
        let tasks = vec!["auth refactor".to_string(), "update readme".to_string()];
        let prompt = coordinator_prompt(&tasks);
        assert!(prompt.contains("auth refactor"));
        assert!(prompt.contains("update readme"));
        assert!(prompt.contains("stages"));
    }
}
