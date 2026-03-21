//! Skill system — loading, resolution, invocation, hints, and awareness.

pub mod awareness;
pub mod builtins;
pub mod creation;
pub mod expand;
pub mod handoff;
pub mod hints;
pub mod loader;
pub mod resolver;
pub mod types;

pub use types::*;
