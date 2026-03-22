use anyhow::Result;
use caboose_core::config::Config;
use caboose_core::events::CoreHandle;
use caboose_server::{ServerConfig, start_server};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let config = Config::load()?;
    let (core_handle, _cmd_rx) = CoreHandle::new();
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("caboose");
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("devices.db");

    let server = start_server(
        ServerConfig { port: 9090, bind: "0.0.0.0".into(), config, db_path },
        core_handle,
    ).await?;
    println!("caboose-server running on {}", server.local_addr);
    tokio::signal::ctrl_c().await?;
    server.shutdown();
    Ok(())
}
