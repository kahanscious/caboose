//! Bridge between [`CoreEvent`]/[`CoreCommand`] and the WebSocket JSON protocol.
//!
//! - [`event_to_message`] serialises a [`CoreEvent`] into an [`OutgoingMessage`].
//! - [`message_to_command`] parses an [`IncomingMessage`] into a [`CoreCommand`].

use caboose_core::events::{CoreCommand, CoreEvent};
use serde_json::{Value, json};

use crate::ws::envelope::{IncomingMessage, OutgoingMessage};

// ---------------------------------------------------------------------------
// CoreEvent → OutgoingMessage
// ---------------------------------------------------------------------------

/// Convert a [`CoreEvent`] into an [`OutgoingMessage`] destined for the client.
///
/// The `id` parameter is the correlation id to embed in the message; pass `""`
/// for unsolicited server-push events.
pub fn event_to_message(event: &CoreEvent, id: &str) -> OutgoingMessage {
    match event {
        CoreEvent::TextDelta(text) => {
            OutgoingMessage::event(id, "TextDelta", json!({ "text": text }))
        }

        CoreEvent::ThinkingDelta(text) => {
            OutgoingMessage::event(id, "ThinkingDelta", json!({ "text": text }))
        }

        CoreEvent::ToolCall {
            id: tool_id,
            name,
            arguments,
        } => OutgoingMessage::event(
            id,
            "ToolCall",
            json!({ "id": tool_id, "name": name, "arguments": arguments }),
        ),

        CoreEvent::TurnComplete {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
        } => OutgoingMessage::event(
            id,
            "TurnComplete",
            json!({
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_read_tokens": cache_read_tokens,
                "cache_creation_tokens": cache_creation_tokens,
            }),
        ),

        CoreEvent::Error(msg) => OutgoingMessage::event(id, "Error", json!({ "message": msg })),

        CoreEvent::ProviderError {
            category,
            provider,
            message,
            hint,
        } => OutgoingMessage::event(
            id,
            "ProviderError",
            json!({
                "category": category,
                "provider": provider,
                "message": message,
                "hint": hint,
            }),
        ),

        CoreEvent::CompactionComplete => {
            OutgoingMessage::event(id, "CompactionComplete", json!({}))
        }

        CoreEvent::ToolApprovalRequired {
            tool_calls,
            current_index,
        } => {
            let calls: Vec<Value> = tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "name": tc.name,
                        "arguments": tc.arguments,
                    })
                })
                .collect();
            OutgoingMessage::event(
                id,
                "ToolApprovalRequired",
                json!({ "tool_calls": calls, "current_index": current_index }),
            )
        }

        CoreEvent::ToolExecuted(result) => OutgoingMessage::event(
            id,
            "ToolExecuted",
            serde_json::json!({
                "tool_use_id": result.tool_use_id,
                "tool_name": result.tool_name,
                "output": result.output,
                "is_error": result.is_error,
                "file_path": result.file_path,
            }),
        ),

        // --- Session events ---
        CoreEvent::SessionCreated(session) => {
            OutgoingMessage::event(id, "SessionCreated", json!({ "session_id": session.id }))
        }

        CoreEvent::SessionList(sessions) => {
            let list: Vec<Value> = sessions.iter().map(|s| json!({ "id": s.id })).collect();
            OutgoingMessage::event(id, "SessionList", json!({ "sessions": list }))
        }

        CoreEvent::SessionLoaded { session, messages } => {
            let msg_list: Vec<Value> = messages
                .iter()
                .map(|m| {
                    json!({
                        "role": m.role,
                        "content": m.content,
                    })
                })
                .collect();
            OutgoingMessage::event(
                id,
                "SessionLoaded",
                json!({
                    "session_id": session.id,
                    "messages": msg_list,
                }),
            )
        }

        CoreEvent::SessionDeleted { session_id } => {
            OutgoingMessage::event(id, "SessionDeleted", json!({ "session_id": session_id }))
        }

        // --- Provider ---
        CoreEvent::ProviderSwitched { provider, model } => OutgoingMessage::event(
            id,
            "ProviderSwitched",
            json!({ "provider": provider, "model": model }),
        ),

        CoreEvent::ModelList(models) => {
            let list: Vec<Value> = models
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "name": m.name,
                        "context_window": m.context_window,
                        "supports_tools": m.supports_tools,
                        "supports_vision": m.supports_vision,
                        "supports_thinking": m.supports_thinking,
                    })
                })
                .collect();
            OutgoingMessage::event(id, "ModelList", json!({ "models": list }))
        }

        // --- MCP ---
        CoreEvent::McpServerConnected { name } => {
            OutgoingMessage::event(id, "McpServerConnected", json!({ "name": name }))
        }

        CoreEvent::McpServerDisconnected { name } => {
            OutgoingMessage::event(id, "McpServerDisconnected", json!({ "name": name }))
        }

        CoreEvent::McpToolsDiscovered { server, tools: _ } => {
            OutgoingMessage::event(id, "McpToolsDiscovered", json!({ "server": server }))
        }

        // --- Background agents ---
        CoreEvent::BackgroundAgentStarted {
            id: agent_id,
            prompt_summary,
            budget,
            parent_session_id,
        } => OutgoingMessage::event(
            id,
            "BackgroundAgentStarted",
            json!({
                "id": agent_id,
                "prompt_summary": prompt_summary,
                "budget": budget,
                "parent_session_id": parent_session_id,
            }),
        ),

        CoreEvent::BackgroundAgentProgress {
            id: agent_id,
            tokens_used,
            budget_remaining,
            turn_count,
        } => OutgoingMessage::event(
            id,
            "BackgroundAgentProgress",
            json!({
                "id": agent_id,
                "tokens_used": tokens_used,
                "budget_remaining": budget_remaining,
                "turn_count": turn_count,
            }),
        ),

        CoreEvent::BackgroundAgentComplete {
            id: agent_id,
            tokens_used,
            session_id,
        } => OutgoingMessage::event(
            id,
            "BackgroundAgentComplete",
            json!({
                "id": agent_id,
                "tokens_used": tokens_used,
                "session_id": session_id,
            }),
        ),

        CoreEvent::BackgroundAgentFailed {
            id: agent_id,
            reason,
            tokens_used,
        } => OutgoingMessage::event(
            id,
            "BackgroundAgentFailed",
            json!({
                "id": agent_id,
                "reason": reason,
                "tokens_used": tokens_used,
            }),
        ),

        // --- Checkpoints ---
        CoreEvent::CheckpointCreated { name } => {
            OutgoingMessage::event(id, "CheckpointCreated", json!({ "name": name }))
        }

        CoreEvent::CheckpointRewound { name } => {
            OutgoingMessage::event(id, "CheckpointRewound", json!({ "name": name }))
        }

        // --- Roundhouse ---
        CoreEvent::RoundhousePhaseChanged { phase } => {
            OutgoingMessage::event(id, "RoundhousePhaseChanged", json!({ "phase": phase }))
        }

        CoreEvent::RoundhouseComplete { plan } => {
            OutgoingMessage::event(id, "RoundhouseComplete", json!({ "plan": plan }))
        }

        // --- Status ---
        CoreEvent::Status {
            provider,
            model,
            session_id,
            permission_mode,
        } => OutgoingMessage::event(
            id,
            "Status",
            json!({
                "provider": provider,
                "model": model,
                "session_id": session_id,
                "permission_mode": permission_mode,
            }),
        ),

        // --- Device connectivity ---
        CoreEvent::DeviceConnected {
            device_id,
            device_name,
        } => OutgoingMessage::event(
            id,
            "DeviceConnected",
            json!({ "device_id": device_id, "device_name": device_name }),
        ),

        CoreEvent::DeviceDisconnected { device_id } => {
            OutgoingMessage::event(id, "DeviceDisconnected", json!({ "device_id": device_id }))
        }

        CoreEvent::SessionHistory { messages } => {
            OutgoingMessage::event(id, "SessionHistory", json!({ "messages": messages }))
        }

        CoreEvent::ServerShutdown => {
            OutgoingMessage::event(id, "ServerShutdown", json!({}))
        }
    }
}

