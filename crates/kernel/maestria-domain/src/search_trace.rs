use super::{
    ConflictSet, ConflictSetId, EvidenceCandidate, EvidenceCoverage, SearchCompatibilityError,
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace,
};

mod search_trace_identity;

impl SearchTrace {
    pub fn matches_plan(&self, plan: &SearchPlan) -> bool {
        self.query_id == plan.query_id
            && self.original_query == plan.original_query
            && self.intent == plan.intent
            && self.scope == plan.scope
            && self.corpus_snapshot == plan.corpus_snapshot
            && self.index_generation == plan.index_generation
            && self.freshness == plan.freshness
            && self.modalities == plan.modalities
            && self.stages == plan.stages
            && self.budgets == plan.budgets
            && self.stop_conditions == plan.stop_conditions
            && self.evidence_requirements == plan.evidence_requirements
            && self.fingerprint == plan.fingerprint
    }

    pub fn with_gaps_and_conflicts(
        mut self,
        missing_evidence: Vec<String>,
        conflicts: Vec<ConflictSetId>,
    ) -> Self {
        self.missing_evidence = missing_evidence;
        self.conflicts = conflicts;
        self
    }

    pub fn with_policy_fingerprint(mut self, policy_fingerprint: String) -> Self {
        self.policy_fingerprint = Some(policy_fingerprint);
        self
    }
    pub fn with_lanes(mut self, lanes: Vec<super::SearchTraceLane>) -> Self {
        self.lanes = lanes;
        self
    }

    pub fn matches_evidence(&self, evidence: &[EvidenceCandidate]) -> bool {
        self.raw_candidates.len() == evidence.len()
            && self
                .raw_candidates
                .iter()
                .enumerate()
                .all(|(rank, traced)| {
                    evidence.get(rank).is_some_and(|candidate| {
                        traced.evidence_id == candidate.evidence_id
                            && traced.artifact_version == candidate.artifact_version
                            && traced.source_span == candidate.source_span
                            && traced.rank == rank as u32
                            && traced.scores == candidate.scores
                            && traced.trust == candidate.trust
                            && traced.freshness == candidate.freshness
                            && traced.duplicate_cluster == candidate.duplicate_cluster
                            && traced.reasons == candidate.reasons
                            && traced.coverage_keys == candidate.coverage_keys
                    })
                })
    }

    pub fn matches_outcome(&self, status: &SearchStatus, evidence_len: usize) -> bool {
        match &self.stop_reason {
            SearchStopReason::ResultsLimit => {
                matches!(
                    status,
                    SearchStatus::Answerable
                        | SearchStatus::AnswerableWithWarnings
                        | SearchStatus::EvidenceIncomplete
                        | SearchStatus::StaleEvidenceOnly
                ) && evidence_len >= self.stop_conditions.max_results as usize
            }
            SearchStopReason::EvidenceComplete => {
                matches!(
                    status,
                    SearchStatus::Answerable
                        | SearchStatus::AnswerableWithWarnings
                        | SearchStatus::StaleEvidenceOnly
                ) && evidence_len < self.stop_conditions.max_results as usize
            }
            SearchStopReason::RequirementsUnmet => matches!(
                status,
                SearchStatus::AnswerableWithWarnings
                    | SearchStatus::EvidenceIncomplete
                    | SearchStatus::StaleEvidenceOnly
                    | SearchStatus::SourcesConflict
            ),
            SearchStopReason::NoEvidence => {
                *status == SearchStatus::NoEvidenceFound && evidence_len == 0
            }
            SearchStopReason::PolicyDenied => {
                matches!(
                    status,
                    SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview
                ) && evidence_len == 0
            }
            SearchStopReason::Abstained => *status == SearchStatus::Abstained && evidence_len == 0,
            SearchStopReason::BudgetExhausted => {
                matches!(
                    status,
                    SearchStatus::AnswerableWithWarnings
                        | SearchStatus::EvidenceIncomplete
                        | SearchStatus::StaleEvidenceOnly
                ) || (*status == SearchStatus::NoEvidenceFound && evidence_len == 0)
                    || (*status == SearchStatus::Abstained && evidence_len == 0)
            }
            SearchStopReason::LowMarginalGain => {
                matches!(
                    status,
                    SearchStatus::Answerable
                        | SearchStatus::AnswerableWithWarnings
                        | SearchStatus::EvidenceIncomplete
                        | SearchStatus::StaleEvidenceOnly
                )
            }
        }
    }

