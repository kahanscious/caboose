//! Shared server state.

use std::sync::Arc;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;

pub struct AppState {
    pub core_handle: CoreHandle,
    pub config: Config,
}

impl AppState {
    pub fn new(core_handle: CoreHandle, config: Config) -> Arc<Self> {
        Arc::new(Self { core_handle, config })
    }
}
