//! WebSocket session actor.
//!
//! Each connected client gets its own `handle_session` task that:
//!  - Forwards [`CoreEvent`]s from the broadcast channel to the client as JSON.
//!  - Reads JSON commands from the client and dispatches them as [`CoreCommand`]s.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::StreamExt;
use serde_json;

use crate::bridge;
use crate::state::AppState;
use crate::ws::envelope::{IncomingMessage, OutgoingMessage};

/// Drive a single WebSocket connection to completion.
///
/// The task exits when the socket closes, the client sends a `Close` frame,
/// or either channel closes unexpectedly.
pub async fn handle_session(mut socket: WebSocket, state: Arc<AppState>, device_id: String) {
    tracing::debug!("ws session started for device={device_id}");

    let mut event_rx = state.core_handle.subscribe();

    loop {
        tokio::select! {
            // Core → client: broadcast event arrives.
            result = event_rx.recv() => {
                match result {
                    Ok(event) => {
                        let msg = bridge::event_to_message(&event, "");
                        let json = serde_json::to_string(&msg).unwrap_or_default();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }

            // Client → core: incoming WebSocket frame.
            maybe_msg = socket.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<IncomingMessage>(&text) {
                            Ok(incoming) => {
                                match bridge::message_to_command(&incoming) {
                                    Ok(cmd) => {
                                        let _ = state.core_handle.send(cmd);
                                    }
                                    Err(e) => {
                                        let err = OutgoingMessage::error(&incoming.id, &e);
                                        let json = serde_json::to_string(&err).unwrap_or_default();
                                        if socket.send(Message::Text(json.into())).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("ws parse error for device={device_id}: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // Ping/Pong/Binary — ignore.
                    Some(Err(e)) => {
                        tracing::warn!("ws read error for device={device_id}: {e}");
                        break;
                    }
                }
            }
        }
    }

    tracing::debug!("ws session ended for device={device_id}");
}