    pub fn matches_coverage(
        &self,
        coverage: &EvidenceCoverage,
        conflicts: &[ConflictSet],
        evidence_len: usize,
    ) -> bool {
        let percent_consistent = match (evidence_len, self.missing_evidence.is_empty()) {
            (0, _) => coverage.percent_covered == 0,
            (_, true) => coverage.percent_covered == 100,
            (_, false) => coverage.percent_covered < 100,
        };
        let diversity_consistent = match &self.diversity {
            Some(trace) => {
                trace.distinct_sources == coverage.distinct_sources
                    && trace.distinct_documents == coverage.distinct_documents
                    && trace.distinct_sections == coverage.distinct_sections
                    && trace.required_claims == coverage.required_claims
                    && trace.required_subquestions == coverage.required_subquestions
                    && trace.covered_keys == coverage.candidate_coverage_keys
            }
            None => {
                coverage.distinct_sources == 0
                    && coverage.distinct_documents == 0
                    && coverage.distinct_sections == 0
                    && coverage.required_claims.is_empty()
                    && coverage.required_subquestions.is_empty()
                    && coverage.candidate_coverage_keys.is_empty()
            }
        };
        percent_consistent
            && diversity_consistent
            && self.missing_evidence == coverage.gaps_identified
            && self.conflicts
                == conflicts
                    .iter()
                    .map(|conflict| conflict.id)
                    .collect::<Vec<_>>()
    }

    pub fn validate_rewrites(&self) -> Result<(), SearchCompatibilityError> {
        if self.rewrites.is_empty() {
            return if self.identity_version == 0 {
                Ok(())
            } else {
                Err(SearchCompatibilityError::TracePlanMismatch(
                    "search trace is missing rewrite provenance",
                ))
            };
        }
        let mut original_seen = false;
        let mut model_seen = false;
        let mut token_total = 0_u64;
        let mut latency_total = 0_u64;
        for rewrite in &self.rewrites {
            let (next_original, next_model) =
                self.validate_rewrite_record(rewrite, original_seen, model_seen)?;
            original_seen = next_original;
            model_seen = next_model;
            token_total = token_total.saturating_add(u64::from(rewrite.accounting.token_estimate));
            latency_total =
                latency_total.saturating_add(u64::from(rewrite.accounting.latency_budget_units));
        }
        if !original_seen
            || self.rewrites.len() > self.budgets.max_queries() as usize
            || token_total > u64::from(self.budgets.max_tokens())
            || latency_total > u64::from(self.budgets.max_latency_ms())
        {
            return Err(SearchCompatibilityError::TracePlanMismatch(
                "rewrite trace exceeds its plan budget",
            ));
        }
        Ok(())
    }

