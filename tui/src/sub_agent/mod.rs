pub mod executor;
pub mod pipeline;
pub mod worktree;

use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubAgentStreamLine {
    pub kind: StreamLineKind,
    pub text: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum StreamLineKind {
    Thinking,
    ToolCall,
    ToolResult,
    Text,
    Error,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SubAgentState {
    Pending,
    Running,
    WaitingApproval { tool_name: String },
    Review,
    Done,
    Failed { message: String },
    Conflict { report: String },
}

impl SubAgentState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed { .. } | Self::Conflict { .. })
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubAgent {
    pub id: Uuid,
    pub task: String,
    pub branch: String,
    pub worktree_path: PathBuf,
    pub base_sha: String,
    pub state: SubAgentState,
    pub started_at: Option<Instant>,
    pub cost_usd: f64,
    pub stream: Vec<SubAgentStreamLine>,
    /// Present while subagent is running. Used to route approval responses back.
    /// Set to None when the agent reaches a terminal state.
    pub approval_tx: Option<tokio::sync::mpsc::UnboundedSender<bool>>,
    /// When true, automatically approve all tool calls for this agent.
    pub auto_approve: bool,
}

#[allow(dead_code)]
impl SubAgent {
    pub fn new(task: String, branch: String, worktree_path: PathBuf, base_sha: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            task,
            branch,
            worktree_path,
            base_sha,
            state: SubAgentState::Pending,
            started_at: None,
            cost_usd: 0.0,
            stream: Vec::new(),
            approval_tx: None,
            auto_approve: false,
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// Format elapsed seconds. Under 1h: "XmYYs". At or over 1h: "XhYYm".
#[allow(dead_code)]
pub fn format_elapsed(secs: u64) -> String {
    if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m{s:02}s")
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h{m:02}m")
    }
}

/// Events sent from subagent executor tasks to the main thread.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SubAgentEvent {
    StreamLine {
        id: Uuid,
        line: SubAgentStreamLine,
    },
    StateChange {
        id: Uuid,
        state: SubAgentState,
    },
    CostUpdate {
        id: Uuid,
        cost_usd: f64,
    },
    AgentMerged {
        id: Uuid,
        task: String,
        elapsed_secs: u64,
        cost_usd: f64,
    },
    AgentFailed {
        id: Uuid,
        task: String,
        message: String,
    },
    AgentConflict {
        id: Uuid,
        task: String,
        worktree_path: PathBuf,
    },
    /// Subagent needs user approval to proceed.
    /// `id` is the SubAgent's UUID — use it to find the correct `approval_tx`
    /// in `State::sub_agents`.
    ApprovalRequest {
        id: Uuid,
        tool_name: String,
        arguments: String,
    },
}

/// Result from a completed spawn_agent background task.
/// Returned via JoinHandle to the main event loop.
#[allow(dead_code)]
pub struct SpawnAgentResult {
    pub agent_id: Uuid,
    pub tool_use_id: String,
    pub task: String,
    pub result_text: String,
    pub is_error: bool,
    pub final_state: SubAgentState,
    pub cost_usd: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TaskPipeline {
    pub stages: Vec<TaskStage>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TaskStage {
    pub tasks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_agent_starts_pending() {
        let agent = SubAgent::new(
            "auth refactor".to_string(),
            "agent/auth-refactor".to_string(),
            std::path::PathBuf::from(".worktrees/agent-auth-refactor"),
            "abc123".to_string(),
        );
        assert!(matches!(agent.state, SubAgentState::Pending));
        assert_eq!(agent.task, "auth refactor");
        assert_eq!(agent.cost_usd, 0.0);
        assert!(agent.stream.is_empty());
        assert!(agent.started_at.is_none());
    }

    #[test]
    fn elapsed_secs_zero_when_not_started() {
        let agent = SubAgent::new("t".into(), "b".into(), std::path::PathBuf::new(), String::new());
        assert_eq!(agent.elapsed_secs(), 0);
    }

    #[test]
    fn format_elapsed_under_one_hour() {
        assert_eq!(format_elapsed(0), "0m00s");
        assert_eq!(format_elapsed(59), "0m59s");
        assert_eq!(format_elapsed(74), "1m14s");
        assert_eq!(format_elapsed(3599), "59m59s");
    }

    #[test]
    fn format_elapsed_at_or_over_one_hour() {
        assert_eq!(format_elapsed(3600), "1h00m");
        assert_eq!(format_elapsed(3661), "1h01m");
        assert_eq!(format_elapsed(7200), "2h00m");
    }

    #[test]
    fn waiting_approval_variant() {
        let state = SubAgentState::WaitingApproval {
            tool_name: "write_file".to_string(),
        };
        assert!(matches!(state, SubAgentState::WaitingApproval { .. }));
    }

    #[test]
    fn approval_request_event_fields() {
        let id = Uuid::new_v4();
        let ev = SubAgentEvent::ApprovalRequest {
            id,
            tool_name: "write_file".to_string(),
            arguments: "{}".to_string(),
        };
        assert!(matches!(ev, SubAgentEvent::ApprovalRequest { .. }));
    }

    #[test]
    fn sub_agent_approval_tx_initially_none() {
        let agent = SubAgent::new(
            "task".to_string(),
            "agent/task".to_string(),
            std::path::PathBuf::from(".worktrees/agent-task"),
            String::new(),
        );
        assert!(agent.approval_tx.is_none());
    }

    #[test]
    fn sub_agent_auto_approve_defaults_false() {
        let agent = SubAgent::new("task".into(), "branch".into(), std::path::PathBuf::new(), String::new());
        assert!(!agent.auto_approve);
    }

    #[test]
    fn sub_agent_auto_approve_toggleable() {
        let mut agent = SubAgent::new("task".into(), "branch".into(), std::path::PathBuf::new(), String::new());
        assert!(!agent.auto_approve);
        agent.auto_approve = true;
        assert!(agent.auto_approve);
    }

    #[test]
    fn format_elapsed_one_second() {
        assert_eq!(format_elapsed(1), "0m01s");
    }

    #[test]
    fn task_pipeline_struct() {
        let p = TaskPipeline {
            stages: vec![
                TaskStage {
                    tasks: vec!["a".into(), "b".into()],
                },
                TaskStage {
                    tasks: vec!["c".into()],
                },
            ],
        };
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[0].tasks.len(), 2);
    }
}
