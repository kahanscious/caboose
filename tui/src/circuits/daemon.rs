use anyhow::Result;
use std::path::PathBuf;

/// Lockfile location for daemon discovery
pub fn lockfile_path() -> PathBuf {
    let dir = data_dir();
    dir.join("daemon.lock")
}

/// Path to the circuits SQLite database
pub fn circuits_db_path() -> PathBuf {
    let dir = data_dir();
    dir.join("circuits.db")
}

/// Common data directory for caboose
fn data_dir() -> PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".caboose"));
    let dir = base.join("caboose");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Check if a daemon is already running
pub fn is_daemon_running() -> bool {
    let path = lockfile_path();
    if !path.exists() {
        return false;
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            // Format: "PID:PORT"
            if let Some(pid_str) = contents.split(':').next()
                && let Ok(_pid) = pid_str.parse::<u32>()
            {
                // TODO: check if PID is alive (platform-specific)
                return true;
            }
            false
        }
        Err(_) => false,
    }
}

/// Write the daemon lockfile
pub fn write_lockfile(port: u16) -> Result<()> {
    let path = lockfile_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pid = std::process::id();
    std::fs::write(&path, format!("{pid}:{port}"))?;
    Ok(())
}

/// Remove the daemon lockfile
pub fn remove_lockfile() -> Result<()> {
    let path = lockfile_path();
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Run the daemon main loop: TCP listener + circuit scheduler
pub async fn run_daemon() -> Result<()> {
    use crate::circuits::storage;
    use rusqlite::Connection;
    use tokio::net::TcpListener;

    let db_path = circuits_db_path();
    let conn = Connection::open(&db_path)?;
    storage::init_circuits_table(&conn)?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    write_lockfile(port)?;

    eprintln!("caboose daemon started on port {port}");

    let circuits = storage::list_circuits(&conn, Some(&crate::circuits::CircuitKind::Persistent))?;
    eprintln!("loaded {} persistent circuit(s)", circuits.len());

    // Wrap conn in Arc<Mutex> for sharing across tasks
    let conn = std::sync::Arc::new(std::sync::Mutex::new(conn));
    let shutdown = tokio::sync::watch::channel(false);
    let (shutdown_tx, shutdown_rx) = (shutdown.0, shutdown.1);

    loop {
        let mut rx = shutdown_rx.clone();
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((mut stream, addr)) => {
                        eprintln!("client connected: {addr}");
                        let db = conn.clone();
                        let tx = shutdown_tx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_client(&mut stream, &db, &tx).await {
                                eprintln!("client error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("accept error: {e}");
                    }
                }
            }
            _ = rx.changed() => {
                if *rx.borrow() {
                    eprintln!("daemon shutting down via request...");
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("daemon shutting down via ctrl-c...");
                break;
            }
        }
    }

    remove_lockfile()?;
    eprintln!("daemon stopped");
    Ok(())
}

/// Handle a single client connection
async fn handle_client(
    stream: &mut tokio::net::TcpStream,
    db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    shutdown_tx: &tokio::sync::watch::Sender<bool>,
) -> Result<()> {
    use crate::circuits::ipc::*;
    use crate::circuits::storage;

    let mut reader = tokio::io::BufReader::new(stream);
    let request: DaemonRequest = read_message(&mut reader).await?;
    let stream = reader.into_inner();

    let response = match request {
        DaemonRequest::Ping => DaemonResponse::Pong,

        DaemonRequest::ListCircuits => {
            let conn = db.lock().unwrap();
            match storage::list_circuits(&conn, None) {
                Ok(circuits) => DaemonResponse::CircuitList(circuits),
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }

        DaemonRequest::StopCircuit { id } => {
            let conn = db.lock().unwrap();
            match storage::update_circuit_status(
                &conn,
                &id,
                &crate::circuits::CircuitStatus::Paused,
            ) {
                Ok(()) => DaemonResponse::CircuitStopped(id),
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }

        DaemonRequest::StopAll => {
            let conn = db.lock().unwrap();
            match storage::list_circuits(&conn, None) {
                Ok(circuits) => {
                    let mut count = 0;
                    for c in &circuits {
                        if storage::update_circuit_status(
                            &conn,
                            &c.id,
                            &crate::circuits::CircuitStatus::Paused,
                        )
                        .is_ok()
                        {
                            count += 1;
                        }
                    }
                    DaemonResponse::AllStopped(count)
                }
                Err(e) => DaemonResponse::Error(e.to_string()),
            }
        }

        DaemonRequest::Shutdown => {
            let _ = shutdown_tx.send(true);
            DaemonResponse::ShuttingDown
        }
    };

    send_message(stream, &response).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockfile_path_is_in_data_dir() {
        let path = lockfile_path();
        assert!(path.to_string_lossy().contains("caboose"));
        assert!(path.file_name().unwrap() == "daemon.lock");
    }

    #[test]
    fn test_circuits_db_path() {
        let path = circuits_db_path();
        assert!(path.to_string_lossy().contains("caboose"));
        assert!(path.file_name().unwrap() == "circuits.db");
    }
}
