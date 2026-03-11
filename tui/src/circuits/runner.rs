use crate::circuits::types::*;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;

/// Event emitted by a running circuit
#[derive(Debug)]
pub enum CircuitEvent {
    /// A circuit tick started
    TickStarted { circuit_id: String },
    /// A circuit tick completed with output
    #[allow(dead_code)]
    TickCompleted {
        circuit_id: String,
        output: String,
        cost: f64,
        tokens_used: u64,
        success: bool,
    },
    /// A circuit encountered an error
    Error { circuit_id: String, error: String },
}

/// Handle for a running in-session circuit
pub struct CircuitHandle {
    pub circuit: Circuit,
    pub cancel_token: CancellationToken,
}

/// Manages all in-session circuits
pub struct CircuitManager {
    pub circuits: Vec<CircuitHandle>,
    pub event_tx: mpsc::UnboundedSender<CircuitEvent>,
    pub event_rx: mpsc::UnboundedReceiver<CircuitEvent>,
    pub max_concurrent: usize,
}

impl CircuitManager {
    pub fn new(max_concurrent: usize) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        Self {
            circuits: Vec::new(),
            event_tx,
            event_rx,
            max_concurrent,
        }
    }

    /// Start a new in-session circuit. Returns error if at max capacity.
    pub fn start_circuit(&mut self, circuit: Circuit) -> Result<(), String> {
        if self.circuits.len() >= self.max_concurrent {
            return Err(format!(
                "max concurrent circuits reached ({})",
                self.max_concurrent
            ));
        }

        let cancel_token = CancellationToken::new();
        let handle = CircuitHandle {
            circuit: circuit.clone(),
            cancel_token: cancel_token.clone(),
        };

        let event_tx = self.event_tx.clone();
        let interval_secs = circuit.interval_secs;
        let circuit_id = circuit.id.clone();

        // Spawn the interval task
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(interval_secs));
            ticker.tick().await; // consume the immediate first tick
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => break,
                    _ = ticker.tick() => {
                        let _ = event_tx.send(CircuitEvent::TickStarted {
                            circuit_id: circuit_id.clone(),
                        });
                        // Actual LLM execution will be wired up in integration task
                        // For now, emit a placeholder tick
                    }
                }
            }
        });

        self.circuits.push(handle);
        Ok(())
    }

    /// Stop a specific circuit by ID
    pub fn stop_circuit(&mut self, id: &str) -> bool {
        if let Some(pos) = self.circuits.iter().position(|h| h.circuit.id == id) {
            self.circuits[pos].cancel_token.cancel();
            self.circuits.remove(pos);
            true
        } else {
            false
        }
    }

    /// Stop all in-session circuits
    pub fn stop_all(&mut self) {
        for handle in self.circuits.drain(..) {
            handle.cancel_token.cancel();
        }
    }

    /// Number of active circuits
    pub fn active_count(&self) -> usize {
        self.circuits.len()
    }

    /// Look up a circuit by ID (for reading prompt/provider/model on tick).
    pub fn get_circuit(&self, id: &str) -> Option<&Circuit> {
        self.circuits
            .iter()
            .find(|h| h.circuit.id == id)
            .map(|h| &h.circuit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_circuit(id: &str) -> Circuit {
        Circuit {
            id: id.into(),
            prompt: "test".into(),
            interval_secs: 60,
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            permission_mode: "default".into(),
            kind: CircuitKind::InSession,
            status: CircuitStatus::Active,
            last_run: None,
            next_run: None,
            created_at: "2026-03-10T00:00:00Z".into(),
            total_cost: 0.0,
            run_count: 0,
        }
    }

    #[tokio::test]
    async fn test_max_concurrent_limit() {
        let mut mgr = CircuitManager::new(2);
        assert!(mgr.start_circuit(test_circuit("a")).is_ok());
        assert!(mgr.start_circuit(test_circuit("b")).is_ok());
        assert!(mgr.start_circuit(test_circuit("c")).is_err());
    }

    #[tokio::test]
    async fn test_stop_circuit() {
        let mut mgr = CircuitManager::new(5);
        mgr.start_circuit(test_circuit("a")).unwrap();
        mgr.start_circuit(test_circuit("b")).unwrap();
        assert_eq!(mgr.active_count(), 2);
        assert!(mgr.stop_circuit("a"));
        assert_eq!(mgr.active_count(), 1);
        assert!(!mgr.stop_circuit("nonexistent"));
    }

    #[tokio::test]
    async fn test_stop_all() {
        let mut mgr = CircuitManager::new(5);
        mgr.start_circuit(test_circuit("a")).unwrap();
        mgr.start_circuit(test_circuit("b")).unwrap();
        mgr.stop_all();
        assert_eq!(mgr.active_count(), 0);
    }
}
