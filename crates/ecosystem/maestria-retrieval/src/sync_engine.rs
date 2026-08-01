use crate::SyncPipeline;
use maestria_domain::{
    EvidenceCandidate, EvidenceCoverage, SearchExecution, SearchExecutionBudget, SearchLaneStatus,
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace, SearchTraceLane,
    SearchTraceLaneCandidate, SecurityMetadata, TrustLabel, TrustZone,
};

use crate::engine::{EnsureTraceOptions, applied_security_filters, ensure_trace, reconcile_status};
use crate::types::{RankedCandidate, RetrievalError, RetrievalResult};
fn candidate_security_metadata(candidate: &EvidenceCandidate) -> SecurityMetadata {
    let mut metadata = SecurityMetadata::default();
    match candidate.trust {
        TrustLabel::Verified => {
            metadata.trust_zone = TrustZone::Verified;
            metadata.integrity = maestria_domain::IntegrityState::Verified;
            metadata.review_status = maestria_domain::ReviewStatus::Approved;
            metadata.authority = maestria_domain::Authority::User;
        }
        TrustLabel::Unverified => {}
        TrustLabel::Disputed => {
            metadata.review_status = maestria_domain::ReviewStatus::Pending;
        }
        TrustLabel::Deprecated => {
            metadata.trust_zone = TrustZone::Quarantined;
            metadata.integrity = maestria_domain::IntegrityState::Compromised;
            metadata.review_status = maestria_domain::ReviewStatus::Rejected;
            metadata.read_allowed = false;
        }
    }
    metadata
}

fn filter_sync_candidates(
    candidates: Vec<EvidenceCandidate>,
    _plan: &SearchPlan,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> RetrievalResult<(Vec<EvidenceCandidate>, SearchLaneStatus)> {
    let mut allowed = Vec::with_capacity(candidates.len());
    let mut first_denial = None;
    for candidate in candidates {
        match policy.evaluate(&candidate_security_metadata(&candidate)) {
            maestria_governance::RetrievalDecision::Allowed => allowed.push(candidate),
            maestria_governance::RetrievalDecision::Denied(reason) => {
                if first_denial.is_none() {
                    first_denial = Some(reason);
                }
            }
        }
    }
    let status = if !allowed.is_empty() {
        SearchLaneStatus::Succeeded
    } else if let Some(reason) = first_denial {
        SearchLaneStatus::Failed {
            error: format!("candidate denied by security policy: {reason}"),
        }
    } else {
        SearchLaneStatus::Empty
    };
    Ok((allowed, status))
}

pub struct SyncRetrievalEngine<'a> {
    pipeline: SyncPipeline<'a, EvidenceCandidate, SearchOutcome>,
    security_policy: maestria_governance::RetrievalSecurityPolicy,
}
impl<'a> SyncRetrievalEngine<'a> {
    pub fn new<R, V>(
        retrievers: Vec<R>,
        evaluator: V,
        security_policy: maestria_governance::RetrievalSecurityPolicy,
    ) -> Self
    where
        R: Fn(&SearchPlan, SearchExecutionBudget) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
        V: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<SearchOutcome> + 'a,
    {
        let candidate_policy = security_policy.clone();
        let pipeline = SyncPipeline::new(retrievers, evaluator)
            .with_security_policy(security_policy.clone())
            .with_candidate_filter(move |candidates, plan| {
                filter_sync_candidates(candidates, plan, &candidate_policy)
            })
            .with_pre_expander(|candidates, plan| {
                let ranked = candidates
                    .into_iter()
                    .enumerate()
                    .map(|(rank, candidate)| RankedCandidate { candidate, rank })
                    .collect::<Vec<_>>();
                Ok(crate::diversity::select_candidates(&ranked, plan)
                    .candidates
                    .into_iter()
                    .map(|candidate| candidate.candidate)
                    .collect())
            });
        Self {
            pipeline,
            security_policy,
        }
    }
    pub fn with_query_retriever<F>(mut self, retriever: F) -> Self
    where
        F: Fn(&SearchPlan, &str, SearchExecutionBudget) -> RetrievalResult<Vec<EvidenceCandidate>>
            + 'a,
    {
        self.pipeline = self.pipeline.with_query_retriever(retriever);
        self
    }

