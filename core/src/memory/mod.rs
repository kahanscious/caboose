//! Cross-session memory — persistent knowledge store.
//!
//! File-based MEMORY.md (human-editable, always-loaded) + SQLite FTS5 index
//! + observation capture for end-of-session auto-extraction.

pub mod extraction;
pub mod observations;
pub mod search;
pub mod store;

pub use store::MemoryStore;
