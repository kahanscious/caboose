//! /suggest — evidence-based codebase improvement suggestions.

pub mod config;
pub mod digest;
pub mod parsers;
pub mod priority;
pub mod scanner;

use priority::{Finding, PriorityWeights};

/// Run the full suggest pipeline: detect scans, run them, parse, digest.
/// Returns the formatted digest + priority prompt ready to inject into conversation.
pub async fn run_suggest(suggest_config: Option<&caboose_core::config::schema::SuggestConfig>) -> String {
    // 1. Resolve scan commands
    let scans = config::resolve_scans(suggest_config);

    // 2. Run all scans in parallel
    let scan_results = scanner::run_scans(&scans).await;

    // 3. Collect all findings + error findings
    let priority_config = suggest_config.and_then(|c| c.priorities.as_ref());
    let weights = PriorityWeights::from_config(priority_config);
    let mut findings: Vec<Finding> = Vec::new();

    for result in &scan_results {
        findings.extend(result.findings.clone());
        if let Some(ref err) = result.error {
            findings.push(Finding {
                category: priority::Category::Custom("scan-error".to_string()),
                severity: priority::Severity::Info,
                summary: err.clone(),
                location: None,
                count: 1,
            });
        }
    }

    // 4. Dedup + sort
    priority::dedup_findings(&mut findings);
    priority::sort_findings(&mut findings, &weights);

    // 5. Build digest + append priority prompt
    let digest = digest::build_digest(&findings, &weights);
    format!("{digest}\n{}", digest::PRIORITY_PROMPT)
}
