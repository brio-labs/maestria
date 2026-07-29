use std::sync::Arc;

use maestria_domain::{
    EvidenceCoverage, SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace,
};

use crate::traits::CandidateRetriever;
use crate::types::{RetrievalError, RetrievalResult};

use super::{RetrievalEngine, applied_security_filters, security_policy_fingerprint};

impl RetrievalEngine {
    pub(super) fn validate_plan(&self, plan: &SearchPlan) -> RetrievalResult<()> {
        let expected_authorization = self
            .security_policy
            .authorization_context(&plan.scope)
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval authorization denied: {error:?}"))
            })?
            .policy_snapshot();
        if plan.authorization.as_ref() != Some(&expected_authorization) {
            return Err(RetrievalError::Internal(
                "search plan authorization is not trusted for this runtime".to_string(),
            ));
        }
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(plan.corpus_snapshot);
        match maestria_governance::SearchPlanValidator::validate(
            plan,
            &capabilities,
            &self.security_policy,
        ) {
            Ok(()) => Ok(()),
            Err(maestria_governance::SearchPlanValidationError::IntentMismatch {
                declared: maestria_domain::SearchIntent::FactualLocal,
                classified,
            }) if classified != maestria_domain::SearchIntent::ExactLookup => {
                let mut fallback_plan = plan.clone();
                fallback_plan.original_query = "fallback local text retrieval".to_string();
                maestria_governance::SearchPlanValidator::validate(
                    &fallback_plan,
                    &capabilities,
                    &self.security_policy,
                )
                .map_err(RetrievalError::SearchPlan)
            }
            Err(error) => Err(RetrievalError::SearchPlan(error)),
        }
    }

    pub(super) fn active_retrievers(&self, plan: &SearchPlan) -> Vec<Arc<dyn CandidateRetriever>> {
        let repository_specialized = self
            .repository_execution_policy
            .allows_specialized(&plan.original_query);
        let visual_enabled = self
            .visual_execution_policy
            .allows_visual(&plan.original_query);
        let sparse_enabled = self
            .learned_sparse_execution_policy
            .allows_sparse(&plan.original_query);
        self.retrievers
            .iter()
            .filter(|retriever| {
                let descriptor = retriever.descriptor();
                let descriptor_id = descriptor.id.to_ascii_lowercase();
                let is_code = descriptor.modality.eq_ignore_ascii_case("code")
                    || descriptor.modality.eq_ignore_ascii_case("rust")
                    || descriptor_id.contains("code_intel");
                crate::visual_benchmark::visual_lane_is_eligible(&descriptor, visual_enabled)
                    && crate::learned_sparse_policy::sparse_lane_is_eligible(
                        &descriptor,
                        sparse_enabled,
                    )
                    && (repository_specialized || !is_code)
            })
            .cloned()
            .collect()
    }

    pub(super) fn learned_sparse_shadow_retrievers(
        &self,
        plan: &SearchPlan,
    ) -> Vec<Arc<dyn CandidateRetriever>> {
        if !self
            .learned_sparse_execution_policy
            .should_shadow(&plan.original_query)
        {
            return Vec::new();
        }
        self.retrievers
            .iter()
            .filter(|retriever| {
                crate::learned_sparse_policy::is_sparse_descriptor(&retriever.descriptor())
            })
            .cloned()
            .collect()
    }

    pub(super) fn prompt_injection_outcome(&self, plan: &SearchPlan) -> SearchOutcome {
        let retriever_ids = self
            .retrievers
            .iter()
            .map(|retriever| retriever.descriptor().id.clone())
            .collect();
        let policy_fingerprint = match plan.authorization.as_ref() {
            Some(policy) => policy.canonical_fingerprint(),
            None => security_policy_fingerprint(&self.security_policy),
        };
        let trace = SearchTrace::from_plan(
            plan,
            retriever_ids,
            &[],
            applied_security_filters(plan, &self.security_policy),
            self.fusion.as_ref().map(|_| "configured".to_string()),
            Vec::new(),
            SearchStopReason::PolicyDenied,
        )
        .with_policy_fingerprint(policy_fingerprint);
        SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: plan.fingerprint.clone(),
            index_generation: plan.index_generation,
            status: SearchStatus::QuarantinedForReview,
            evidence: Vec::new(),
            coverage: EvidenceCoverage {
                required_claims: vec![],
                required_subquestions: vec![],
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
                candidate_coverage_keys: vec![],
                percent_covered: 0,
                gaps_identified: vec![],
            },
            conflicts: Vec::new(),
        }
    }
}
