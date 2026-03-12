#![allow(dead_code)]

pub mod executor;
pub mod pipeline;
pub mod worktree;

use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct SubAgentStreamLine {
    pub kind: StreamLineKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum StreamLineKind {
    Thinking,
    ToolCall,
    ToolResult,
    Text,
    Error,
}

#[derive(Debug, Clone)]
pub enum SubAgentState {
    Pending,
    Running,
    Done,
    Failed { message: String },
    Conflict { report: String },
}

#[derive(Debug)]
pub struct SubAgent {
    pub id: Uuid,
    pub task: String,
    pub branch: String,
    pub worktree_path: PathBuf,
    pub state: SubAgentState,
    pub started_at: Option<Instant>,
    pub cost_usd: f64,
    pub stream: Vec<SubAgentStreamLine>,
}

impl SubAgent {
    pub fn new(task: String, branch: String, worktree_path: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4(),
            task,
            branch,
            worktree_path,
            state: SubAgentState::Pending,
            started_at: None,
            cost_usd: 0.0,
            stream: Vec::new(),
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// Format elapsed seconds. Under 1h: "XmYYs". At or over 1h: "XhYYm".
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

/// Events sent from the pipeline driver tokio task to the main thread.
#[derive(Debug)]
pub enum SubAgentEvent {
    StreamLine { id: Uuid, line: SubAgentStreamLine },
    StateChange { id: Uuid, state: SubAgentState },
    CostUpdate { id: Uuid, cost_usd: f64 },
    AgentMerged { id: Uuid, task: String, elapsed_secs: u64, cost_usd: f64 },
    AgentFailed { id: Uuid, task: String, message: String },
    AgentConflict { id: Uuid, task: String, worktree_path: PathBuf },
    PipelineDone,
    PipelineHalted { message: String },
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TaskPipeline {
    pub stages: Vec<TaskStage>,
}

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
        );
        assert!(matches!(agent.state, SubAgentState::Pending));
        assert_eq!(agent.task, "auth refactor");
        assert_eq!(agent.cost_usd, 0.0);
        assert!(agent.stream.is_empty());
        assert!(agent.started_at.is_none());
    }

    #[test]
    fn elapsed_secs_zero_when_not_started() {
        let agent = SubAgent::new("t".into(), "b".into(), std::path::PathBuf::new());
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
    fn task_pipeline_struct() {
        let p = TaskPipeline {
            stages: vec![
                TaskStage { tasks: vec!["a".into(), "b".into()] },
                TaskStage { tasks: vec!["c".into()] },
            ],
        };
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[0].tasks.len(), 2);
    }
}
