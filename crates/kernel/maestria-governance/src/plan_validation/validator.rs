use maestria_domain::{CorpusScope, FreshnessRequirement, SearchIntent, SearchPlan};

use crate::RetrievalSecurityPolicy;

use super::capabilities::{ScopeMode, SearchCapabilities};
use super::error::SearchPlanValidationError;

pub struct SearchPlanValidator;

impl SearchPlanValidator {
    pub fn validate(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
        policy: &RetrievalSecurityPolicy,
    ) -> Result<(), SearchPlanValidationError> {
        // Plan schema invariants are enforced at construction and decode, so
        // this validator only checks capability and policy conformance.
        let classified = SearchIntent::classify(plan.original_query());
        if classified != plan.intent() {
            return Err(SearchPlanValidationError::IntentMismatch {
                declared: plan.intent(),
                classified,
            });
        }
        if !capabilities.supports_intent(plan.intent()) {
            return Err(SearchPlanValidationError::UnsupportedIntent(plan.intent()));
        }
        if let Some(stage) = plan
            .stages()
            .iter()
            .find(|stage| !capabilities.supports_stage(stage))
        {
            return Err(SearchPlanValidationError::UnsupportedStage(*stage));
        }
        if let Some(modality) = plan
            .modalities()
            .values()
            .iter()
            .find(|modality| !capabilities.supports_modality(modality))
        {
            return Err(SearchPlanValidationError::UnsupportedModality(*modality));
        }
        if !capabilities.supports_snapshot(plan.corpus_snapshot()) {
            return Err(SearchPlanValidationError::SnapshotUnavailable(
                plan.corpus_snapshot(),
            ));
        }
        if !capabilities.supports_generation(plan.index_generation()) {
            return Err(SearchPlanValidationError::GenerationUnavailable(
                plan.index_generation(),
            ));
        }
        Self::validate_scope(plan, capabilities, policy)?;
        Self::validate_freshness(plan, capabilities)?;
        Self::validate_budgets(plan, capabilities)?;
        Self::validate_security(capabilities, policy)?;
        if plan.is_web_plan() && !capabilities.web_enabled() {
            return Err(SearchPlanValidationError::WebCapabilityMissing);
        }
        Ok(())
    }

    fn validate_scope(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
        policy: &RetrievalSecurityPolicy,
    ) -> Result<(), SearchPlanValidationError> {
        match &plan.scope() {
            CorpusScope::Global
                if policy.required_scope_id.is_some()
                    || !matches!(capabilities.scope(), ScopeMode::Global) =>
            {
                Err(SearchPlanValidationError::ScopeDenied)
            }
            CorpusScope::Global => Ok(()),
            CorpusScope::Restricted(scopes) => {
                if scopes.len() > capabilities.scope_id_limit() as usize {
                    return Err(SearchPlanValidationError::TooManyScopes {
                        requested: scopes.len(),
                        allowed: capabilities.scope_id_limit(),
                    });
                }
                if policy
                    .required_scope_id
                    .is_some_and(|required| scopes.len() != 1 || scopes.first() != Some(&required))
                {
                    return Err(SearchPlanValidationError::ScopeDenied);
                }
                if let ScopeMode::Restricted(allowed) = capabilities.scope()
                    && scopes.iter().any(|scope| !allowed.contains(scope))
                {
                    return Err(SearchPlanValidationError::ScopeDenied);
                }
                Ok(())
            }
        }
    }

    fn validate_freshness(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
    ) -> Result<(), SearchPlanValidationError> {
        match plan.freshness() {
            FreshnessRequirement::Any => Ok(()),
            FreshnessRequirement::Realtime if capabilities.supports_realtime() => Ok(()),
            FreshnessRequirement::MaximumAgeDays(days)
                if capabilities.max_age_days().is_some_and(|max| *days <= max) =>
            {
                Ok(())
            }
            _ => Err(SearchPlanValidationError::FreshnessUnsupported),
        }
    }

    fn validate_budgets(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
    ) -> Result<(), SearchPlanValidationError> {
        let budgets = [
            (
                "token",
                u64::from(plan.budgets().max_tokens()),
                u64::from(capabilities.max_tokens()),
            ),
            (
                "latency_ms",
                u64::from(plan.budgets().max_latency_ms()),
                u64::from(capabilities.max_latency_ms()),
            ),
            (
                "query",
                u64::from(plan.budgets().max_queries()),
                u64::from(capabilities.max_queries()),
            ),
            (
                "stage",
                u64::from(plan.budgets().max_stages()),
                u64::from(capabilities.max_stages()),
            ),
            (
                "web_request",
                u64::from(plan.budgets().max_web_requests()),
                u64::from(capabilities.max_web_requests()),
            ),
            (
                "bytes_read",
                plan.budgets().max_bytes_read(),
                capabilities.byte_limit(),
            ),
            (
                "concurrency",
                u64::from(plan.budgets().max_concurrency()),
                u64::from(capabilities.concurrency_limit()),
            ),
        ];
        budgets
            .into_iter()
            .find_map(|(budget, requested, allowed)| {
                (requested > allowed).then_some(SearchPlanValidationError::BudgetExceeded {
                    budget,
                    requested,
                    allowed,
                })
            })
            .map_or(Ok(()), Err)
    }

    fn validate_security(
        capabilities: &SearchCapabilities,
        policy: &RetrievalSecurityPolicy,
    ) -> Result<(), SearchPlanValidationError> {
        if !capabilities.acl_filtering() && policy.require_read_allowed {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing("ACL"));
        }
        if !capabilities.trust_filtering() && policy.require_trust_zone.is_some() {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing(
                "trust filtering",
            ));
        }
        if !capabilities.sensitivity_filtering() && policy.max_sensitivity.is_some() {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing(
                "sensitivity filtering",
            ));
        }
        if !capabilities.quarantine_filtering() {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing(
                "quarantine filtering",
            ));
        }
        Ok(())
    }
}
