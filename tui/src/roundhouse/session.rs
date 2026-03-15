use crate::roundhouse::types::*;
use std::path::PathBuf;

/// Holds all state for a Roundhouse planning + execution session
pub struct RoundhouseSession {
    pub primary_provider: String,
    pub primary_model: String,
    pub primary_status: PlannerStatus,
    pub primary_status_tick: u64,
    pub primary_plan: Option<String>,
    pub primary_streaming_text: String,
    pub synthesis_streaming_text: String,
    pub secondaries: Vec<SecondaryPlanner>,
    pub phase: RoundhousePhase,
    pub prompt: Option<String>,
    pub synthesized_plan: Option<String>,
    pub plan_file: Option<PathBuf>,
    pub config: RoundhouseConfig,
    pub total_cost: f64,
    #[allow(dead_code)]
    pub primary_critique: Option<String>,
    #[allow(dead_code)]
    pub primary_critique_status: PlannerStatus,
    #[allow(dead_code)]
    pub primary_critique_status_tick: u64,
    #[allow(dead_code)]
    pub primary_critique_streaming_text: String,
    #[allow(dead_code)]
    pub critique_enabled: bool,
}

impl RoundhouseSession {
    pub fn new(
        primary_provider: String,
        primary_model: String,
        critique_enabled: bool,
        config: RoundhouseConfig,
    ) -> Self {
        Self {
            primary_provider,
            primary_model,
            primary_status: PlannerStatus::Pending,
            primary_status_tick: 0,
            primary_plan: None,
            primary_streaming_text: String::new(),
            synthesis_streaming_text: String::new(),
            secondaries: Vec::new(),
            phase: RoundhousePhase::SelectingProviders,
            prompt: None,
            synthesized_plan: None,
            plan_file: None,
            config,
            total_cost: 0.0,
            primary_critique: None,
            primary_critique_status: PlannerStatus::Pending,
            primary_critique_status_tick: 0,
            primary_critique_streaming_text: String::new(),
            critique_enabled,
        }
    }

    pub fn add_secondary(&mut self, provider: String, model: String) {
        self.secondaries.push(SecondaryPlanner {
            provider_name: provider,
            model_name: model,
            status: PlannerStatus::Pending,
            status_tick: 0,
            plan: None,
            token_count: 0,
            cost: 0.0,
            critique: None,
            critique_status: PlannerStatus::Pending,
            critique_status_tick: 0,
            critique_streaming_text: String::new(),
        });
    }

    #[allow(dead_code)]
    pub fn remove_secondary(&mut self, index: usize) {
        if index < self.secondaries.len() {
            self.secondaries.remove(index);
        }
    }

    pub fn all_planners_done(&self) -> bool {
        let primary_done = matches!(
            self.primary_status,
            PlannerStatus::Done | PlannerStatus::Failed(_) | PlannerStatus::TimedOut
        );
        let secondaries_done = self.secondaries.iter().all(|s| {
            matches!(
                s.status,
                PlannerStatus::Done | PlannerStatus::Failed(_) | PlannerStatus::TimedOut
            )
        });
        primary_done && secondaries_done
    }

    #[allow(dead_code)]
    pub fn all_critiques_done(&self) -> bool {
        let primary_done = matches!(
            self.primary_critique_status,
            PlannerStatus::Done | PlannerStatus::Failed(_) | PlannerStatus::TimedOut
        );
        let secondaries_done = self.secondaries.iter().all(|s| {
            matches!(
                s.critique_status,
                PlannerStatus::Done | PlannerStatus::Failed(_) | PlannerStatus::TimedOut
            )
        });
        primary_done && secondaries_done
    }

    #[allow(dead_code)]
    pub fn successful_critiques(&self) -> Vec<(&str, &str)> {
        let mut critiques = Vec::new();
        if let Some(ref critique) = self.primary_critique {
            critiques.push((self.primary_provider.as_str(), critique.as_str()));
        }
        for s in &self.secondaries {
            if let Some(ref c) = s.critique {
                critiques.push((s.provider_name.as_str(), c.as_str()));
            }
        }
        critiques
    }

