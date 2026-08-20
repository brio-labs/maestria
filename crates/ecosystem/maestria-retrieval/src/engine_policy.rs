use std::sync::Arc;

use maestria_domain::{
    EvidenceCoverage, EvidenceCoverageDto, SearchOutcome, SearchPlan, SearchStatus,
    SearchStopReason, SearchTrace,
};

use crate::traits::CandidateRetriever;
use crate::types::{RetrievalError, RetrievalResult};

use super::{RetrievalEngine, applied_security_filters};

impl RetrievalEngine {
    pub(super) fn validate_plan(&self, plan: &SearchPlan) -> RetrievalResult<()> {
        let expected_authorization = self
            .security_policy
            .authorization_context(plan.scope())
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval authorization denied: {error:?}"))
            })?
            .policy_snapshot()
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval policy snapshot invalid: {error}"))
            })?;
        if plan.authorization() != &expected_authorization {
            return Err(RetrievalError::Internal(
                "search plan authorization is not trusted for this runtime".to_string(),
            ));
        }
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(plan.corpus_snapshot());
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
                let fallback_plan = plan
                    .clone()
                    .with_original_query("fallback local text retrieval".to_string())
                    .map_err(RetrievalError::Compatibility)?;
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
            .allows_specialized(plan.original_query());
        let visual_enabled = self
            .visual_execution_policy
            .allows_visual(plan.original_query());
        let sparse_enabled = self
            .learned_sparse_execution_policy
            .allows_sparse(plan.original_query());
        self.retrievers
            .iter()
            .filter(|retriever| {
                let descriptor = retriever.descriptor();
                super::batch_is_eligible(
                    descriptor,
                    &self.hybrid_policy,
                    repository_specialized,
                    plan.original_query(),
                ) && crate::visual_benchmark::visual_lane_is_eligible(descriptor, visual_enabled)
                    && crate::learned_sparse_policy::sparse_lane_is_eligible(
                        descriptor,
                        sparse_enabled,
                    )
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
            .should_shadow(plan.original_query())
        {
            return Vec::new();
        }
        self.retrievers
            .iter()
            .filter(|retriever| {
                crate::learned_sparse_policy::is_sparse_descriptor(retriever.descriptor())
            })
            .cloned()
            .collect()
    }

    pub(super) fn prompt_injection_outcome(
        &self,
        plan: &SearchPlan,
        source_filter: Option<&crate::types::CandidateSourceFilter>,
    ) -> RetrievalResult<SearchOutcome> {
        let retriever_ids = self
            .retrievers
            .iter()
            .map(|retriever| retriever.descriptor().id.clone())
            .collect();
        let policy_fingerprint = plan.authorization().canonical_fingerprint();
        let mut trace = SearchTrace::from_plan(
            plan,
            retriever_ids,
            &[],
            applied_security_filters(plan, &self.security_policy),
            self.fusion.as_ref().map(|_| "configured".to_string()),
            Vec::new(),
            SearchStopReason::PolicyDenied,
        )?
        .with_policy_fingerprint(policy_fingerprint);
        trace.source_selection_digest =
            source_filter.map(crate::types::CandidateSourceFilter::digest);
        Ok(SearchOutcome::from_trace(
            trace,
            plan,
            SearchStatus::QuarantinedForReview,
            Vec::new(),
            EvidenceCoverage::new(EvidenceCoverageDto {
                required_claims: vec![],
                required_subquestions: vec![],
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
                candidate_coverage_keys: vec![],
                percent_covered: 0,
                gaps_identified: vec![],
            })?,
            Vec::new(),
        ))
    }
}
