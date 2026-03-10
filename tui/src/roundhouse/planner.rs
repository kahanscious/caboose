use crate::provider::{Message, Provider, StreamEvent, ToolDefinition};
use futures::StreamExt;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

/// Tools allowed during the planning phase (read-only)
const PLANNING_TOOLS: &[&str] = &[
    "read_file",
    "glob",
    "grep",
    "list_directory",
];

/// Maximum number of tool-use loops before forcing completion
const MAX_TOOL_ROUNDS: usize = 20;

/// Filter tool definitions to read-only subset for secondary planners
pub fn planning_tool_subset(all_tools: &[ToolDefinition]) -> Vec<ToolDefinition> {
    all_tools
        .iter()
        .filter(|t| PLANNING_TOOLS.contains(&t.name.as_str()))
        .cloned()
        .collect()
}

/// Updates from a planner task back to the main event loop.
#[derive(Debug)]
pub enum PlannerUpdate {
    StatusChanged {
        planner_index: usize,
        status: super::types::PlannerStatus,
    },
    #[allow(dead_code)]
    TokensUsed {
        planner_index: usize,
        input_tokens: u32,
        output_tokens: u32,
    },
    PlanComplete {
        planner_index: usize,
        result: Result<String, String>,
    },
}

/// Execute a single read-only tool for the planning phase.
/// Only handles read_file, glob, grep, list_directory.
async fn execute_read_only_tool(name: &str, input: &Value) -> Result<String, String> {
    let result = match name {
        "read_file" => crate::tools::read::execute(input)
            .await
            .map_err(|e| e.to_string())?,
        "glob" => crate::tools::glob::execute(input)
            .await
            .map_err(|e| e.to_string())?,
        "grep" => crate::tools::grep::execute(input)
            .await
            .map_err(|e| e.to_string())?,
        "list_directory" => crate::tools::read::execute_list_dir(input)
            .await
            .map_err(|e| e.to_string())?,
        _ => {
            return Err(format!("Tool '{name}' is not allowed in planning mode"));
        }
    };
    if result.is_error {
        Err(result.output)
    } else {
        Ok(result.output)
    }
}

/// Run a single planner — a mini agent loop that streams, handles tool calls,
/// and returns the final plan text.
///
/// This function is meant to be spawned as a tokio task. It sends status
/// updates via `update_tx` and returns the final plan as Ok(String) or
/// an error as Err(String).
pub async fn run_planner(
    provider: Box<dyn Provider>,
    prompt: String,
    tools: Vec<ToolDefinition>,
    timeout_secs: u64,
    update_tx: mpsc::UnboundedSender<PlannerUpdate>,
    planner_index: usize,
) -> Result<String, String> {
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        run_planner_inner(&*provider, &prompt, &tools, &update_tx, planner_index),
    )
    .await;

    match result {
        Ok(inner_result) => inner_result,
        Err(_) => {
            let _ = update_tx.send(PlannerUpdate::StatusChanged {
                planner_index,
                status: super::types::PlannerStatus::TimedOut,
            });
            Err("Planning timed out".to_string())
        }
    }
}

async fn run_planner_inner(
    provider: &dyn Provider,
    prompt: &str,
    tools: &[ToolDefinition],
    update_tx: &mpsc::UnboundedSender<PlannerUpdate>,
    planner_index: usize,
) -> Result<String, String> {
    let system_prompt = planning_system_prompt(prompt);

    // Build initial messages: system + user prompt
    let mut messages = vec![
        Message {
            role: "system".to_string(),
            content: serde_json::json!(system_prompt),
        },
        Message {
            role: "user".to_string(),
            content: serde_json::json!(prompt),
        },
    ];

    for _round in 0..MAX_TOOL_ROUNDS {
        // Update status to Streaming
        let _ = update_tx.send(PlannerUpdate::StatusChanged {
            planner_index,
            status: super::types::PlannerStatus::Streaming,
        });

        // Stream from provider
        let mut stream = provider.stream(&messages, tools);

        let mut text_content = String::new();
        let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, arguments)

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(StreamEvent::TextDelta(text)) => {
                    text_content.push_str(&text);
                }
                Ok(StreamEvent::ToolCall { id, name, arguments }) => {
                    tool_calls.push((id, name, arguments));
                }
                Ok(StreamEvent::Done {
                    input_tokens,
                    output_tokens,
                    ..
                }) => {
                    let _ = update_tx.send(PlannerUpdate::TokensUsed {
                        planner_index,
                        input_tokens: input_tokens.unwrap_or(0),
                        output_tokens: output_tokens.unwrap_or(0),
                    });
                }
                Ok(StreamEvent::Error(e)) => {
                    // Strip retry noise from error messages
                    let clean = if let Some(idx) = e.find(". Retrying") {
                        &e[..idx]
                    } else {
                        &e
                    };
                    return Err(format!("Stream error: {clean}"));
                }
                Ok(StreamEvent::ProviderError { message, .. }) => {
                    let clean = if let Some(idx) = message.find(". Retrying") {
                        &message[..idx]
                    } else {
                        &message
                    };
                    return Err(format!("Provider error: {clean}"));
                }
                Ok(StreamEvent::ThinkingDelta(_)) => {
                    // Ignore thinking tokens
                }
                Err(e) => {
                    return Err(format!("Stream error: {e}"));
                }
            }
        }

        // No tool calls → the text is our plan
        if tool_calls.is_empty() {
            return if text_content.is_empty() {
                Err("Planner returned empty response".to_string())
            } else {
                Ok(text_content)
            };
        }

        // Build assistant message with text + tool_use blocks
        let mut assistant_blocks = Vec::new();
        if !text_content.is_empty() {
            assistant_blocks.push(serde_json::json!({
                "type": "text",
                "text": text_content,
            }));
        }
        for (id, name, arguments) in &tool_calls {
            let input_val: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
            assistant_blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input_val,
            }));
        }
        messages.push(Message {
            role: "assistant".to_string(),
            content: serde_json::json!(assistant_blocks),
        });

        // Execute each tool call and build tool_result blocks
        let mut tool_result_blocks = Vec::new();
        for (id, name, arguments) in &tool_calls {
            let _ = update_tx.send(PlannerUpdate::StatusChanged {
                planner_index,
                status: super::types::PlannerStatus::UsingTool(name.clone()),
            });

            let input_val: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
            let (output, is_error) = match execute_read_only_tool(name, &input_val).await {
                Ok(output) => (output, false),
                Err(e) => (e, true),
            };

            tool_result_blocks.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": output,
                "is_error": is_error,
            }));
        }
        messages.push(Message {
            role: "user".to_string(),
            content: serde_json::json!(tool_result_blocks),
        });
    }

    // If we exhausted all rounds, return whatever text we have
    Err("Planner exceeded maximum tool rounds without producing a final plan".to_string())
}

