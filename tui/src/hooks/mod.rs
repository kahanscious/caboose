//! Hook systems — post-tool enrichment hooks and lifecycle hooks.

pub mod diagnostics;
pub mod lifecycle;

pub use lifecycle::{HookAction, fire_hooks, fire_hooks_for_tool, parse_context, parse_must_keep};

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;

use crate::agent::tools::ToolResult;
use crate::lsp::LspManager;

/// Context available to hooks during execution.
pub struct HookContext<'a> {
    pub lsp_manager: Option<&'a mut LspManager>,
    // Future expansion: formatter, linter, test runner, etc.
}

/// A hook that runs after tool execution.
pub trait PostToolHook: Send + Sync {
    /// Whether this hook should run for the given tool result.
    fn applies(&self, result: &ToolResult) -> bool;

    /// Enrich the tool result (append to output, set metadata, etc.).
    fn run<'a>(
        &'a self,
        result: &'a mut ToolResult,
        ctx: &'a mut HookContext<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

/// Pipeline of post-tool hooks. Runs applicable hooks in order.
pub struct PostToolHooks {
    hooks: Vec<Box<dyn PostToolHook>>,
}

impl PostToolHooks {
    /// Create a new pipeline with the default hooks.
    pub fn new() -> Self {
        Self {
            hooks: vec![Box::new(diagnostics::LspDiagnosticsHook)],
        }
    }

    /// Run all applicable hooks on a tool result.
    pub async fn run(&self, result: &mut ToolResult, ctx: &mut HookContext<'_>) {
        for hook in &self.hooks {
            if hook.applies(result)
                && let Err(e) = hook.run(result, ctx).await
            {
                // Hook errors are non-fatal — don't fail the tool
                eprintln!("Post-tool hook error: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_creates_with_default_hooks() {
        let hooks = PostToolHooks::new();
        assert!(!hooks.hooks.is_empty());
    }
}
