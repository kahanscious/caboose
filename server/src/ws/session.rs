//! WebSocket session actor with auth gate, heartbeat, and backpressure.
//!
//! Each connected client gets its own `handle_session` task that:
//!  1. Sends a protocol version handshake.
//!  2. Runs an auth loop (Pair / Authenticate only).
//!  3. Enters the authenticated event loop with heartbeat and TextDelta
//!     backpressure.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::StreamExt;
use serde_json::{self, json, Value};
use tokio::time::{self, Instant};

use caboose_core::events::CoreEvent;

use crate::bridge::{self, AuthCommand};
use crate::state::AppState;
use crate::ws::envelope::{IncomingMessage, OutgoingMessage};

/// Current protocol version sent on every new connection.
const PROTOCOL_VERSION: u32 = 1;

/// Interval between server-initiated WebSocket pings.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// TextDelta events are buffered and flushed at most this often.
const TEXT_DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Drive a single WebSocket connection to completion.
///
/// The session goes through three phases:
///   1. **Protocol version** — send `{ "protocol_version": N }`.
///   2. **Auth loop** — only `Pair` / `Authenticate` accepted.
///   3. **Authenticated loop** — full event/command relay with heartbeat and
///      TextDelta backpressure.
pub async fn handle_session(mut socket: WebSocket, state: Arc<AppState>) {
    // ------------------------------------------------------------------
    // Phase 1: protocol version
    // ------------------------------------------------------------------
    let version_msg = json!({ "protocol_version": PROTOCOL_VERSION });
    if send_text(&mut socket, &version_msg).await.is_err() {
        return;
    }

    // ------------------------------------------------------------------
    // Phase 2: auth loop
    // ------------------------------------------------------------------
    let device = match auth_loop(&mut socket, &state).await {
        Some(dev) => dev,
        None => return, // socket closed or auth never completed
    };

    let device_id = device.id.clone();
    let device_name = device.name.clone();

    // Notify core that a device connected.
    state.core_handle.emit(CoreEvent::DeviceConnected {
        device_id: device_id.clone(),
        device_name: device_name.clone(),
    });

    tracing::info!("device authenticated: id={device_id} name={device_name}");

    // ------------------------------------------------------------------
    // Phase 3: authenticated event loop
    // ------------------------------------------------------------------
    let (shell_tx, shell_rx) = tokio::sync::mpsc::unbounded_channel();
    let shell_mgr = crate::shell::manager::ShellManager::new(shell_tx);
    authenticated_loop(&mut socket, &state, &device_id, shell_mgr, shell_rx).await;

    // Notify core that the device disconnected.
    state.core_handle.emit(CoreEvent::DeviceDisconnected {
        device_id: device_id.clone(),
    });

    tracing::info!("session ended for device={device_id}");
}

// ---------------------------------------------------------------------------
// Phase 2 — auth loop
// ---------------------------------------------------------------------------

