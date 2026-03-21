//! Tool registry — definitions and dispatch for all agent tools.

// LSP-dependent tools stay in TUI.
pub mod diagnostics;
pub mod lsp;

// Re-export tool modules from caboose-core so existing `crate::tools::X` paths keep working.
pub use caboose_core::tools::executable;
pub use caboose_core::tools::fetch;
pub use caboose_core::tools::glob;
pub use caboose_core::tools::grep;
pub use caboose_core::tools::names;
pub use caboose_core::tools::patch;
pub use caboose_core::tools::read;
pub use caboose_core::tools::shell;
pub use caboose_core::tools::write;

// Re-export ToolRegistry and generate_skill_tool_def from core.
pub use caboose_core::tools::ToolRegistry;
pub use caboose_core::tools::generate_skill_tool_def;
