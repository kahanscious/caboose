//! MCP (Model Context Protocol) client — extend tools via external servers.

pub mod manager;
pub mod presets;

pub use manager::{McpConnectResult, McpManager, McpServer, ServerStatus};
pub use presets::find_preset;