    pub fn with_fusion<F>(mut self, fusion: F) -> Self
    where
        F: Fn(Vec<Vec<EvidenceCandidate>>) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_fusion(fusion);
        self
    }

    pub fn with_reranker<F>(mut self, reranker: F) -> Self
    where
        F: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_reranker(reranker);
        self
    }

    pub fn with_expander<F>(mut self, expander: F) -> Self
    where
        F: Fn(Vec<EvidenceCandidate>, &SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> + 'a,
    {
        self.pipeline = self.pipeline.with_expander(expander);
        self
    }
    fn finalize_sync_outcome(
        &self,
        plan: &SearchPlan,
        mut outcome: SearchOutcome,
        lane_sets: Vec<(
            String,
            Vec<EvidenceCandidate>,
            SearchLaneStatus,
            SearchExecution,
        )>,
    ) -> RetrievalResult<SearchOutcome> {
        let budget_exhausted = lane_sets.iter().any(|(_, _, _, execution)| {
            matches!(
                execution.completion,
                maestria_domain::SearchExecutionCompletion::Exhausted(_)
            )
        });
        let policy_denied = !lane_sets.is_empty()
            && lane_sets.iter().all(|(_, candidates, status, _execution)| {
                candidates.is_empty()
                    && matches!(
                        status,
                        SearchLaneStatus::Failed { error }
                            if error.starts_with("candidate denied by security policy:")
                    )
            });
        if policy_denied && outcome.evidence.is_empty() {
            outcome.status = SearchStatus::DeniedByPolicy;
        }
        let ranked = outcome
            .evidence
            .iter()
            .cloned()
            .enumerate()
            .map(|(rank, candidate)| RankedCandidate { candidate, rank })
            .collect::<Vec<_>>();
        let diversity = crate::diversity::select_candidates(&ranked, plan);
        outcome.evidence = diversity
            .candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect();
        outcome.coverage = diversity.coverage;
        outcome.status = reconcile_status(&outcome.status, &diversity.status);
        if budget_exhausted
            && matches!(
                outcome.status,
                SearchStatus::Answerable | SearchStatus::AnswerableWithWarnings
            )
        {
            outcome.status = SearchStatus::EvidenceIncomplete;
        }
        let lanes = lane_sets
            .into_iter()
            .map(|(query, candidates, status, execution)| SearchTraceLane {
                retriever_id: "sync_pipeline".to_string(),
                generation: Some(plan.index_generation()),
                query,
                status,
                execution,
                candidates: candidates
                    .into_iter()
                    .enumerate()
                    .map(|(index, candidate)| SearchTraceLaneCandidate {
                        evidence_id: candidate.evidence_id,
                        artifact_version: candidate.artifact_version,
                        source_span: candidate.source_span,
                        lane_rank: (index + 1) as u32,
                        duplicate_cluster: candidate.duplicate_cluster,
                        scores: candidate.scores,
                        reasons: candidate.reasons,
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        let rewrites = if self.pipeline.query_rewrites_enabled() {
            crate::engine::rewrite_session(plan).trace_records()
        } else {
            crate::rewrite::QueryRewriteSession::with_limits(
                plan.original_query(),
                plan.budgets().max_tokens() as usize,
                plan.budgets().max_latency_ms(),
                plan.budgets().max_queries(),
            )
            .trace_records()
        };
        let outcome = ensure_trace(
            plan,
            outcome,
            lanes,
            EnsureTraceOptions {
                security_policy: self.security_policy.clone(),
                fusion_enabled: self.pipeline.fusion_enabled(),
                expansion_enabled: self.pipeline.expander_enabled(),
                rerank_trace: None,
                diversity_trace: Some(diversity.trace),
                rewrites,
                explicit_stop_reason: budget_exhausted.then_some(SearchStopReason::BudgetExhausted),
            },
        );
        outcome.verify_compatibility(plan)?;
        Ok(outcome)
    }

    pub fn search_sync(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        let expected_authorization = self
            .security_policy
            .authorization_context(plan.scope())
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval authorization denied: {error:?}"))
            })?
            .policy_snapshot();
        if plan.authorization().as_ref() != Some(&expected_authorization) {
            return Err(RetrievalError::Internal(
                "search plan authorization is not trusted for this runtime".to_string(),
            ));
        }
        if maestria_governance::contains_prompt_injection_risk(plan.original_query()) {
            return self.quarantine_outcome(plan);
        }
        let (outcome, lane_sets) = self.pipeline.run_with_trace(plan)?;
        self.finalize_sync_outcome(plan, outcome, lane_sets)
    }

    fn quarantine_outcome(&self, plan: &SearchPlan) -> RetrievalResult<SearchOutcome> {
        let policy_fingerprint = match plan.authorization().as_ref() {
            Some(authorization) => authorization.canonical_fingerprint(),
            None => {
                return Err(RetrievalError::Internal(
                    "search plan authorization snapshot is missing".to_string(),
                ));
            }
        };
        let trace = SearchTrace::from_plan(
            plan,
            Vec::new(),
            &[],
            applied_security_filters(plan, &self.security_policy),
            None,
            Vec::new(),
            SearchStopReason::PolicyDenied,
        )
        .with_policy_fingerprint(policy_fingerprint);
        Ok(SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: plan.fingerprint().clone(),
            index_generation: plan.index_generation(),
            status: SearchStatus::QuarantinedForReview,
            evidence: Vec::new(),
            coverage: EvidenceCoverage {
                percent_covered: 0,
                gaps_identified: vec![],
                required_claims: vec![],
                required_subquestions: vec![],
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
                candidate_coverage_keys: vec![],
            },
            conflicts: Vec::new(),
        })
    }
}
