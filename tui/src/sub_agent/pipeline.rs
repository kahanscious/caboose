use crate::sub_agent::{TaskPipeline, TaskStage};

/// Extract the last top-level markdown list (≥2 items) from `text`.
/// Handles both `- item` and `1. item` styles. Returns None if not found.
#[allow(dead_code)]
pub fn extract_tasks(text: &str) -> Option<Vec<String>> {
    let mut current: Vec<String> = Vec::new();
    let mut last_valid: Option<Vec<String>> = None;

    for line in text.lines() {
        if let Some(item) = strip_list_marker(line) {
            current.push(item.trim().to_string());
        } else {
            // Indented lines (nested bullets, continuation) are silently skipped
            // so they don't interrupt the current top-level list.
            if line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            let t = line.trim();
            if !t.is_empty() && !current.is_empty() {
                if current.len() >= 2 {
                    last_valid = Some(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
        }
    }
    if current.len() >= 2 {
        last_valid = Some(current);
    }
    last_valid
}

/// Returns the list item text if `line` is a top-level list marker (not indented).
/// Indented lines (nested bullets) return None.
fn strip_list_marker(line: &str) -> Option<&str> {
    // Reject indented lines — they are nested bullets, not top-level tasks
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }
    let t = line.trim();
    // Unordered
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    // Ordered: "1. " or "1) "
    let digits = t.trim_start_matches(|c: char| c.is_ascii_digit());
    if let Some(rest) = digits.strip_prefix(". ") {
        return Some(rest);
    }
    if let Some(rest) = digits.strip_prefix(") ") {
        return Some(rest);
    }
    None
}

/// Try to parse a TaskPipeline from an LLM response.
/// Tries ```json ... ``` block first, then bare JSON object.
#[allow(dead_code)]
pub fn parse_pipeline_response(response: &str) -> Option<TaskPipeline> {
    // Try fenced JSON
    if let Some(start) = response.find("```json") {
        let rest = &response[start + 7..];
        if let Some(end) = rest.find("```")
            && let Ok(p) = serde_json::from_str::<TaskPipeline>(rest[..end].trim())
        {
            return Some(p);
        }
    }
    // Try bare JSON object
    if let Some(start) = response.find('{')
        && let Some(end) = response.rfind('}')
        && let Ok(p) = serde_json::from_str::<TaskPipeline>(&response[start..=end])
    {
        return Some(p);
    }
    None
}

/// Fallback: put all tasks in a single sequential stage.
#[allow(dead_code)]
pub fn single_stage_fallback(tasks: Vec<String>) -> TaskPipeline {
    TaskPipeline { stages: vec![TaskStage { tasks }] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sub_agent::TaskPipeline;

    #[test]
    fn extract_unordered_list() {
        let text = "I'll do these:\n- auth refactor\n- add tests\n- update readme";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks, vec!["auth refactor", "add tests", "update readme"]);
    }

    #[test]
    fn extract_ordered_list() {
        let text = "Plan:\n1. implement feature\n2. write tests\n3. update docs";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks[0], "implement feature");
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn extract_returns_last_list_when_multiple() {
        let text = "First:\n- a\n- b\n\nSecond:\n- c\n- d\n- e";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks, vec!["c", "d", "e"]);
    }

    #[test]
    fn extract_requires_min_two_items() {
        assert!(extract_tasks("- just one thing").is_none());
    }

    #[test]
    fn extract_none_for_prose() {
        assert!(extract_tasks("just prose, no list").is_none());
    }

    #[test]
    fn extract_ignores_nested_bullets() {
        // Nested bullets are NOT separate tasks — only top-level items count
        let text = "Tasks:\n- auth refactor\n  - sub-step A\n  - sub-step B\n- update readme";
        let tasks = extract_tasks(text).unwrap();
        assert_eq!(tasks.len(), 2, "nested bullets must not be treated as top-level tasks");
        assert_eq!(tasks[0], "auth refactor");
        assert_eq!(tasks[1], "update readme");
    }

    #[test]
    fn parse_pipeline_from_json() {
        let json = r#"{"stages":[{"tasks":["a","b"]},{"tasks":["c"]}]}"#;
        let p: TaskPipeline = serde_json::from_str(json).unwrap();
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[0].tasks, vec!["a", "b"]);
    }

    #[test]
    fn extract_pipeline_from_code_fence() {
        let response = "analysis:\n```json\n{\"stages\":[{\"tasks\":[\"task a\"]}]}\n```";
        let p = parse_pipeline_response(response).unwrap();
        assert_eq!(p.stages[0].tasks[0], "task a");
    }

    #[test]
    fn parse_pipeline_falls_back_to_none_on_bad_json() {
        assert!(parse_pipeline_response("no json here").is_none());
    }

    #[test]
    fn single_stage_fallback_wraps_all_tasks() {
        let tasks = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let p = single_stage_fallback(tasks);
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].tasks.len(), 3);
    }
}