/// Block until the client successfully pairs or authenticates.
///
/// Returns `Some(Device)` on success, `None` if the socket closes first.
async fn auth_loop(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
) -> Option<crate::auth::devices::Device> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                let incoming: IncomingMessage = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("ws auth parse error: {e}");
                        continue;
                    }
                };

                // Only auth messages allowed before authentication.
                if incoming.msg_type != "auth" {
                    let err = OutgoingMessage::error(
                        &incoming.id,
                        "Authentication required",
                    );
                    if send_json(socket, &err).await.is_err() {
                        return None;
                    }
                    continue;
                }

                match bridge::parse_auth_command(&incoming) {
                    Ok(AuthCommand::Pair { code, device_name }) => {
                        let mut pairing = state.pairing.lock().await;
                        if pairing.validate(&code) {
                            let device_id = uuid::Uuid::new_v4().to_string();
                            match state.devices.pair(&device_id, &device_name) {
                                Ok(token) => {
                                    drop(pairing);
                                    let resp = OutgoingMessage::paired(
                                        &incoming.id,
                                        &token,
                                        &device_id,
                                    );
                                    if send_json(socket, &resp).await.is_err() {
                                        return None;
                                    }
                                    // After pairing, the client should authenticate
                                    // with the token, but we can auto-authenticate
                                    // since pairing is proof enough.
                                    return Some(crate::auth::devices::Device {
                                        id: device_id,
                                        name: device_name,
                                        paired_at: chrono::Utc::now().to_rfc3339(),
                                        last_seen: Some(chrono::Utc::now().to_rfc3339()),
                                    });
                                }
                                Err(e) => {
                                    drop(pairing);
                                    let resp = OutgoingMessage::auth_failed(
                                        &incoming.id,
                                        &e.to_string(),
                                    );
                                    if send_json(socket, &resp).await.is_err() {
                                        return None;
                                    }
                                }
                            }
                        } else {
                            drop(pairing);
                            let resp = OutgoingMessage::auth_failed(
                                &incoming.id,
                                "Invalid pairing code",
                            );
                            if send_json(socket, &resp).await.is_err() {
                                return None;
                            }
                        }
                    }

                    Ok(AuthCommand::Authenticate { token, device_name: _, os: _ }) => {
                        match state.devices.verify(&token) {
                            Ok(Some(device)) => {
                                let resp = OutgoingMessage::authenticated(
                                    &incoming.id,
                                    &device.id,
                                    &device.name,
                                );
                                if send_json(socket, &resp).await.is_err() {
                                    return None;
                                }
                                return Some(device);
                            }
                            Ok(None) => {
                                let resp = OutgoingMessage::auth_failed(
                                    &incoming.id,
                                    "Invalid or revoked token",
                                );
                                if send_json(socket, &resp).await.is_err() {
                                    return None;
                                }
                            }
                            Err(e) => {
                                let resp = OutgoingMessage::auth_failed(
                                    &incoming.id,
                                    &format!("Auth error: {e}"),
                                );
                                if send_json(socket, &resp).await.is_err() {
                                    return None;
                                }
                            }
                        }
                    }

                    Ok(_) => {
                        // ListDevices / RevokeDevice not allowed pre-auth.
                        let resp = OutgoingMessage::auth_failed(
                            &incoming.id,
                            "Authentication required before using this command",
                        );
                        if send_json(socket, &resp).await.is_err() {
                            return None;
                        }
                    }

                    Err(e) => {
                        let resp = OutgoingMessage::auth_failed(&incoming.id, &e);
                        if send_json(socket, &resp).await.is_err() {
                            return None;
                        }
                    }
                }
            }

            Some(Ok(Message::Close(_))) | None => return None,
            Some(Ok(_)) => {} // Ping/Pong/Binary — ignore.
            Some(Err(e)) => {
                tracing::warn!("ws read error during auth: {e}");
                return None;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3 — authenticated event loop
// ---------------------------------------------------------------------------

async fn authenticated_loop(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    device_id: &str,
    mut shell_mgr: crate::shell::manager::ShellManager,
    mut shell_rx: tokio::sync::mpsc::UnboundedReceiver<crate::shell::manager::ShellEvent>,
) {
    let mut event_rx = state.core_handle.subscribe();
    let mut heartbeat = time::interval(HEARTBEAT_INTERVAL);
    heartbeat.reset(); // don't fire immediately

    // TextDelta backpressure: buffer deltas and flush periodically.
    let mut text_delta_buf: Vec<OutgoingMessage> = Vec::new();
    let mut flush_deadline: Option<Instant> = None;

    loop {
        // Compute the flush sleep future.
        let flush_sleep = match flush_deadline {
            Some(deadline) => time::sleep_until(deadline),
            None => time::sleep(Duration::from_secs(86400)), // effectively never
        };

        tokio::select! {
            // Heartbeat tick — send Ping.
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }

            // Flush buffered TextDelta events.
            _ = flush_sleep, if flush_deadline.is_some() => {
                for msg in text_delta_buf.drain(..) {
                    if send_json(socket, &msg).await.is_err() {
                        return;
                    }
                }
                flush_deadline = None;
            }

            // Shell → client: PTY output or exit notification.
            shell_event = shell_rx.recv() => {
                if let Some(event) = shell_event {
                    let out = match &event {
                        crate::shell::manager::ShellEvent::Output { shell_id, data } => {
                            crate::bridge::shell_output_message(device_id, shell_id, data)
                        }
                        crate::shell::manager::ShellEvent::Exited { shell_id, exit_code } => {
                            crate::bridge::shell_exited_message(device_id, shell_id, *exit_code)
                        }
                    };
                    let json = serde_json::to_string(&out).unwrap();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                    // Clean up dead shells
                    if let crate::shell::manager::ShellEvent::Exited { shell_id, .. } = &event {
                        shell_mgr.kill(shell_id).ok();
                    }
                }
            }

            // Core → client: broadcast event arrives.
            result = event_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = bridge::event_to_message(&event, "");

                        // TextDelta backpressure: buffer and flush on interval.
                        if matches!(event, CoreEvent::TextDelta(_)) {
                            text_delta_buf.push(msg);
                            if flush_deadline.is_none() {
                                flush_deadline = Some(Instant::now() + TEXT_DELTA_FLUSH_INTERVAL);
                            }
                        } else {
                            // Flush any pending TextDeltas before non-delta events.
                            for buffered in text_delta_buf.drain(..) {
                                if send_json(socket, &buffered).await.is_err() {
                                    return;
                                }
                            }
                            flush_deadline = None;

                            if send_json(socket, &msg).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("device={device_id} lagged {n} events");
                    }
                    Err(_) => break,
                }
            }

            // Client → core: incoming WebSocket frame.
            maybe_msg = socket.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        let incoming: IncomingMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!("ws parse error device={device_id}: {e}");
                                continue;
                            }
                        };

                        if incoming.msg_type == "auth" {
                            // Post-auth device management commands.
                            handle_auth_command(socket, state, &incoming).await;
                        } else if incoming.msg_type == "command" {
                            // Shell commands — handled before regular command routing.
                            if let Some(ref cmd_name) = incoming.command {
                                if cmd_name.starts_with("Shell") {
                                    match crate::bridge::parse_shell_command(&incoming) {
                                        Ok(shell_cmd) => {
                                            use crate::bridge::ShellCommand;
                                            let response = match shell_cmd {
                                                ShellCommand::Spawn { cols, rows } => {
                                                    match shell_mgr.spawn(cols, rows) {
                                                        Ok(shell_id) => OutgoingMessage::event(
                                                            &incoming.id,
                                                            "ShellSpawned",
                                                            serde_json::json!({ "shell_id": shell_id }),
                                                        ),
                                                        Err(e) => OutgoingMessage::error(&incoming.id, &e),
                                                    }
                                                }
                                                ShellCommand::Input { shell_id, data } => {
                                                    if let Err(e) = shell_mgr.write(&shell_id, &data) {
                                                        OutgoingMessage::error(&incoming.id, &e)
                                                    } else {
                                                        continue; // No response needed for input
                                                    }
                                                }
                                                ShellCommand::Resize { shell_id, cols, rows } => {
                                                    if let Err(e) = shell_mgr.resize(&shell_id, cols, rows) {
                                                        OutgoingMessage::error(&incoming.id, &e)
                                                    } else {
                                                        continue; // No response needed for resize
                                                    }
                                                }
                                                ShellCommand::Kill { shell_id } => {
                                                    match shell_mgr.kill(&shell_id) {
                                                        Ok(()) => OutgoingMessage::event(
                                                            &incoming.id,
                                                            "ShellExited",
                                                            serde_json::json!({ "shell_id": shell_id, "exit_code": -1 }),
                                                        ),
                                                        Err(e) => OutgoingMessage::error(&incoming.id, &e),
                                                    }
                                                }
                                            };
                                            let _ = socket.send(Message::Text(
                                                serde_json::to_string(&response).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                        Err(e) => {
                                            let _ = socket.send(Message::Text(
                                                serde_json::to_string(&OutgoingMessage::error(&incoming.id, &e)).unwrap().into()
                                            )).await;
                                            continue;
                                        }
                                    }
                                }
                            }

                            match bridge::message_to_command(&incoming) {
                                Ok(cmd) => {
                                    let _ = state.core_handle.send(cmd);
                                }
                                Err(e) => {
                                    let err = OutgoingMessage::error(&incoming.id, &e);
                                    if send_json(socket, &err).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        } else {
                            let err = OutgoingMessage::error(
                                &incoming.id,
                                &format!("Unknown message type: {}", incoming.msg_type),
                            );
                            if send_json(socket, &err).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // Ping/Pong/Binary — ignore.
                    Some(Err(e)) => {
                        tracing::warn!("ws read error device={device_id}: {e}");
                        break;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post-auth device management
// ---------------------------------------------------------------------------

async fn handle_auth_command(
    socket: &mut WebSocket,
    state: &Arc<AppState>,
    incoming: &IncomingMessage,
) {
    match bridge::parse_auth_command(incoming) {
        Ok(AuthCommand::ListDevices) => {
            match state.devices.list() {
                Ok(devices) => {
                    let list: Vec<Value> = devices
                        .iter()
                        .map(|d| {
                            json!({
                                "id": d.id,
                                "name": d.name,
                                "paired_at": d.paired_at,
                                "last_seen": d.last_seen,
                            })
                        })
                        .collect();
                    let resp = OutgoingMessage::auth(&incoming.id, "DeviceList", json!({ "devices": list }));
                    let _ = send_json(socket, &resp).await;
                }
                Err(e) => {
                    let resp = OutgoingMessage::error(&incoming.id, &format!("Failed to list devices: {e}"));
                    let _ = send_json(socket, &resp).await;
                }
            }
        }
        Ok(AuthCommand::RevokeDevice { device_id }) => {
            match state.devices.revoke(&device_id) {
                Ok(true) => {
                    let resp = OutgoingMessage::auth(&incoming.id, "DeviceRevoked", json!({ "device_id": device_id }));
                    let _ = send_json(socket, &resp).await;
                }
                Ok(false) => {
                    let resp = OutgoingMessage::error(&incoming.id, "Device not found or already revoked");
                    let _ = send_json(socket, &resp).await;
                }
                Err(e) => {
                    let resp = OutgoingMessage::error(&incoming.id, &format!("Failed to revoke device: {e}"));
                    let _ = send_json(socket, &resp).await;
                }
            }
        }
        Ok(_) => {
            // Pair / Authenticate after already authenticated — reject.
            let resp = OutgoingMessage::error(&incoming.id, "Already authenticated");
            let _ = send_json(socket, &resp).await;
        }
        Err(e) => {
            let resp = OutgoingMessage::error(&incoming.id, &e);
            let _ = send_json(socket, &resp).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialize and send an `OutgoingMessage`. Returns `Err(())` if the socket
/// is closed.
async fn send_json(socket: &mut WebSocket, msg: &OutgoingMessage) -> Result<(), ()> {
    let json = serde_json::to_string(msg).unwrap_or_default();
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

/// Send a raw JSON value as text. Returns `Err(())` if the socket is closed.
async fn send_text(socket: &mut WebSocket, value: &Value) -> Result<(), ()> {
    let json = serde_json::to_string(value).unwrap_or_default();
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}
