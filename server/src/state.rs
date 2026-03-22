//! Shared server state.

use std::path::Path;
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::Mutex;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;
use crate::auth::pairing::PairingManager;
use crate::auth::devices::DeviceStore;

pub struct AppState {
    pub core_handle: CoreHandle,
    pub config: Config,
    pub pairing: Mutex<PairingManager>,
    pub devices: DeviceStore,
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
        }))
    }
}
