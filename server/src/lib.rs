//! Caboose WebSocket server — wraps caboose-core for mobile/web clients.

pub mod auth;
pub mod bridge;
pub mod mdns;
pub mod push;
pub mod shell;
pub mod state;
pub mod ws;

use anyhow::Result;
use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;
use state::AppState;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;

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
    _mdns: Option<mdns::MdnsAdvertiser>,
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
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });

    let mdns_advertiser = mdns::MdnsAdvertiser::new(local_addr.port(), local_ip()).ok();

    Ok(ServerHandle {
        local_addr,
        shutdown_tx: Some(shutdown_tx),
        state,
        _mdns: mdns_advertiser,
    })
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws::session::handle_session(socket, state))
}

async fn pair_handler(State(state): State<Arc<AppState>>) -> axum::Json<serde_json::Value> {
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
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message as TMessage};

    /// Helper: start a server on a random port with an isolated temp DB.
    /// Returns (ServerHandle, cmd_rx, tempdir).
    async fn test_server() -> (
        ServerHandle,
        tokio::sync::mpsc::UnboundedReceiver<caboose_core::events::CoreCommand>,
        tempfile::TempDir,
    ) {
        let (handle, cmd_rx) = CoreHandle::new();
        let tmp = tempfile::tempdir().unwrap();
        let config = ServerConfig {
            port: 0,
            bind: "127.0.0.1".into(),
            config: Config::default(),
            db_path: tmp.path().join("devices.db"),
        };
        let server = start_server(config, handle).await.unwrap();
        (server, cmd_rx, tmp)
    }

    /// Helper: connect a WebSocket and read the protocol version frame.
    async fn connect_ws(
        addr: &SocketAddr,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let url = format!("ws://{addr}/ws");
        let (ws, _) = connect_async(&url).await.unwrap();
        ws
    }

    /// Helper: read the next text frame as JSON.
    async fn read_json(
        ws: &mut (
                 impl StreamExt<Item = Result<TMessage, tokio_tungstenite::tungstenite::Error>> + Unpin
             ),
    ) -> serde_json::Value {
        loop {
            match ws.next().await {
                Some(Ok(TMessage::Text(text))) => {
                    return serde_json::from_str(&text).unwrap();
                }
                Some(Ok(_)) => continue, // skip ping/pong/binary
                other => panic!("expected text frame, got {:?}", other),
            }
        }
    }

    /// Helper: send a JSON object as text.
    async fn send_json(ws: &mut (impl SinkExt<TMessage> + Unpin), val: serde_json::Value) {
        ws.send(TMessage::Text(val.to_string().into()))
            .await
            .map_err(|_| "send failed")
            .unwrap();
    }

    /// Helper: GET /pair and return the pairing code string.
    async fn get_pair_code(addr: &SocketAddr) -> String {
        let url = format!("http://{addr}/pair");
        let resp: serde_json::Value = reqwest::get(&url).await.unwrap().json().await.unwrap();
        resp["code"].as_str().unwrap().to_string()
    }

    /// Helper: perform the full pair handshake on an already-connected WS.
    /// Reads protocol_version, sends Pair, reads Paired response.
    /// Returns (token, device_id).
    async fn pair_ws(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        code: &str,
    ) -> (String, String) {
        // Read protocol version.
        let proto = read_json(ws).await;
        assert_eq!(proto["protocol_version"], 1);

        // Send Pair command.
        send_json(
            ws,
            serde_json::json!({
                "id": "pair-1",
                "type": "auth",
                "command": "Pair",
                "payload": { "code": code, "device_name": "Test Device" }
            }),
        )
        .await;

        // Read Paired response.
        let resp = read_json(ws).await;
        assert_eq!(resp["type"], "auth");
        assert_eq!(resp["event"], "Paired");
        let token = resp["payload"]["token"].as_str().unwrap().to_string();
        let device_id = resp["payload"]["device_id"].as_str().unwrap().to_string();
        assert_eq!(token.len(), 64, "token should be 64 hex chars");
        (token, device_id)
    }

    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn server_starts_and_shuts_down() {
        let (server, _rx, _tmp) = test_server().await;
        assert_ne!(server.local_addr.port(), 0);
        server.shutdown();
    }

    #[tokio::test]
    async fn pair_endpoint_returns_code() {
        let (server, _rx, _tmp) = test_server().await;

        let url = format!("http://{}/pair", server.local_addr);
        let resp = reqwest::get(&url).await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body["code"].as_str().unwrap().len() == 6);
        assert!(body["address"].as_str().is_some());
        assert!(body["expires_at"].as_str().is_some());

        server.shutdown();
    }

    #[tokio::test]
    async fn pair_then_send_message() {
        let (server, mut cmd_rx, _tmp) = test_server().await;
        let addr = server.local_addr;

        // GET /pair to obtain a code.
        let code = get_pair_code(&addr).await;

        // Connect WebSocket and pair.
        let mut ws = connect_ws(&addr).await;
        let (token, _device_id) = pair_ws(&mut ws, &code).await;
        assert_eq!(token.len(), 64);

        // Send a command now that we're authenticated.
        send_json(
            &mut ws,
            serde_json::json!({
                "id": "cmd-1",
                "type": "command",
                "command": "SendMessage",
                "payload": { "text": "hello from test" }
            }),
        )
        .await;

        // Verify the command arrives at the core via cmd_rx.
        let cmd = tokio::time::timeout(std::time::Duration::from_secs(2), cmd_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match cmd {
            caboose_core::events::CoreCommand::SendMessage { text } => {
                assert_eq!(text, "hello from test");
            }
            other => panic!("expected SendMessage, got {:?}", other),
        }

        server.shutdown();
    }

    #[tokio::test]
    async fn rejects_commands_before_auth() {
        let (server, _rx, _tmp) = test_server().await;
        let addr = server.local_addr;

        let mut ws = connect_ws(&addr).await;

        // Read protocol version.
        let proto = read_json(&mut ws).await;
        assert_eq!(proto["protocol_version"], 1);

        // Send a command without authenticating.
        send_json(
            &mut ws,
            serde_json::json!({
                "id": "unauth-1",
                "type": "command",
                "command": "SendMessage",
                "payload": { "text": "should be rejected" }
            }),
        )
        .await;

        // Verify error response.
        let resp = read_json(&mut ws).await;
        assert_eq!(resp["type"], "error");
        let msg = resp["payload"]["message"].as_str().unwrap();
        assert!(
            msg.contains("Authentication required"),
            "expected 'Authentication required', got: {msg}"
        );

        server.shutdown();
    }

    #[tokio::test]
    async fn authenticate_with_stored_token() {
        let (server, mut cmd_rx, _tmp) = test_server().await;
        let addr = server.local_addr;

        // Pair on a first connection.
        let code = get_pair_code(&addr).await;
        let mut ws1 = connect_ws(&addr).await;
        let (token, _device_id) = pair_ws(&mut ws1, &code).await;
        drop(ws1); // disconnect

        // Reconnect with a fresh WebSocket.
        let mut ws2 = connect_ws(&addr).await;

        // Read protocol version.
        let proto = read_json(&mut ws2).await;
        assert_eq!(proto["protocol_version"], 1);

        // Authenticate with the stored token.
        send_json(
            &mut ws2,
            serde_json::json!({
                "id": "auth-1",
                "type": "auth",
                "command": "Authenticate",
                "payload": { "token": token, "device_name": "Test Device", "os": "test" }
            }),
        )
        .await;

        // Verify Authenticated response.
        let resp = read_json(&mut ws2).await;
        assert_eq!(resp["type"], "auth");
        assert_eq!(resp["event"], "Authenticated");
        assert!(resp["payload"]["device_id"].as_str().is_some());

        // Prove the session works by sending a command.
        send_json(
            &mut ws2,
            serde_json::json!({
                "id": "cmd-2",
                "type": "command",
                "command": "SendMessage",
                "payload": { "text": "after re-auth" }
            }),
        )
        .await;

        // Drain any commands from the first session (DeviceConnected etc. are events, not commands).
        let cmd = tokio::time::timeout(std::time::Duration::from_secs(2), cmd_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match cmd {
            caboose_core::events::CoreCommand::SendMessage { text } => {
                assert_eq!(text, "after re-auth");
            }
            other => panic!("expected SendMessage, got {:?}", other),
        }

        server.shutdown();
    }

    #[tokio::test]
    async fn revoked_device_rejected() {
        let (server, _rx, _tmp) = test_server().await;
        let addr = server.local_addr;

        // Pair to get a token and device_id.
        let code = get_pair_code(&addr).await;
        let mut ws1 = connect_ws(&addr).await;
        let (token, device_id) = pair_ws(&mut ws1, &code).await;
        drop(ws1);

        // Revoke the device via the DeviceStore on the server state.
        let revoked = server.state.devices.revoke(&device_id).unwrap();
        assert!(revoked, "revoke should return true");

        // Reconnect and try to authenticate with the revoked token.
        let mut ws2 = connect_ws(&addr).await;
        let proto = read_json(&mut ws2).await;
        assert_eq!(proto["protocol_version"], 1);

        send_json(
            &mut ws2,
            serde_json::json!({
                "id": "auth-revoked",
                "type": "auth",
                "command": "Authenticate",
                "payload": { "token": token, "device_name": "Test Device", "os": "test" }
            }),
        )
        .await;

        // Verify AuthFailed response.
        let resp = read_json(&mut ws2).await;
        assert_eq!(resp["type"], "auth");
        assert_eq!(resp["event"], "AuthFailed");
        let reason = resp["payload"]["reason"].as_str().unwrap();
        assert!(
            reason.contains("Invalid or revoked"),
            "expected 'Invalid or revoked', got: {reason}"
        );

        server.shutdown();
    }

    #[tokio::test]
    async fn remote_shell_over_websocket() {
        let (server, _rx, _tmp) = test_server().await;
        let addr = server.local_addr;

        // Pair and authenticate.
        let code = get_pair_code(&addr).await;
        let mut ws = connect_ws(&addr).await;
        let (_token, _device_id) = pair_ws(&mut ws, &code).await;

        // Send ShellSpawn command.
        send_json(
            &mut ws,
            serde_json::json!({
                "id": "shell-1",
                "type": "command",
                "command": "ShellSpawn",
                "payload": { "cols": 80, "rows": 24 }
            }),
        )
        .await;

        // Verify ShellSpawned response with shell_id.
        let resp = read_json(&mut ws).await;
        assert_eq!(
            resp["event"], "ShellSpawned",
            "expected ShellSpawned, got: {resp}"
        );
        let shell_id = resp["payload"]["shell_id"]
            .as_str()
            .expect("shell_id missing from ShellSpawned response")
            .to_string();
        assert!(!shell_id.is_empty());

        // Send ShellInput with an echo command.
        let echo_cmd = if cfg!(windows) {
            "echo hello\r\n"
        } else {
            "echo hello\n"
        };
        send_json(
            &mut ws,
            serde_json::json!({
                "id": "shell-2",
                "type": "command",
                "command": "ShellInput",
                "payload": { "shell_id": shell_id, "data": echo_cmd }
            }),
        )
        .await;

        // Read messages until we find a ShellOutput containing "hello" (with timeout).
        let found_hello = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let msg = read_json(&mut ws).await;
                if msg["event"] == "ShellOutput" {
                    let data = msg["payload"]["data"].as_str().unwrap_or("");
                    if data.contains("hello") {
                        return true;
                    }
                }
                // Skip other events (e.g. initial shell banner output)
            }
        })
        .await;
        assert!(
            found_hello.is_ok(),
            "timed out waiting for 'hello' in ShellOutput"
        );

        // Send ShellKill.
        send_json(
            &mut ws,
            serde_json::json!({
                "id": "shell-3",
                "type": "command",
                "command": "ShellKill",
                "payload": { "shell_id": shell_id }
            }),
        )
        .await;

        // Verify ShellExited response.
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), read_json(&mut ws))
            .await
            .expect("timed out waiting for ShellExited after ShellKill");
        assert_eq!(
            resp["event"], "ShellExited",
            "expected ShellExited, got: {resp}"
        );
        assert_eq!(resp["payload"]["shell_id"].as_str().unwrap(), shell_id);

        server.shutdown();
    }
}
