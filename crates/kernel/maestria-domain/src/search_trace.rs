use super::{
    ConflictSet, ConflictSetId, EvidenceCandidate, EvidenceCoverage, SearchCompatibilityError,
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace, SearchTraceDiversity,
    SearchTraceId,
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
                    SearchStatus::EvidenceIncomplete | SearchStatus::StaleEvidenceOnly
                ) || (*status == SearchStatus::Abstained && evidence_len == 0)
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
            mix(format!("{:?}", candidate.coverage_keys).as_bytes());
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
        if let Some(diversity) = &self.diversity {
            mix_diversity(&mut hash, diversity);
        }
        SearchTraceId::new(hash)
    }
}

fn mix_diversity(hash: &mut u64, diversity: &SearchTraceDiversity) {
    let mut mix = |bytes: &[u8]| {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    mix(&(diversity.distinct_sources as u64).to_le_bytes());
    mix(&(diversity.distinct_documents as u64).to_le_bytes());
    mix(&(diversity.distinct_sections as u64).to_le_bytes());
    for claim in &diversity.required_claims {
        mix(claim.as_bytes());
    }
    for subquestion in &diversity.required_subquestions {
        mix(subquestion.as_bytes());
    }
    for key in &diversity.covered_keys {
        mix(key.as_bytes());
    }
    mix(format!("{:?}", diversity.stop_reason).as_bytes());
    for candidate in &diversity.candidates {
        mix(&candidate.candidate_id.value().to_le_bytes());
        mix(&(candidate.original_rank as u64).to_le_bytes());
        mix(format!("{:?}", candidate.selected_rank).as_bytes());
        mix(format!("{:?}", candidate.duplicate_cluster).as_bytes());
        mix(&u64::from(candidate.marginal_coverage).to_le_bytes());
        for key in &candidate.coverage_keys {
            mix(key.as_bytes());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CorpusScope, EvidenceRequirements, FreshnessRequirement, ModalitySet,
        RetrievalModelFingerprint, SearchBudget, SearchIntent, SearchPlan, StopConditions,
        ids::{CorpusSnapshotId, IndexGenerationId, QueryId},
    };

    #[test]
    fn test_deterministic_id_with_diversity() -> Result<(), SearchCompatibilityError> {
        let plan = SearchPlan {
            query_id: QueryId::new(1),
            original_query: "test".to_string(),
            intent: SearchIntent::ExactLookup,
            scope: CorpusScope::Global,
            corpus_snapshot: CorpusSnapshotId::new(1),
            index_generation: IndexGenerationId::new(1),
            freshness: FreshnessRequirement::Any,
            modalities: ModalitySet::new(vec![]),
            stages: vec![],
            budgets: SearchBudget::new(1000, 1000)?,
            stop_conditions: StopConditions {
                max_results: 10,
                min_score_threshold: 0,
            },
            evidence_requirements: EvidenceRequirements {
                required_claims: vec![],
                required_subquestions: vec![],
                minimum_sources: 0,
                minimum_documents: 0,
                minimum_sections: 0,
                require_primary_sources: false,
                minimum_corroboration: 1,
            },
            fingerprint: RetrievalModelFingerprint::new("test".to_string())?,
        };

        let mut trace = SearchTrace::from_plan(
            &plan,
            vec![],
            &[],
            vec![],
            None,
            vec![],
            SearchStopReason::EvidenceComplete,
        );
        let id1 = trace.deterministic_id();
        trace.diversity = Some(crate::SearchTraceDiversity {
            distinct_sources: 1,
            distinct_documents: 1,
            distinct_sections: 1,
            required_claims: vec!["claim1".to_string()],
            required_subquestions: vec![],
            covered_keys: vec!["claim1".to_string()],
            stop_reason: SearchStopReason::EvidenceComplete,
            candidates: vec![],
        });
        let id2 = trace.deterministic_id();
        assert_ne!(id1, id2);
        let diversity =
            trace
                .diversity
                .as_mut()
                .ok_or(SearchCompatibilityError::TracePlanMismatch(
                    "diversity trace fixture missing",
                ))?;
        diversity
            .candidates
            .push(crate::SearchTraceDiversityCandidate {
                candidate_id: crate::ids::EvidenceId::new(1),
                original_rank: 0,
                selected_rank: Some(0),
                duplicate_cluster: None,
                marginal_coverage: 1,
                coverage_keys: vec!["key1".to_string()],
            });
        let id3 = trace.deterministic_id();
        assert_ne!(id2, id3);
        Ok(())
    }
}
