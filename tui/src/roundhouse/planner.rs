use crate::provider::ToolDefinition;

/// Tools allowed during the planning phase (read-only)
const PLANNING_TOOLS: &[&str] = &[
    "read_file",
    "glob",
    "grep",
    "list_directory",
];

/// Filter tool definitions to read-only subset for secondary planners
pub fn planning_tool_subset(all_tools: &[ToolDefinition]) -> Vec<ToolDefinition> {
    all_tools
        .iter()
        .filter(|t| PLANNING_TOOLS.contains(&t.name.as_str()))
        .cloned()
        .collect()
}

/// Events emitted by a secondary planner back to the Roundhouse coordinator
#[derive(Debug)]
pub enum PlannerEvent {
    StatusChanged {
        index: usize,
        status: super::types::PlannerStatus,
    },
    PlanComplete {
        index: usize,
        plan: String,
        tokens_used: u64,
    },
    PlanFailed {
        index: usize,
        error: String,
    },
    CostUpdate {
        index: usize,
        cost: f64,
    },
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
}
