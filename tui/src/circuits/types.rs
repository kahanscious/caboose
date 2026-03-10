#![allow(dead_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CircuitStatus {
    Active,
    Paused,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CircuitKind {
    /// Dies when TUI session ends
    InSession,
    /// Survives TUI close, managed by daemon
    Persistent,
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
    pub kind: CircuitKind,
    pub status: CircuitStatus,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub created_at: String,
    pub total_cost: f64,
    pub run_count: u64,
}

/// Result from a single circuit execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitRun {
    pub circuit_id: String,
    pub output: String,
    pub cost: f64,
    pub tokens_used: u64,
    pub completed_at: String,
    pub success: bool,
}