// ---------------------------------------------------------------------------
// IncomingMessage → CoreCommand
// ---------------------------------------------------------------------------

/// Parse an [`IncomingMessage`] into a [`CoreCommand`].
///
/// Returns `Err` with a human-readable description when the command is unknown
/// or required payload fields are missing.
pub fn message_to_command(msg: &IncomingMessage) -> Result<CoreCommand, String> {
    let command = msg
        .command
        .as_deref()
        .ok_or_else(|| "message has no 'command' field".to_string())?;

    let payload = msg.payload.as_ref();

    match command {
        "SendMessage" => {
            let text = payload
                .and_then(|p| p["text"].as_str())
                .ok_or("SendMessage requires payload.text")?
                .to_string();
            Ok(CoreCommand::SendMessage { text })
        }

        "CancelTurn" => Ok(CoreCommand::CancelTurn),
        "ApproveTool" => Ok(CoreCommand::ApproveTool),
        "DenyTool" => Ok(CoreCommand::DenyTool),
        "AlwaysAllowTool" => Ok(CoreCommand::AlwaysAllowTool),
        "CreateSession" => Ok(CoreCommand::CreateSession),

        "ListSessions" => {
            let limit = payload
                .and_then(|p| p["limit"].as_u64())
                .map(|v| v as usize)
                .unwrap_or(20);
            Ok(CoreCommand::ListSessions { limit })
        }

        "LoadSession" => {
            let session_id = payload
                .and_then(|p| p["session_id"].as_str())
                .ok_or("LoadSession requires payload.session_id")?
                .to_string();
            Ok(CoreCommand::LoadSession { session_id })
        }

        "DeleteSession" => {
            let session_id = payload
                .and_then(|p| p["session_id"].as_str())
                .ok_or("DeleteSession requires payload.session_id")?
                .to_string();
            Ok(CoreCommand::DeleteSession { session_id })
        }

        "GetStatus" => Ok(CoreCommand::GetStatus),

        "SpawnBackgroundAgent" => {
            let prompt = payload
                .and_then(|p| p["prompt"].as_str())
                .ok_or("SpawnBackgroundAgent requires payload.prompt")?
                .to_string();
            let budget = payload.and_then(|p| p["budget"].as_u64());
            Ok(CoreCommand::SpawnBackgroundAgent { prompt, budget })
        }

        "KillBackgroundAgent" => {
            let id = payload
                .and_then(|p| p["id"].as_str())
                .ok_or("KillBackgroundAgent requires payload.id")?
                .to_string();
            Ok(CoreCommand::KillBackgroundAgent { id })
        }

        // RegisterPushToken is handled directly by the session, not forwarded to core.
        "RegisterPushToken" => Err("RegisterPushToken is handled by the server".to_string()),

        other => Err(format!("Unknown command: '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// Auth command parsing
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum AuthCommand {
    Pair {
        code: String,
        device_name: String,
    },
    Authenticate {
        token: String,
        device_name: String,
        os: String,
    },
    ListDevices,
    RevokeDevice {
        device_id: String,
    },
}

pub fn parse_auth_command(msg: &IncomingMessage) -> Result<AuthCommand, String> {
    let cmd = msg.command.as_deref().unwrap_or("");
    let payload = msg.payload.as_ref();

    match cmd {
        "Pair" => {
            let p = payload.ok_or("Pair requires payload")?;
            let code = p
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or("Pair requires 'code'")?
                .to_string();
            let device_name = p
                .get("device_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(AuthCommand::Pair { code, device_name })
        }
        "Authenticate" => {
            let p = payload.ok_or("Authenticate requires payload")?;
            let token = p
                .get("token")
                .and_then(|v| v.as_str())
                .ok_or("Authenticate requires 'token'")?
                .to_string();
            let device_name = p
                .get("device_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let os = p
                .get("os")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            Ok(AuthCommand::Authenticate {
                token,
                device_name,
                os,
            })
        }
        "ListDevices" => Ok(AuthCommand::ListDevices),
        "RevokeDevice" => {
            let p = payload.ok_or("RevokeDevice requires payload")?;
            let device_id = p
                .get("device_id")
                .and_then(|v| v.as_str())
                .ok_or("RevokeDevice requires 'device_id'")?
                .to_string();
            Ok(AuthCommand::RevokeDevice { device_id })
        }
        _ => Err(format!("Unknown auth command: {}", cmd)),
    }
}

// ---------------------------------------------------------------------------
// Shell command parsing
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum ShellCommand {
    Spawn {
        cols: u16,
        rows: u16,
    },
    Input {
        shell_id: String,
        data: String,
    },
    Resize {
        shell_id: String,
        cols: u16,
        rows: u16,
    },
    Kill {
        shell_id: String,
    },
}

pub fn parse_shell_command(msg: &IncomingMessage) -> Result<ShellCommand, String> {
    let cmd = msg.command.as_deref().unwrap_or("");
    let payload = msg.payload.as_ref();

    match cmd {
        "ShellSpawn" => {
            let p = payload.ok_or("ShellSpawn requires payload")?;
            let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
            let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
            Ok(ShellCommand::Spawn { cols, rows })
        }
        "ShellInput" => {
            let p = payload.ok_or("ShellInput requires payload")?;
            let shell_id = p
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or("ShellInput requires 'shell_id'")?
                .to_string();
            let data = p
                .get("data")
                .and_then(|v| v.as_str())
                .ok_or("ShellInput requires 'data'")?
                .to_string();
            Ok(ShellCommand::Input { shell_id, data })
        }
        "ShellResize" => {
            let p = payload.ok_or("ShellResize requires payload")?;
            let shell_id = p
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or("ShellResize requires 'shell_id'")?
                .to_string();
            let cols = p.get("cols").and_then(|v| v.as_u64()).unwrap_or(80) as u16;
            let rows = p.get("rows").and_then(|v| v.as_u64()).unwrap_or(24) as u16;
            Ok(ShellCommand::Resize {
                shell_id,
                cols,
                rows,
            })
        }
        "ShellKill" => {
            let p = payload.ok_or("ShellKill requires payload")?;
            let shell_id = p
                .get("shell_id")
                .and_then(|v| v.as_str())
                .ok_or("ShellKill requires 'shell_id'")?
                .to_string();
            Ok(ShellCommand::Kill { shell_id })
        }
        _ => Err(format!("Unknown shell command: {}", cmd)),
    }
}

// ---------------------------------------------------------------------------
// Shell event serialization
// ---------------------------------------------------------------------------

pub fn shell_output_message(id: &str, shell_id: &str, data: &str) -> OutgoingMessage {
    OutgoingMessage::event(
        id,
        "ShellOutput",
        serde_json::json!({
            "shell_id": shell_id,
            "data": data,
        }),
    )
}

pub fn shell_exited_message(id: &str, shell_id: &str, exit_code: i32) -> OutgoingMessage {
    OutgoingMessage::event(
        id,
        "ShellExited",
        serde_json::json!({
            "shell_id": shell_id,
            "exit_code": exit_code,
        }),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_delta_event_to_json() {
        let event = CoreEvent::TextDelta("hello world".into());
        let msg = event_to_message(&event, "msg-1");
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["event"], "TextDelta");
        assert_eq!(v["payload"]["text"], "hello world");
        assert_eq!(v["id"], "msg-1");
    }

    #[test]
    fn send_message_command_parsed() {
        let incoming = IncomingMessage {
            id: "cmd-1".into(),
            msg_type: "command".into(),
            command: Some("SendMessage".into()),
            payload: Some(json!({ "text": "hi there" })),
        };
        let cmd = message_to_command(&incoming).unwrap();
        match cmd {
            CoreCommand::SendMessage { text } => assert_eq!(text, "hi there"),
            other => panic!("expected SendMessage, got {:?}", other),
        }
    }

    #[test]
    fn unknown_command_returns_error() {
        let incoming = IncomingMessage {
            id: "cmd-2".into(),
            msg_type: "command".into(),
            command: Some("DoSomethingWeird".into()),
            payload: None,
        };
        let result = message_to_command(&incoming);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Unknown command"), "got: {msg}");
        assert!(msg.contains("DoSomethingWeird"), "got: {msg}");
    }

    #[test]
    fn parse_pair_command() {
        let msg = IncomingMessage {
            id: "1".into(),
            msg_type: "auth".into(),
            command: Some("Pair".into()),
            payload: Some(serde_json::json!({
                "code": "A3B7K2",
                "device_name": "Pixel 8"
            })),
        };
        let result = parse_auth_command(&msg);
        assert!(result.is_ok());
        let (code, name) = match result.unwrap() {
            AuthCommand::Pair { code, device_name } => (code, device_name),
            _ => panic!("expected Pair"),
        };
        assert_eq!(code, "A3B7K2");
        assert_eq!(name, "Pixel 8");
    }

    #[test]
    fn parse_authenticate_command() {
        let msg = IncomingMessage {
            id: "2".into(),
            msg_type: "auth".into(),
            command: Some("Authenticate".into()),
            payload: Some(serde_json::json!({
                "token": "abc123def456",
                "device_name": "Pixel 8",
                "os": "Android 15"
            })),
        };
        let result = parse_auth_command(&msg);
        assert!(result.is_ok());
        match result.unwrap() {
            AuthCommand::Authenticate {
                token,
                device_name,
                os,
            } => {
                assert_eq!(token, "abc123def456");
                assert_eq!(device_name, "Pixel 8");
                assert_eq!(os, "Android 15");
            }
            _ => panic!("expected Authenticate"),
        }
    }

    #[test]
    fn parse_shell_spawn_command() {
        let msg = IncomingMessage {
            id: "1".into(),
            msg_type: "command".into(),
            command: Some("ShellSpawn".into()),
            payload: Some(serde_json::json!({ "cols": 80, "rows": 24 })),
        };
        let result = parse_shell_command(&msg);
        assert!(result.is_ok());
        match result.unwrap() {
            ShellCommand::Spawn { cols, rows } => {
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            _ => panic!("expected Spawn"),
        }
    }
}
