use tokio::task::JoinHandle;

/// Mouse text selection range (screen coordinates).
pub struct TextSelection {
    pub anchor_row: u16,
    pub anchor_col: u16,
    pub end_row: u16,
    pub end_col: u16,
}

/// A pending spawn_agent background task tracked by the event loop.
pub struct SpawnAgentHandle {
    pub tool_use_id: String,
    #[allow(dead_code)]
    pub arguments: serde_json::Value,
    pub chat_placeholder_idx: usize,
    pub handle: JoinHandle<crate::sub_agent::SpawnAgentResult>,
}

pub(crate) fn slice_chars(text: &str, start: usize, end: usize) -> String {
    if start >= end {
        return String::new();
    }
    text.chars().skip(start).take(end - start).collect()
}

/// Status of a tool execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    /// Awaiting user approval (shown with diff preview before execution).
    Pending,
    Running,
    Success,
    Failed,
}

/// Status of a task in the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

/// A single task in the outline.
#[derive(Debug, Clone)]
pub struct Task {
    pub content: String,
    pub active_form: String,
    pub status: TaskStatus,
}

/// Structured task outline displayed inline in the chat.
#[derive(Debug, Clone)]
pub struct TaskOutline {
    pub tasks: Vec<Task>,
}

impl TaskOutline {
    /// Parse from `todo_write` tool input JSON.
    pub fn from_tool_input(input: &serde_json::Value) -> Result<Self, String> {
        let todos = input
            .get("todos")
            .and_then(|v| v.as_array())
            .ok_or("Missing 'todos' array")?;

        if todos.is_empty() {
            return Err("Task list cannot be empty".to_string());
        }

        let tasks = todos
            .iter()
            .map(|t| {
                let content = t
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let active_form = t
                    .get("active_form")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&content)
                    .to_string();
                let status = match t.get("status").and_then(|v| v.as_str()) {
                    Some("in_progress") => TaskStatus::InProgress,
                    Some("completed") => TaskStatus::Completed,
                    Some("cancelled") => TaskStatus::Cancelled,
                    _ => TaskStatus::Pending,
                };
                Task {
                    content,
                    active_form,
                    status,
                }
            })
            .collect();

        Ok(Self { tasks })
    }

    /// Serialize to JSON for session persistence.
    pub fn to_json(&self) -> serde_json::Value {
        let todos: Vec<serde_json::Value> = self
            .tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "content": t.content,
                    "active_form": t.active_form,
                    "status": match t.status {
                        TaskStatus::Pending => "pending",
                        TaskStatus::InProgress => "in_progress",
                        TaskStatus::Completed => "completed",
                        TaskStatus::Cancelled => "cancelled",
                    }
                })
            })
            .collect();
        serde_json::json!({"todos": todos})
    }
}

/// Structured data for a tool message.
#[derive(Debug, Clone)]
pub struct ToolMessage {
    pub name: String,
    pub args: serde_json::Value,
    pub output: Option<String>,
    pub status: ToolStatus,
    pub expanded: bool,
    pub file_path: Option<String>,
    pub diff_preview: Option<Vec<String>>, // pre-computed diff lines for pending state
    /// Per-message diff expand/collapse state for post-execution diffs.
    /// For pending messages this is unused — pending diff state lives in State.diff_expanded.
    /// For post-execution edit_file / apply_patch, true = diff shown (default), false = collapsed.
    pub diff_expanded: bool,
}

/// A message in the chat display.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    User {
        content: String,
        images: Vec<(String, usize)>,
    },
    Assistant {
        content: String,
        thinking: Option<String>,
    },
    Tool(ToolMessage),
    System {
        content: String,
    },
    Error {
        content: String,
    },
    /// Structured provider error with category-specific rendering.
    ProviderError {
        category: caboose_core::provider::error::ErrorCategory,
        provider: String,
        message: String,
        hint: Option<String>,
    },
    TaskOutline(TaskOutline),
    Skill {
        name: String,
        description: String,
    },
    /// A user message queued while the agent was busy. Rendered dimmed.
    Queued {
        content: String,
    },
    /// An interactive ask-user question block.
    AskUser {
        header: String,
        question: String,
        options: Vec<(String, String)>,
        /// Selected answer, if answered. None while waiting.
        answer: Option<String>,
        multi_select: bool,
    },
}

/// Tracks file modifications during the session.
#[derive(Debug, Clone, Default)]
pub struct FileStats {
    pub additions: usize,
    pub deletions: usize,
    pub reads: usize,
}

#[cfg(test)]
mod task_outline_tests {
    use super::*;

    #[test]
    fn task_outline_from_json() {
        let json = serde_json::json!({
            "todos": [
                {"content": "Read config", "active_form": "Reading config", "status": "completed"},
                {"content": "Write handler", "active_form": "Writing handler", "status": "in_progress"},
                {"content": "Run tests", "active_form": "Running tests", "status": "pending"},
                {"content": "Old task", "active_form": "Old task", "status": "cancelled"}
            ]
        });
        let outline = TaskOutline::from_tool_input(&json).unwrap();
        assert_eq!(outline.tasks.len(), 4);
        assert_eq!(outline.tasks[0].status, TaskStatus::Completed);
        assert_eq!(outline.tasks[1].status, TaskStatus::InProgress);
        assert_eq!(outline.tasks[1].active_form, "Writing handler");
        assert_eq!(outline.tasks[2].status, TaskStatus::Pending);
        assert_eq!(outline.tasks[3].status, TaskStatus::Cancelled);
    }

    #[test]
    fn task_outline_to_json_roundtrip() {
        let outline = TaskOutline {
            tasks: vec![Task {
                content: "Do thing".into(),
                active_form: "Doing thing".into(),
                status: TaskStatus::Pending,
            }],
        };
        let json = outline.to_json();
        let restored = TaskOutline::from_tool_input(&json).unwrap();
        assert_eq!(restored.tasks.len(), 1);
        assert_eq!(restored.tasks[0].content, "Do thing");
    }

    #[test]
    fn task_outline_cancelled_roundtrip() {
        let outline = TaskOutline {
            tasks: vec![
                Task {
                    content: "Done".into(),
                    active_form: "Done".into(),
                    status: TaskStatus::Completed,
                },
                Task {
                    content: "Skipped".into(),
                    active_form: "Skipped".into(),
                    status: TaskStatus::Cancelled,
                },
            ],
        };
        let json = outline.to_json();
        let restored = TaskOutline::from_tool_input(&json).unwrap();
        assert_eq!(restored.tasks[1].status, TaskStatus::Cancelled);
    }

    #[test]
    fn task_outline_empty_returns_error() {
        let json = serde_json::json!({"todos": []});
        assert!(TaskOutline::from_tool_input(&json).is_err());
    }

    #[test]
    fn task_outline_serializes_for_storage() {
        let outline = TaskOutline {
            tasks: vec![Task {
                content: "Do X".into(),
                active_form: "Doing X".into(),
                status: TaskStatus::InProgress,
            }],
        };
        let json = outline.to_json().to_string();
        let restored: serde_json::Value = serde_json::from_str(&json).unwrap();
        let outline2 = TaskOutline::from_tool_input(&restored).unwrap();
        assert_eq!(outline2.tasks[0].content, "Do X");
        assert_eq!(outline2.tasks[0].status, TaskStatus::InProgress);
    }
}
