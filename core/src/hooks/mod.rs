//! Hook systems — lifecycle hooks for agent events.

pub mod lifecycle;

pub use lifecycle::{HookAction, fire_hooks, fire_hooks_for_tool, parse_context, parse_must_keep};
