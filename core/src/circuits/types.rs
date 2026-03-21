use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CircuitStatus {
    Active,
    Paused,
    Error(String),
}

/// A recurring scheduled task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Circuit {
    pub id: String,
    pub prompt: String,
    pub interval_secs: u64,
    pub provider: String,
    pub model: String,
    pub permission_mode: String,
    pub status: CircuitStatus,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub created_at: String,
    pub total_cost: f64,
    pub run_count: u64,
}
