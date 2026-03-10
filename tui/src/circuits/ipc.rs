#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use crate::circuits::types::{Circuit, CircuitStatus};

/// Messages from TUI to Daemon
#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonRequest {
    ListCircuits,
    StopCircuit { id: String },
    StopAll,
    Shutdown,
    Ping,
}

/// Messages from Daemon to TUI
#[derive(Debug, Serialize, Deserialize)]
pub enum DaemonResponse {
    CircuitList(Vec<Circuit>),
    CircuitStopped(String),
    AllStopped(usize),
    ShuttingDown,
    Pong,
    Error(String),
}

/// Send a message as line-delimited JSON over a TCP stream
pub async fn send_message<T: Serialize>(
    stream: &mut tokio::net::TcpStream,
    msg: &T,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let json = serde_json::to_string(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(json.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

/// Read a line-delimited JSON message from a TCP stream
pub async fn read_message<T: serde::de::DeserializeOwned>(
    reader: &mut tokio::io::BufReader<&mut tokio::net::TcpStream>,
) -> std::io::Result<T> {
    use tokio::io::AsyncBufReadExt;
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "connection closed",
        ));
    }
    serde_json::from_str(&line)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Read the daemon port from the lockfile
pub fn read_daemon_port() -> Option<u16> {
    let path = super::daemon::lockfile_path();
    let contents = std::fs::read_to_string(path).ok()?;
    let port_str = contents.split(':').nth(1)?;
    port_str.trim().parse().ok()
}
