use super::{
    ConflictSet, ConflictSetId, EvidenceCandidate, EvidenceCoverage, SearchCompatibilityError,
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace, SearchTraceId,
};

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
                    })
                })
    }

    pub fn matches_outcome(&self, status: &SearchStatus, evidence_len: usize) -> bool {
        match &self.stop_reason {
            SearchStopReason::ResultsLimit => {
                matches!(
                    status,
                    SearchStatus::Answerable | SearchStatus::AnswerableWithWarnings
                ) && evidence_len >= self.stop_conditions.max_results as usize
            }
            SearchStopReason::EvidenceComplete => {
                matches!(
                    status,
                    SearchStatus::Answerable | SearchStatus::AnswerableWithWarnings
                ) && evidence_len < self.stop_conditions.max_results as usize
            }
            SearchStopReason::RequirementsUnmet => matches!(
                status,
                SearchStatus::EvidenceIncomplete
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
                    SearchStatus::EvidenceIncomplete | SearchStatus::StaleEvidenceOnly
                ) || (*status == SearchStatus::Abstained && evidence_len == 0)
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
        percent_consistent
            && self.missing_evidence == coverage.gaps_identified
            && self.conflicts
                == conflicts
                    .iter()
                    .map(|conflict| conflict.id)
                    .collect::<Vec<_>>()
    }

    pub fn deterministic_id(&self) -> SearchTraceId {
        let mut hash = 0xcbf29ce484222325u64;
        let mut mix = |bytes: &[u8]| {
            for byte in bytes {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        };
        mix(&self.query_id.value().to_le_bytes());
        mix(self.original_query.as_bytes());
        mix(format!("{:?}", self.intent).as_bytes());
        mix(format!("{:?}", self.scope).as_bytes());
        mix(format!("{:?}", self.freshness).as_bytes());
        mix(format!("{:?}", self.modalities).as_bytes());
        mix(format!("{:?}", self.stages).as_bytes());
        mix(format!("{:?}", self.evidence_requirements).as_bytes());
        mix(&self.corpus_snapshot.value().to_le_bytes());
        mix(&self.index_generation.value().to_le_bytes());
        mix(self.fingerprint.as_str().as_bytes());
        for retriever in &self.retrievers {
            mix(retriever.as_bytes());
        }
        mix(format!("{:?}", self.policy_fingerprint).as_bytes());
        mix(&u64::from(self.budgets.max_tokens()).to_le_bytes());
        mix(&u64::from(self.budgets.max_latency_ms()).to_le_bytes());
        mix(&u64::from(self.stop_conditions.max_results).to_le_bytes());
        mix(&u64::from(self.stop_conditions.min_score_threshold).to_le_bytes());
        for candidate in &self.raw_candidates {
            mix(&candidate.evidence_id.value().to_le_bytes());
            mix(&candidate.artifact_version.value().to_le_bytes());
            mix(format!("{:?}", candidate.source_span).as_bytes());
            mix(&u64::from(candidate.rank).to_le_bytes());
            mix(&u64::from(candidate.scores.bm25).to_le_bytes());
            mix(&u64::from(candidate.scores.semantic_similarity).to_le_bytes());
            mix(format!("{:?}", candidate.trust).as_bytes());
            mix(format!("{:?}", candidate.freshness).as_bytes());
            mix(format!("{:?}", candidate.duplicate_cluster).as_bytes());
            mix(format!("{:?}", candidate.reasons).as_bytes());
        }
        mix(format!("{:?}", self.fusion).as_bytes());
        mix(format!("{:?}", self.filters).as_bytes());
        mix(format!("{:?}", self.expansions).as_bytes());
        mix(format!("{:?}", self.missing_evidence).as_bytes());
        for conflict in &self.conflicts {
            mix(&conflict.value().to_le_bytes());
        }
        mix(format!("{:?}", self.stop_reason).as_bytes());
        for lane in &self.lanes {
            mix(lane.retriever_id.as_bytes());
            mix(format!("{:?}", lane.status).as_bytes());
            for candidate in &lane.candidates {
                mix(&candidate.evidence_id.value().to_le_bytes());
                mix(&candidate.artifact_version.value().to_le_bytes());
                mix(format!("{:?}", candidate.source_span).as_bytes());
                mix(&u64::from(candidate.lane_rank).to_le_bytes());
                mix(format!("{:?}", candidate.duplicate_cluster).as_bytes());
                mix(&u64::from(candidate.scores.bm25).to_le_bytes());
                mix(&u64::from(candidate.scores.semantic_similarity).to_le_bytes());
                mix(format!("{:?}", candidate.reasons).as_bytes());
            }
        }
        if let Some(rerank) = &self.rerank {
            mix(rerank.model.as_bytes());
            mix(rerank.fingerprint.as_str().as_bytes());
            let input_cap = rerank.input_cap as u64;
            let score_cap = rerank.score_cap as u64;
            let output_cap = rerank.output_cap as u64;
            mix(&input_cap.to_le_bytes());
            mix(&score_cap.to_le_bytes());
            mix(&output_cap.to_le_bytes());
            for c in &rerank.candidates {
                mix(&c.candidate_id.value().to_le_bytes());
                let original_rank = c.original_rank as u64;
                mix(&original_rank.to_le_bytes());
                mix(format!("{:?}", c.new_rank).as_bytes());
                mix(format!("{:?}", c.status).as_bytes());
                mix(format!("{:?}", c.relevance_score).as_bytes());
                mix(format!("{:?}", c.constraint_score).as_bytes());
                for constraint in &c.constraint_scores {
                    mix(constraint.name.as_bytes());
                    mix(&u64::from(constraint.score).to_le_bytes());
                }
            }
        }
        SearchTraceId::new(hash)
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