    pub fn successful_plans(&self) -> Vec<(&str, &str)> {
        let mut plans = Vec::new();
        if let Some(ref plan) = self.primary_plan {
            plans.push((self.primary_provider.as_str(), plan.as_str()));
        }
        for s in &self.secondaries {
            if let Some(ref p) = s.plan {
                plans.push((s.provider_name.as_str(), p.as_str()));
            }
        }
        plans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_session() {
        let s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        assert_eq!(s.phase, RoundhousePhase::SelectingProviders);
        assert!(s.secondaries.is_empty());
        assert!(s.synthesized_plan.is_none());
    }

    #[test]
    fn test_add_remove_secondary() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());
        assert_eq!(s.secondaries.len(), 2);
        s.remove_secondary(0);
        assert_eq!(s.secondaries.len(), 1);
        assert_eq!(s.secondaries[0].provider_name, "gemini");
    }

    #[test]
    fn test_all_planners_done() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());
        assert!(!s.all_planners_done());

        s.primary_status = PlannerStatus::Done;
        s.secondaries[0].status = PlannerStatus::Done;
        s.secondaries[1].status = PlannerStatus::Failed("timeout".into());
        assert!(s.all_planners_done());
    }

    #[test]
    fn test_all_planners_done_requires_primary() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());
        // Secondaries done but primary still pending
        s.secondaries[0].status = PlannerStatus::Done;
        assert!(!s.all_planners_done());
        // Now mark primary done
        s.primary_status = PlannerStatus::Done;
        assert!(s.all_planners_done());
    }

    #[test]
    fn test_successful_plans() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());
        s.primary_plan = Some("Primary Plan".into());
        s.primary_status = PlannerStatus::Done;
        s.secondaries[0].plan = Some("Plan A".into());
        s.secondaries[0].status = PlannerStatus::Done;
        s.secondaries[1].status = PlannerStatus::Failed("err".into());

        let plans = s.successful_plans();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0], ("anthropic", "Primary Plan"));
        assert_eq!(plans[1], ("openai", "Plan A"));
    }

    #[test]
    fn test_new_session_critique_fields() {
        let s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        assert!(s.critique_enabled);
        assert!(s.primary_critique.is_none());
        assert_eq!(s.primary_critique_status, PlannerStatus::Pending);
        assert_eq!(s.primary_critique_status_tick, 0);
        assert!(s.primary_critique_streaming_text.is_empty());

        let s2 = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            false,
            RoundhouseConfig::default(),
        );
        assert!(!s2.critique_enabled);
    }

    #[test]
    fn test_all_critiques_done() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());
        assert!(!s.all_critiques_done());

        s.primary_critique_status = PlannerStatus::Done;
        assert!(!s.all_critiques_done());

        s.secondaries[0].critique_status = PlannerStatus::Done;
        assert!(s.all_critiques_done());
    }

    #[test]
    fn test_all_critiques_done_with_failure() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());

        s.primary_critique_status = PlannerStatus::Done;
        s.secondaries[0].critique_status = PlannerStatus::Failed("error".into());
        assert!(s.all_critiques_done());

        // TimedOut also counts as done
        s.secondaries[0].critique_status = PlannerStatus::TimedOut;
        assert!(s.all_critiques_done());
    }

    #[test]
    fn test_successful_critiques() {
        let mut s = RoundhouseSession::new(
            "anthropic".into(),
            "claude-sonnet".into(),
            true,
            RoundhouseConfig::default(),
        );
        s.add_secondary("openai".into(), "gpt-4o".into());
        s.add_secondary("gemini".into(), "gemini-2.5-pro".into());

        s.primary_critique = Some("Primary critique".into());
        s.primary_critique_status = PlannerStatus::Done;
        s.secondaries[0].critique = Some("OpenAI critique".into());
        s.secondaries[0].critique_status = PlannerStatus::Done;
        s.secondaries[1].critique_status = PlannerStatus::Failed("err".into());

        let critiques = s.successful_critiques();
        assert_eq!(critiques.len(), 2);
        assert_eq!(critiques[0], ("anthropic", "Primary critique"));
        assert_eq!(critiques[1], ("openai", "OpenAI critique"));
    }
}
