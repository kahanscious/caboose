//! Caboose WebSocket server — wraps caboose-core for mobile/web clients.

pub mod state;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use axum::Router;
use tokio::sync::oneshot;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;
use state::AppState;

pub struct ServerConfig {
    pub port: u16,
    pub bind: String,
    pub config: Config,
}

pub struct ServerHandle {
    pub local_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    pub state: Arc<AppState>,
}

impl ServerHandle {
    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

pub async fn start_server(config: ServerConfig, core_handle: CoreHandle) -> Result<ServerHandle> {
    let state = AppState::new(core_handle, config.config);
    let app = Router::new().with_state(state.clone());

    let addr: SocketAddr = format!("{}:{}", config.bind, config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    tokio::spawn(async move {
        tracing::info!("caboose-server listening on {}", local_addr);
        axum::serve(listener, app)
            .with_graceful_shutdown(async { let _ = shutdown_rx.await; })
            .await
            .ok();
    });

    Ok(ServerHandle {
        local_addr,
        shutdown_tx: Some(shutdown_tx),
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_starts_and_shuts_down() {
        let (handle, _rx) = CoreHandle::new();
        let config = ServerConfig {
            port: 0,
            bind: "127.0.0.1".into(),
            config: Config::default(),
        };
        let server = start_server(config, handle).await.unwrap();
        assert_ne!(server.local_addr.port(), 0);
        server.shutdown();
    }
}
