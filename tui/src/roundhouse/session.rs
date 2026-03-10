use crate::roundhouse::types::*;
use std::path::PathBuf;

/// Holds all state for a Roundhouse planning + execution session
pub struct RoundhouseSession {
    pub primary_provider: String,
    pub primary_model: String,
    pub secondaries: Vec<SecondaryPlanner>,
    pub phase: RoundhousePhase,
    pub prompt: Option<String>,
    pub synthesized_plan: Option<String>,
    pub plan_file: Option<PathBuf>,
    pub config: RoundhouseConfig,
    pub total_cost: f64,
}

impl RoundhouseSession {
    pub fn new(primary_provider: String, primary_model: String) -> Self {
        Self {
            primary_provider,
            primary_model,
            secondaries: Vec::new(),
            phase: RoundhousePhase::SelectingProviders,
            prompt: None,
            synthesized_plan: None,
            plan_file: None,
            config: RoundhouseConfig::default(),
            total_cost: 0.0,
        }
    }

    pub fn add_secondary(&mut self, provider: String, model: String) {
        self.secondaries.push(SecondaryPlanner {
            provider_name: provider,
            model_name: model,
            status: PlannerStatus::Pending,
            plan: None,
            token_count: 0,
            cost: 0.0,
        });
    }

    pub fn remove_secondary(&mut self, index: usize) {
        if index < self.secondaries.len() {
            self.secondaries.remove(index);
        }
    }

    pub fn all_planners_done(&self) -> bool {
        self.secondaries.iter().all(|s| {
            matches!(
                s.status,
                PlannerStatus::Done | PlannerStatus::Failed(_) | PlannerStatus::TimedOut
            )
        })
    }

    pub fn successful_plans(&self) -> Vec<(&str, &str)> {
        self.secondaries
            .iter()
            .filter_map(|s| {
                s.plan.as_deref().map(|p| (s.provider_name.as_str(), p))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_session() {
        let s = RoundhouseSession::new("anthropic".into(), "claude-sonnet".into());
        assert_eq!(s.phase, RoundhousePhase::SelectingProviders);
        assert!(s.secondaries.is_empty());
        assert!(s.synthesized_plan.is_none());
    }

    #[test]
    fn test_add_remove_secondary() {
        let mut s = RoundhouseSession::new("anthropic".into(), "claude-sonnet".into());
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());
        assert_eq!(s.secondaries.len(), 2);
        s.remove_secondary(0);
        assert_eq!(s.secondaries.len(), 1);
        assert_eq!(s.secondaries[0].provider_name, "gemini");
    }

    #[test]
    fn test_all_planners_done() {
        let mut s = RoundhouseSession::new("anthropic".into(), "claude-sonnet".into());
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());
        assert!(!s.all_planners_done());

        s.secondaries[0].status = PlannerStatus::Done;
        s.secondaries[1].status = PlannerStatus::Failed("timeout".into());
        assert!(s.all_planners_done());
    }

    #[test]
    fn test_successful_plans() {
        let mut s = RoundhouseSession::new("anthropic".into(), "claude-sonnet".into());
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());
        s.secondaries[0].plan = Some("Plan A".into());
        s.secondaries[0].status = PlannerStatus::Done;
        s.secondaries[1].status = PlannerStatus::Failed("err".into());

        let plans = s.successful_plans();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0], ("openai", "Plan A"));
    }
}
