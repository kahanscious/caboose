/// Status of an individual LLM during planning
#[derive(Debug, Clone, PartialEq)]
pub enum PlannerStatus {
    Pending,
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
    pub plan: Option<String>,
    pub token_count: u64,
    pub cost: f64,
}

/// The overall state of a Roundhouse session
#[derive(Debug, Clone, PartialEq)]
pub enum RoundhousePhase {
    SelectingProviders,
    AwaitingPrompt,
    Planning,
    Synthesizing,
    Reviewing,
    Executing,
    Complete,
    Cancelled,
}

/// Roundhouse configuration limits
#[derive(Debug, Clone)]
pub struct RoundhouseConfig {
    pub planning_timeout_secs: u64,      // default 120
    pub per_llm_token_budget: Option<u64>,
}

impl Default for RoundhouseConfig {
    fn default() -> Self {
        Self {
            planning_timeout_secs: 120,
            per_llm_token_budget: None,
        }
    }
}
