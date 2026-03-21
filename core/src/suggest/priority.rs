//! Finding types, priority weights, sorting, and deduplication.

/// Finding category from a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Test,
    Lint,
    Todo,
    Churn,
    Custom(String),
}

/// Severity within a category.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A single finding from a scan.
#[derive(Debug, Clone)]
pub struct Finding {
    pub category: Category,
    pub severity: Severity,
    pub summary: String,
    pub location: Option<String>,
    pub count: usize,
}

/// Priority weights (lower = higher priority).
pub struct PriorityWeights {
    pub test_failure: u8,
    pub lint_error: u8,
    pub lint_warning: u8,
    pub todo: u8,
    pub recent_churn: u8,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            test_failure: 1,
            lint_error: 2,
            lint_warning: 3,
            todo: 4,
            recent_churn: 5,
        }
    }
}

impl PriorityWeights {
    /// Build from optional config, falling back to defaults.
    pub fn from_config(config: Option<&crate::config::schema::PriorityConfig>) -> Self {
        let defaults = Self::default();
        match config {
            None => defaults,
            Some(c) => Self {
                test_failure: c.test_failure.unwrap_or(defaults.test_failure),
                lint_error: c.lint_error.unwrap_or(defaults.lint_error),
                lint_warning: c.lint_warning.unwrap_or(defaults.lint_warning),
                todo: c.todo.unwrap_or(defaults.todo),
                recent_churn: c.recent_churn.unwrap_or(defaults.recent_churn),
            },
        }
    }

    /// Get the priority weight for a finding.
    pub fn weight(&self, finding: &Finding) -> u8 {
        match (&finding.category, &finding.severity) {
            (Category::Test, _) => self.test_failure,
            (Category::Lint, Severity::Error) => self.lint_error,
            (Category::Lint, _) => self.lint_warning,
            (Category::Todo, _) => self.todo,
            (Category::Churn, _) => self.recent_churn,
            (Category::Custom(_), _) => self.todo,
        }
    }
}

/// Sort findings by priority weight (lowest first = highest priority).
pub fn sort_findings(findings: &mut [Finding], weights: &PriorityWeights) {
    findings.sort_by_key(|f| (weights.weight(f), f.severity.clone()));
}

/// Deduplicate findings with the same summary — merge into one with count incremented.
pub fn dedup_findings(findings: &mut Vec<Finding>) {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut deduped: Vec<Finding> = Vec::new();
    for f in findings.drain(..) {
        if let Some(&idx) = seen.get(&f.summary) {
            deduped[idx].count += f.count;
        } else {
            seen.insert(f.summary.clone(), deduped.len());
            deduped.push(f);
        }
    }
    *findings = deduped;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(cat: Category, sev: Severity, summary: &str) -> Finding {
        Finding {
            category: cat,
            severity: sev,
            summary: summary.to_string(),
            location: None,
            count: 1,
        }
    }

    #[test]
    fn sort_by_priority_weight() {
        let weights = PriorityWeights::default();
        let mut findings = vec![
            finding(Category::Todo, Severity::Info, "TODO: fix this"),
            finding(Category::Test, Severity::Error, "test_foo failed"),
            finding(Category::Lint, Severity::Warning, "unused import"),
        ];
        sort_findings(&mut findings, &weights);
        assert_eq!(findings[0].category, Category::Test);
        assert_eq!(findings[1].category, Category::Lint);
        assert_eq!(findings[2].category, Category::Todo);
    }

    #[test]
    fn sort_lint_errors_before_warnings() {
        let weights = PriorityWeights::default();
        let mut findings = vec![
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Error, "type mismatch"),
        ];
        sort_findings(&mut findings, &weights);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[1].severity, Severity::Warning);
    }

    #[test]
    fn custom_priority_weights() {
        let weights = PriorityWeights {
            todo: 1,
            test_failure: 5,
            ..PriorityWeights::default()
        };
        let mut findings = vec![
            finding(Category::Test, Severity::Error, "test_foo failed"),
            finding(Category::Todo, Severity::Info, "TODO: fix this"),
        ];
        sort_findings(&mut findings, &weights);
        assert_eq!(findings[0].category, Category::Todo);
        assert_eq!(findings[1].category, Category::Test);
    }

    #[test]
    fn dedup_merges_identical_summaries() {
        let mut findings = vec![
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Warning, "unused import"),
        ];
        dedup_findings(&mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].count, 3);
    }

    #[test]
    fn dedup_preserves_different_summaries() {
        let mut findings = vec![
            finding(Category::Lint, Severity::Warning, "unused import"),
            finding(Category::Lint, Severity::Warning, "needless borrow"),
        ];
        dedup_findings(&mut findings);
        assert_eq!(findings.len(), 2);
    }
}
