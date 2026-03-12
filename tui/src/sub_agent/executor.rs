use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::agent::{AgentEvent, AgentLoop, AgentState, permission::PermissionMode};
use crate::config::Config;
use super::{SubAgentEvent, SubAgentStreamLine, StreamLineKind};

#[allow(dead_code)]
pub struct SubAgentInput {
    pub id: Uuid,
    pub task: String,
    pub worktree_path: PathBuf,
    pub system_prompt: String,
}

/// Run a subagent task headlessly. Returns total cost in USD, or an error message.
#[allow(dead_code)]
pub async fn run_subagent(
    input: SubAgentInput,
    provider: Arc<dyn crate::provider::Provider + Send + Sync>,
    config: Config,
    tx: tokio::sync::mpsc::UnboundedSender<SubAgentEvent>,
) -> Result<f64, String> {
    let mut agent = AgentLoop::new(input.system_prompt, PermissionMode::Chug);
    agent.primary_root = input.worktree_path.clone();

    // Wire tools config
    if let Some(ref tools_cfg) = config.tools {
        if let Some(ref allow) = tools_cfg.allow_commands {
            agent.allow_list = allow.clone();
        }
        if let Some(ref deny) = tools_cfg.deny_commands {
            agent.deny_list = deny.clone();
        }
        if let Some(ref secrets) = tools_cfg.additional_secret_names {
            agent.additional_secrets = secrets.clone();
        }
    }

    let cli_tools_ref = config.tools.as_ref().and_then(|t| t.registry.as_ref());
    let exec_tools_ref = config.tools.as_ref().and_then(|t| t.executable.as_ref());
    let worktree_scm = crate::scm::detection::detect_provider(&input.worktree_path);
    let tool_registry = crate::tools::ToolRegistry::new(cli_tools_ref, exec_tools_ref, &worktree_scm);
    let mcp_config = config.mcp.clone().unwrap_or_default();
    let mut mcp_manager = crate::mcp::McpManager::from_config(&mcp_config);
    mcp_manager.connect_all().await;
    let mut tool_defs = tool_registry.definitions().to_vec();
    tool_defs.extend(mcp_manager.tool_definitions());

    // Inject working directory into the task message
    let task_message = format!(
        "Working directory: `{}`\n\nTask: {}",
        input.worktree_path.display(),
        input.task
    );

    // Start the agent with the task
    agent.send_message(task_message, provider.as_ref(), &tool_defs);

    let mut total_cost = 0.0f64;
    // Sonnet 4.5 pricing: $3.00/M input, $15.00/M output
    const INPUT_PRICE_PER_TOKEN: f64 = 3.0 / 1_000_000.0;
    const OUTPUT_PRICE_PER_TOKEN: f64 = 15.0 / 1_000_000.0;

    // Poll loop — drive the agent to completion
    loop {
        tokio::time::sleep(Duration::from_millis(10)).await;

        let events = agent.poll();
        for event in &events {
            match event {
                AgentEvent::TextDelta(text) => {
                    let _ = tx.send(SubAgentEvent::StreamLine {
                        id: input.id,
                        line: SubAgentStreamLine {
                            kind: StreamLineKind::Text,
                            text: text.clone(),
                        },
                    });
                }
                AgentEvent::ToolCall { name, arguments, .. } => {
                    let text: String = format!("{name}  {arguments}").chars().take(120).collect();
                    let _ = tx.send(SubAgentEvent::StreamLine {
                        id: input.id,
                        line: SubAgentStreamLine {
                            kind: StreamLineKind::ToolCall,
                            text,
                        },
                    });
                }
                AgentEvent::TurnComplete { input_tokens, output_tokens } => {
                    let turn_cost = (*input_tokens as f64 * INPUT_PRICE_PER_TOKEN)
                        + (*output_tokens as f64 * OUTPUT_PRICE_PER_TOKEN);
                    total_cost += turn_cost;
                    let _ = tx.send(SubAgentEvent::CostUpdate {
                        id: input.id,
                        cost_usd: total_cost,
                    });
                }
                AgentEvent::Error(e) => {
                    let _ = tx.send(SubAgentEvent::StreamLine {
                        id: input.id,
                        line: SubAgentStreamLine {
                            kind: StreamLineKind::Error,
                            text: e.clone(),
                        },
                    });
                    return Err(e.clone());
                }
                AgentEvent::ProviderError { message, .. } => {
                    let _ = tx.send(SubAgentEvent::StreamLine {
                        id: input.id,
                        line: SubAgentStreamLine {
                            kind: StreamLineKind::Error,
                            text: message.clone(),
                        },
                    });
                    return Err(message.clone());
                }
                AgentEvent::CompactionComplete => {
                    // Resume stream after compaction if needed
                    if !agent.stashed_tool_defs.is_empty() {
                        let defs: Vec<_> = std::mem::take(&mut agent.stashed_tool_defs);
                        agent.start_stream(provider.as_ref(), &defs);
                    }
                }
            }
        }

        match agent.state {
            AgentState::Idle => break,
            AgentState::Streaming | AgentState::Compacting => {
                // Keep polling
            }
            AgentState::PendingApproval { .. } => {
                // Defensive: Chug mode shouldn't reach here, but auto-approve if it does
                while agent.approve_current() {}
            }
            AgentState::ExecutingTools => {
                let results = agent
                    .execute_pending_tools(
                        &mut mcp_manager,
                        config.services.as_ref(),
                        cli_tools_ref,
                        config
                            .tools
                            .as_ref()
                            .and_then(|t| t.deny_commands.as_deref())
                            .unwrap_or(&[]),
                        config.hooks.as_ref(),
                        exec_tools_ref,
                    )
                    .await;
                // Emit tool results as stream lines
                for r in &results {
                    let kind = if r.is_error {
                        StreamLineKind::Error
                    } else {
                        StreamLineKind::ToolResult
                    };
                    let _ = tx.send(SubAgentEvent::StreamLine {
                        id: input.id,
                        line: SubAgentStreamLine {
                            kind,
                            text: r.output.clone(),
                        },
                    });
                }
                agent.continue_after_tools(provider.as_ref(), &tool_defs);
            }
        }
    }

    Ok(total_cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_input_fields() {
        let id = uuid::Uuid::new_v4();
        let input = SubAgentInput {
            id,
            task: "implement auth".to_string(),
            worktree_path: std::path::PathBuf::from(".worktrees/agent-auth"),
            system_prompt: "You are a helpful coding agent.".to_string(),
        };
        assert_eq!(input.task, "implement auth");
        assert_eq!(input.id, id);
    }
}