/// Build the system prompt given to each secondary planner
pub fn planning_system_prompt(user_prompt: &str) -> String {
    format!(
        "You are participating in a multi-LLM planning session. Your task is to \
         create a detailed implementation plan for the following request. \
         You have access to read-only tools to explore the codebase. \
         \n\nProduce a structured plan with:\n\
         1. Architecture overview\n\
         2. Files to create/modify\n\
         3. Step-by-step implementation order\n\
         4. Testing strategy\n\
         5. Potential risks or concerns\n\
         \n\nUser request:\n{user_prompt}"
    )
}

/// Build the synthesis prompt given to the primary LLM
pub fn synthesis_system_prompt(
    user_prompt: &str,
    plans: &[(&str, &str)],
) -> String {
    let mut prompt = format!(
        "You are the primary planner in a multi-LLM planning session. \
         Multiple LLMs have independently created plans for the following request. \
         Review all plans, identify the best ideas from each, and produce \
         one unified implementation plan.\n\n\
         Original request:\n{user_prompt}\n\n"
    );
    for (i, (provider, plan)) in plans.iter().enumerate() {
        prompt.push_str(&format!(
            "--- Plan from {provider} (#{}) ---\n{plan}\n\n",
            i + 1
        ));
    }
    prompt.push_str(
        "Produce a single unified plan that combines the best approaches. \
         Note where plans disagreed and explain your choices."
    );
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolDefinition;

    fn mock_tools() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "write_file".into(),
                description: "Write a file".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "grep".into(),
                description: "Search files".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "run_command".into(),
                description: "Run shell command".into(),
                input_schema: serde_json::json!({}),
            },
        ]
    }

    #[test]
    fn test_planning_tool_subset_filters_write_tools() {
        let tools = mock_tools();
        let subset = planning_tool_subset(&tools);
        assert_eq!(subset.len(), 2);
        assert!(subset.iter().any(|t| t.name == "read_file"));
        assert!(subset.iter().any(|t| t.name == "grep"));
        assert!(!subset.iter().any(|t| t.name == "write_file"));
        assert!(!subset.iter().any(|t| t.name == "run_command"));
    }

    #[test]
    fn test_synthesis_prompt_includes_all_plans() {
        let plans = vec![
            ("openai", "Plan A content"),
            ("gemini", "Plan B content"),
        ];
        let prompt = synthesis_system_prompt("build a feature", &plans);
        assert!(prompt.contains("Plan A content"));
        assert!(prompt.contains("Plan B content"));
        assert!(prompt.contains("openai"));
        assert!(prompt.contains("gemini"));
        assert!(prompt.contains("build a feature"));
    }

    #[test]
    fn test_planner_update_variants() {
        // Verify the enum variants exist and are constructable
        let _status = PlannerUpdate::StatusChanged {
            planner_index: 0,
            status: super::super::types::PlannerStatus::Streaming,
        };
        let _tokens = PlannerUpdate::TokensUsed {
            planner_index: 0,
            input_tokens: 100,
            output_tokens: 50,
        };
        let _complete = PlannerUpdate::PlanComplete {
            planner_index: 0,
            result: Ok("plan text".to_string()),
        };
        let _failed = PlannerUpdate::PlanComplete {
            planner_index: 1,
            result: Err("timeout".to_string()),
        };
    }
}
