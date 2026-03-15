/// Status of an individual LLM during planning
#[derive(Debug, Clone, PartialEq)]
pub enum PlannerStatus {
    Pending,
    #[allow(dead_code)]
    Thinking,
    Streaming,
    UsingTool(String),
    Done,
    Failed(String),
    TimedOut,
}

/// Configuration for a secondary LLM in Roundhouse
#[derive(Debug, Clone)]
pub struct SecondaryPlanner {
    pub provider_name: String,
    pub model_name: String,
    pub status: PlannerStatus,
    pub status_tick: u64,
    pub plan: Option<String>,
    #[allow(dead_code)]
    pub token_count: u64,
    #[allow(dead_code)]
    pub cost: f64,
    pub critique: Option<String>,
    pub critique_status: PlannerStatus,
    pub critique_status_tick: u64,
    pub critique_streaming_text: String,
}

/// The overall state of a Roundhouse session
#[derive(Debug, Clone, PartialEq)]
pub enum RoundhousePhase {
    SelectingProviders,
    AwaitingPrompt,
    Planning,
    Critiquing,
    Synthesizing,
    Reviewing,
    Executing,
    #[allow(dead_code)]
    Complete,
    Cancelled,
}

/// Status of a tool call within a Roundhouse planner
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallStatus {
    Running,
    Success,
    Failed,
}

/// A single tool call tracked during Roundhouse planning
#[derive(Debug, Clone)]
pub struct RoundhouseToolCall {
    pub tool_name: String,
    pub args_summary: String,
    pub status: ToolCallStatus,
    pub result_summary: Option<String>,
}

/// Roundhouse configuration limits
#[derive(Debug, Clone)]
pub struct RoundhouseConfig {
    pub planning_timeout_secs: u64, // default 120
    #[allow(dead_code)]
    pub per_llm_token_budget: Option<u64>,
    pub critique_timeout_secs: u64, // default 60
    pub critique_enabled: bool,     // default true
}

impl Default for RoundhouseConfig {
    fn default() -> Self {
        Self {
            planning_timeout_secs: 120,
            per_llm_token_budget: None,
            critique_timeout_secs: 60,
            critique_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundhouse_config_defaults() {
        let config = RoundhouseConfig::default();
        assert_eq!(config.planning_timeout_secs, 120);
        assert!(config.per_llm_token_budget.is_none());
        assert_eq!(config.critique_timeout_secs, 60);
        assert!(config.critique_enabled);
    }
}
