//! Shared server state.

use crate::auth::devices::DeviceStore;
use crate::auth::pairing::PairingManager;
use crate::push::PushService;
use anyhow::Result;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub struct AppState {
    pub core_handle: CoreHandle,
    pub config: Config,
    pub pairing: Mutex<PairingManager>,
    pub devices: DeviceStore,
    pub push: PushService,
    /// Snapshot of conversation history, populated by the TUI when `/serve` starts.
    /// Sent to mobile clients immediately after authentication.
    pub chat_history: RwLock<Vec<serde_json::Value>>,
}

impl AppState {
    pub fn new(
        core_handle: CoreHandle,
        config: Config,
        db_path: impl AsRef<Path>,
    ) -> Result<Arc<Self>> {
        let devices = DeviceStore::new(db_path)?;
        Ok(Arc::new(Self {
            core_handle,
            config,
            pairing: Mutex::new(PairingManager::new()),
            devices,
            push: PushService::new(),
            chat_history: RwLock::new(Vec::new()),
        }))
    }
}
