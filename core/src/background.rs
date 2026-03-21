//! Background agent manager — tracks spawned agents, enforces budgets, and emits lifecycle events.
//!
//! This module manages agent metadata only. The actual `AgentLoop` instances are created
//! by the TUI or server layer. This manager receives registration calls and emits
//! [`CoreEvent`] variants for each lifecycle transition.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::events::{CoreEvent, CoreHandle};

// ---------------------------------------------------------------------------
// BackgroundAgentStatus
// ---------------------------------------------------------------------------

/// Current lifecycle state of a background agent.
#[derive(Debug, Clone, PartialEq)]
pub enum BackgroundAgentStatus {
    Running,
    Completed,
    Killed,
    BudgetExceeded,
    Error(String),
}

// ---------------------------------------------------------------------------
// BackgroundAgentInfo
// ---------------------------------------------------------------------------

/// Metadata record for a single background agent.
#[derive(Debug, Clone)]
pub struct BackgroundAgentInfo {
    pub id: String,
    pub prompt_summary: String,
    pub status: BackgroundAgentStatus,
    pub tokens_used: u64,
    pub budget: u64,
    pub session_id: String,
    pub parent_session_id: String,
}

// ---------------------------------------------------------------------------
// BackgroundAgentConfig
// ---------------------------------------------------------------------------

/// Configuration for the background agent manager.
#[derive(Debug, Clone)]
pub struct BackgroundAgentConfig {
    /// Per-agent token budget (default: 100_000).
    pub per_agent_budget: u64,
    /// Global token budget across all running agents (default: 500_000).
    pub global_budget: u64,
    /// Maximum number of concurrently running agents (default: 5).
    pub max_concurrent: usize,
}

impl Default for BackgroundAgentConfig {
    fn default() -> Self {
        Self {
            per_agent_budget: 100_000,
            global_budget: 500_000,
            max_concurrent: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// BackgroundAgentManager
// ---------------------------------------------------------------------------

/// Manages background agent lifecycle: registration, token tracking, budget
/// enforcement, and killing. Emits [`CoreEvent`] transitions via [`CoreHandle`].
pub struct BackgroundAgentManager {
    agents: Arc<RwLock<HashMap<String, BackgroundAgentInfo>>>,
    handles: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    config: BackgroundAgentConfig,
    core_handle: CoreHandle,
}

impl BackgroundAgentManager {
    /// Create a new manager with the given config and event handle.
    pub fn new(config: BackgroundAgentConfig, core_handle: CoreHandle) -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            handles: Arc::new(Mutex::new(HashMap::new())),
            config,
            core_handle,
        }
    }

    /// Sum of `tokens_used` across all currently-running agents.
    pub async fn total_tokens_used(&self) -> u64 {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.status == BackgroundAgentStatus::Running)
            .map(|a| a.tokens_used)
            .sum()
    }

    /// Number of agents currently in the `Running` state.
    pub async fn running_count(&self) -> usize {
        self.agents
            .read()
            .await
            .values()
            .filter(|a| a.status == BackgroundAgentStatus::Running)
            .count()
    }

    /// Check whether a new agent can be spawned.
    ///
    /// Returns `Ok(())` if within limits, or `Err(reason)` if not.
    pub async fn can_spawn(&self) -> Result<(), String> {
        let running = self.running_count().await;
        if running >= self.config.max_concurrent {
            return Err(format!(
                "max concurrent agents reached ({}/{})",
                running, self.config.max_concurrent
            ));
        }
        let total = self.total_tokens_used().await;
        if total >= self.config.global_budget {
            return Err(format!(
                "global token budget exhausted ({}/{})",
                total, self.config.global_budget
            ));
        }
        Ok(())
    }

