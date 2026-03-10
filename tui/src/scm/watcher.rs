#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Status of a watched PR/MR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    pub circuit_id: String,
    pub pr_number: u32,
    pub title: Option<String>,
    pub last_status: WatcherStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum WatcherStatus {
    #[default]
    Unknown,
    Open { ci: CiState, reviews: u32 },
    Merged,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum CiState {
    #[default]
    Unknown,
    Pending,
    Passing,
    Failing,
}

impl WatcherStatus {
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Unknown => "?",
            Self::Open {
                ci: CiState::Passing,
                ..
            } => "\u{2713}",
            Self::Open {
                ci: CiState::Failing,
                ..
            } => "\u{2717}",
            Self::Open {
                ci: CiState::Pending,
                ..
            } => "\u{25CC}",
            Self::Open {
                ci: CiState::Unknown,
                ..
            } => "?",
            Self::Merged => "\u{2295}",
            Self::Closed => "\u{2298}",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Merged | Self::Closed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_status_icons() {
        assert_eq!(WatcherStatus::Unknown.icon(), "?");
        assert_eq!(WatcherStatus::Merged.icon(), "\u{2295}");
        assert_eq!(
            WatcherStatus::Open {
                ci: CiState::Passing,
                reviews: 0
            }
            .icon(),
            "\u{2713}"
        );
        assert_eq!(
            WatcherStatus::Open {
                ci: CiState::Failing,
                reviews: 0
            }
            .icon(),
            "\u{2717}"
        );
    }

    #[test]
    fn terminal_states() {
        assert!(WatcherStatus::Merged.is_terminal());
        assert!(WatcherStatus::Closed.is_terminal());
        assert!(!WatcherStatus::Unknown.is_terminal());
    }
}