    fn validate_rewrite_record(
        &self,
        rewrite: &super::SearchTraceRewrite,
        original_seen: bool,
        model_seen: bool,
    ) -> Result<(bool, bool), SearchCompatibilityError> {
        if !original_seen && rewrite.origin != super::SearchRewriteOrigin::Original {
            return Err(SearchCompatibilityError::TracePlanMismatch(
                "rewrite trace must begin with the original query",
            ));
        }
        let expected_tokens = rewrite.query.split_whitespace().count().max(1);
        if rewrite.accounting.token_estimate as usize != expected_tokens
            || rewrite.accounting.latency_budget_units == 0
        {
            return Err(SearchCompatibilityError::TracePlanMismatch(
                "rewrite accounting is invalid",
            ));
        }
        if rewrite.accounting.is_proposal
            != (rewrite.origin == super::SearchRewriteOrigin::ModelProposal)
        {
            return Err(SearchCompatibilityError::TracePlanMismatch(
                "rewrite proposal accounting is invalid",
            ));
        }
        let mut original_seen = original_seen;
        let mut model_seen = model_seen;
        match rewrite.origin {
            super::SearchRewriteOrigin::Original => {
                if original_seen
                    || rewrite.query != self.original_query
                    || rewrite.stage != super::SearchRewriteStage::InitialRetrieval
                {
                    return Err(SearchCompatibilityError::TracePlanMismatch(
                        "original rewrite identity is invalid",
                    ));
                }
                original_seen = true;
            }
            super::SearchRewriteOrigin::Deterministic => {
                if rewrite.stage != super::SearchRewriteStage::InitialRetrieval || model_seen {
                    return Err(SearchCompatibilityError::TracePlanMismatch(
                        "deterministic rewrites must precede model proposals",
                    ));
                }
            }
            super::SearchRewriteOrigin::ModelProposal => {
                if !matches!(
                    rewrite.stage,
                    super::SearchRewriteStage::Reranking
                        | super::SearchRewriteStage::IterativeRetrieval
                ) {
                    return Err(SearchCompatibilityError::TracePlanMismatch(
                        "model rewrite stage is invalid",
                    ));
                }
                model_seen = true;
            }
            super::SearchRewriteOrigin::Feedback => {
                if !matches!(
                    rewrite.stage,
                    super::SearchRewriteStage::Reranking
                        | super::SearchRewriteStage::IterativeRetrieval
                ) {
                    return Err(SearchCompatibilityError::TracePlanMismatch(
                        "feedback rewrite stage is invalid",
                    ));
                }
            }
            super::SearchRewriteOrigin::MissingSlot => {
                let Some(slot) = rewrite.missing_slot.as_deref() else {
                    return Err(SearchCompatibilityError::TracePlanMismatch(
                        "missing-slot rewrite is not identified",
                    ));
                };
                let declared_required = self
                    .evidence_requirements
                    .required_claims
                    .iter()
                    .chain(self.evidence_requirements.required_subquestions.iter())
                    .any(|required| required == slot);
                if rewrite.stage != super::SearchRewriteStage::IterativeRetrieval
                    || slot.trim().is_empty()
                    || (!self.missing_evidence.iter().any(|gap| gap == slot) && !declared_required)
                {
                    return Err(SearchCompatibilityError::TracePlanMismatch(
                        "missing-slot rewrite is not identified",
                    ));
                }
            }
        }
        Ok((original_seen, model_seen))
    }
}

impl SearchOutcome {
    pub fn verify_compatibility(&self, plan: &SearchPlan) -> Result<(), SearchCompatibilityError> {
        if self.fingerprint != plan.fingerprint {
            return Err(SearchCompatibilityError::ModelFingerprintMismatch {
                expected: plan.fingerprint.clone(),
                found: self.fingerprint.clone(),
            });
        }
        if self.index_generation != plan.index_generation {
            return Err(SearchCompatibilityError::IndexGenerationMismatch {
                expected: plan.index_generation,
                found: self.index_generation,
            });
        }
        if let Some(trace) = &self.trace_data {
            if self.trace != trace.deterministic_id() {
                return Err(SearchCompatibilityError::TracePlanMismatch(
                    "trace identity differs",
                ));
            }
            trace.validate_rewrites()?;
            if !trace.matches_plan(plan) {
                return Err(SearchCompatibilityError::TracePlanMismatch(
                    "plan configuration differs",
                ));
            }
            if !trace.matches_evidence(&self.evidence) {
                return Err(SearchCompatibilityError::TracePlanMismatch(
                    "candidate provenance differs",
                ));
            }
            if !trace.matches_coverage(&self.coverage, &self.conflicts, self.evidence.len()) {
                return Err(SearchCompatibilityError::TracePlanMismatch(
                    "coverage or conflicts differ",
                ));
            }
            if !trace.matches_outcome(&self.status, self.evidence.len()) {
                return Err(SearchCompatibilityError::TracePlanMismatch(
                    "stop reason differs",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "search_trace_tests.rs"]
mod tests;