    /// Register a new background agent and emit `BackgroundAgentStarted`.
    ///
    /// Uses `budget` if provided; otherwise falls back to `config.per_agent_budget`.
    pub async fn register(
        &self,
        id: impl Into<String>,
        prompt_summary: impl Into<String>,
        session_id: impl Into<String>,
        parent_session_id: impl Into<String>,
        budget: Option<u64>,
    ) -> BackgroundAgentInfo {
        let id = id.into();
        let prompt_summary = prompt_summary.into();
        let session_id = session_id.into();
        let parent_session_id = parent_session_id.into();
        let resolved_budget = budget.unwrap_or(self.config.per_agent_budget);

        let info = BackgroundAgentInfo {
            id: id.clone(),
            prompt_summary: prompt_summary.clone(),
            status: BackgroundAgentStatus::Running,
            tokens_used: 0,
            budget: resolved_budget,
            session_id,
            parent_session_id: parent_session_id.clone(),
        };

        self.agents.write().await.insert(id.clone(), info.clone());

        self.core_handle.emit(CoreEvent::BackgroundAgentStarted {
            id,
            prompt_summary,
            budget: resolved_budget,
            parent_session_id,
        });

        info
    }

    /// Update token usage for an agent and emit `BackgroundAgentProgress`.
    ///
    /// Returns `true` if the agent's budget has been exceeded (tokens_used >= budget).
    pub async fn update_tokens(&self, id: &str, tokens_used: u64, turn_count: u32) -> bool {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.tokens_used = tokens_used;
            let budget = agent.budget;
            let budget_remaining = budget.saturating_sub(tokens_used);
            let exceeded = tokens_used >= budget;

            self.core_handle.emit(CoreEvent::BackgroundAgentProgress {
                id: id.to_string(),
                tokens_used,
                budget_remaining,
                turn_count,
            });

            exceeded
        } else {
            false
        }
    }

    /// Mark an agent as successfully completed and emit `BackgroundAgentComplete`.
    pub async fn mark_complete(&self, id: &str) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.status = BackgroundAgentStatus::Completed;
            let tokens_used = agent.tokens_used;
            let session_id = agent.session_id.clone();

            self.core_handle.emit(CoreEvent::BackgroundAgentComplete {
                id: id.to_string(),
                tokens_used,
                session_id,
            });
        }
    }

    /// Mark an agent as failed and emit `BackgroundAgentFailed`.
    pub async fn mark_failed(&self, id: &str, reason: impl Into<String>) {
        let reason = reason.into();
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.status = BackgroundAgentStatus::Error(reason.clone());
            let tokens_used = agent.tokens_used;

            self.core_handle.emit(CoreEvent::BackgroundAgentFailed {
                id: id.to_string(),
                reason,
                tokens_used,
            });
        }
    }

    /// Abort the tokio task for an agent, set status to `Killed`, and emit `BackgroundAgentFailed`.
    ///
    /// Returns `Err` if the agent ID is unknown.
    pub async fn kill(&self, id: &str) -> Result<(), String> {
        // Abort the task handle if we have one.
        {
            let mut handles = self
                .handles
                .lock()
                .map_err(|e| format!("handles lock poisoned: {e}"))?;
            if let Some(handle) = handles.remove(id) {
                handle.abort();
            }
        }

        // Update status and emit event.
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.status = BackgroundAgentStatus::Killed;
            let tokens_used = agent.tokens_used;

            self.core_handle.emit(CoreEvent::BackgroundAgentFailed {
                id: id.to_string(),
                reason: "killed".to_string(),
                tokens_used,
            });

            Ok(())
        } else {
            Err(format!("unknown agent id: {id}"))
        }
    }

    /// Store the tokio [`JoinHandle`] for a running agent so it can be aborted via [`kill`].
    pub fn store_handle(&self, id: impl Into<String>, handle: JoinHandle<()>) {
        if let Ok(mut handles) = self.handles.lock() {
            handles.insert(id.into(), handle);
        }
    }

    /// List all registered agents (any status).
    pub async fn list(&self) -> Vec<BackgroundAgentInfo> {
        self.agents.read().await.values().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager(config: BackgroundAgentConfig) -> BackgroundAgentManager {
        let (handle, _rx) = CoreHandle::new();
        BackgroundAgentManager::new(config, handle)
    }

    fn default_manager() -> BackgroundAgentManager {
        make_manager(BackgroundAgentConfig::default())
    }

    // -----------------------------------------------------------------------
    // 1. can_spawn_within_limits
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn can_spawn_within_limits() {
        let manager = default_manager();
        assert!(
            manager.can_spawn().await.is_ok(),
            "fresh manager should allow spawning"
        );
    }

    // -----------------------------------------------------------------------
    // 2. can_spawn_rejects_at_max_concurrent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn can_spawn_rejects_at_max_concurrent() {
        let config = BackgroundAgentConfig {
            max_concurrent: 1,
            ..Default::default()
        };
        let manager = make_manager(config);

        manager
            .register("agent-1", "summary", "sess-1", "parent-1", None)
            .await;

        let result = manager.can_spawn().await;
        assert!(result.is_err(), "should be rejected at max_concurrent=1");
        assert!(
            result.unwrap_err().contains("max concurrent"),
            "error should mention max concurrent"
        );
    }

    // -----------------------------------------------------------------------
    // 3. update_tokens_returns_budget_exceeded
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_tokens_returns_budget_exceeded() {
        let config = BackgroundAgentConfig {
            per_agent_budget: 100,
            ..Default::default()
        };
        let manager = make_manager(config);
        manager
            .register("agent-1", "summary", "sess-1", "parent-1", Some(100))
            .await;

        let exceeded = manager.update_tokens("agent-1", 50, 1).await;
        assert!(!exceeded, "50 tokens should not exceed budget of 100");

        let exceeded = manager.update_tokens("agent-1", 100, 2).await;
        assert!(exceeded, "100 tokens should exceed budget of 100");
    }

    // -----------------------------------------------------------------------
    // 4. mark_complete_updates_status
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn mark_complete_updates_status() {
        let manager = default_manager();
        manager
            .register("agent-1", "summary", "sess-1", "parent-1", None)
            .await;

        manager.mark_complete("agent-1").await;

        let agents = manager.list().await;
        let agent = agents.iter().find(|a| a.id == "agent-1").unwrap();
        assert_eq!(
            agent.status,
            BackgroundAgentStatus::Completed,
            "status should be Completed after mark_complete"
        );
    }

    // -----------------------------------------------------------------------
    // 5. kill_aborts_and_updates_status
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn kill_aborts_and_updates_status() {
        let manager = default_manager();
        manager
            .register("agent-1", "summary", "sess-1", "parent-1", None)
            .await;

        // Spawn a task that sleeps for a long time so it will be alive when we kill it.
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        });
        manager.store_handle("agent-1", handle);

        let result = manager.kill("agent-1").await;
        assert!(result.is_ok(), "kill should succeed for registered agent");

        let agents = manager.list().await;
        let agent = agents.iter().find(|a| a.id == "agent-1").unwrap();
        assert_eq!(
            agent.status,
            BackgroundAgentStatus::Killed,
            "status should be Killed after kill"
        );
    }

    // -----------------------------------------------------------------------
    // 6. kill_nonexistent_returns_error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn kill_nonexistent_returns_error() {
        let manager = default_manager();
        let result = manager.kill("does-not-exist").await;
        assert!(result.is_err(), "kill of unknown id should return Err");
        assert!(
            result.unwrap_err().contains("unknown agent id"),
            "error should mention unknown agent id"
        );
    }

    // -----------------------------------------------------------------------
    // 7. events_emitted_on_lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn events_emitted_on_lifecycle() {
        let (handle, _rx) = CoreHandle::new();
        let mut subscriber = handle.subscribe();
        let manager = BackgroundAgentManager::new(BackgroundAgentConfig::default(), handle);

        manager
            .register("agent-1", "do the thing", "sess-1", "parent-1", None)
            .await;

        let event = subscriber
            .recv()
            .await
            .expect("should receive BackgroundAgentStarted event");

        match event {
            CoreEvent::BackgroundAgentStarted {
                id,
                prompt_summary,
                budget,
                parent_session_id,
            } => {
                assert_eq!(id, "agent-1");
                assert_eq!(prompt_summary, "do the thing");
                assert_eq!(budget, 100_000);
                assert_eq!(parent_session_id, "parent-1");
            }
            other => panic!("expected BackgroundAgentStarted, got {:?}", other),
        }
    }
}
