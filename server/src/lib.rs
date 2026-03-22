//! Caboose WebSocket server — wraps caboose-core for mobile/web clients.

pub mod auth;
pub mod bridge;
pub mod state;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::sync::oneshot;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;
use state::AppState;

pub struct ServerConfig {
    pub port: u16,
    pub bind: String,
    pub config: Config,
    pub db_path: std::path::PathBuf,
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
    let state = AppState::new(core_handle, config.config, &config.db_path)?;

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/pair", get(pair_handler))
        .with_state(state.clone());

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

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        ws::session::handle_session(socket, state)
    })
}

async fn pair_handler(
    State(state): State<Arc<AppState>>,
) -> axum::Json<serde_json::Value> {
    let code = {
        let mut pm = state.pairing.lock().await;
        pm.generate()
    };

    let server_addr = local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let expires_at = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();

    axum::Json(serde_json::json!({
        "code": code,
        "address": server_addr,
        "expires_at": expires_at,
    }))
}

fn local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_starts_and_shuts_down() {
        let (handle, _rx) = CoreHandle::new();
        let tmp = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            port: 0,
            bind: "127.0.0.1".into(),
            config: Config::default(),
            db_path: tmp.path().join("devices.db"),
        };
        let server = start_server(config, handle).await.unwrap();
        assert_ne!(server.local_addr.port(), 0);
        server.shutdown();
    }

    #[tokio::test]
    async fn websocket_sends_and_receives() {
        use tokio_tungstenite::connect_async;
        use futures_util::{SinkExt, StreamExt};

        let (core_handle, _rx) = CoreHandle::new();
        let event_handle = core_handle.clone();

        let tmp = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            port: 0,
            bind: "127.0.0.1".into(),
            config: Config::default(),
            db_path: tmp.path().join("devices.db"),
        };
        let server = start_server(config, core_handle).await.unwrap();
        let addr = server.local_addr;

        // Give the listener a moment to be ready.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("ws://{addr}/ws");
        let (mut ws, _) = connect_async(&url).await.unwrap();

        // Send a command from client.
        let cmd = serde_json::json!({
            "id": "test-1",
            "type": "command",
            "command": "SendMessage",
            "payload": {"text": "hello"}
        });
        ws.send(tokio_tungstenite::tungstenite::Message::Text(cmd.to_string().into()))
            .await
            .unwrap();

        // Emit an event from the core side.
        event_handle.emit(caboose_core::events::CoreEvent::TextDelta("world".into()));

        // Receive the event on the WebSocket client.
        if let Some(Ok(msg)) = ws.next().await {
            let text = msg.to_text().unwrap();
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            assert_eq!(json["event"], "TextDelta");
            assert_eq!(json["payload"]["text"], "world");
        } else {
            panic!("Expected to receive a WebSocket message");
        }

        server.shutdown();
    }

    #[tokio::test]
    async fn pair_endpoint_returns_code() {
        let (handle, _rx) = CoreHandle::new();
        let config = Config::default();
        let tmp = tempfile::tempdir().unwrap();
        let server = start_server(
            ServerConfig { port: 0, bind: "127.0.0.1".into(), config, db_path: tmp.path().join("devices.db") },
            handle,
        ).await.unwrap();

        let url = format!("http://{}/pair", server.local_addr);
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["code"].as_str().unwrap().len() == 6);
        assert!(body["address"].as_str().is_some());
        assert!(body["expires_at"].as_str().is_some());

        server.shutdown();
    }
}
