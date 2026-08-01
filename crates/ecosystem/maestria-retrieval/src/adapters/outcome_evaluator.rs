use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::{SearchOutcome, SearchStatus, SearchStopReason};
use maestria_ports::EvidenceRepository;

use crate::traits::RetrievalEvaluator;
use crate::types::{RetrievalError, RetrievalEvaluationReport};

/// Evaluates already-filtered evidence candidates into a durable outcome.
pub struct EvidenceOutcomeEvaluator {
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
}

impl EvidenceOutcomeEvaluator {
    pub fn new(evidence: Arc<dyn EvidenceRepository + Send + Sync>) -> Self {
        Self { evidence }
    }
}

#[async_trait]
impl RetrievalEvaluator for EvidenceOutcomeEvaluator {
    async fn evaluate(
        &self,
        experiment: crate::types::RetrievalExperiment,
    ) -> Result<RetrievalEvaluationReport, RetrievalError> {
        let evidence = experiment.candidates;
        let status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else {
            SearchStatus::Answerable
        };
        let coverage = maestria_domain::EvidenceCoverage {
            percent_covered: if evidence.is_empty() { 0 } else { 100 },
            gaps_identified: Vec::new(),
            required_claims: experiment
                .plan
                .evidence_requirements()
                .required_claims
                .clone(),
            required_subquestions: experiment
                .plan
                .evidence_requirements()
                .required_subquestions
                .clone(),
            distinct_sources: evidence.len(),
            distinct_documents: evidence.len(),
            distinct_sections: evidence.len(),
            candidate_coverage_keys: evidence
                .iter()
                .flat_map(|candidate| candidate.coverage_keys.clone())
                .collect(),
        };
        let stop_reason = if evidence.is_empty() {
            SearchStopReason::NoEvidence
        } else if evidence.len() >= experiment.plan.stop_conditions().max_results as usize {
            SearchStopReason::ResultsLimit
        } else {
            SearchStopReason::EvidenceComplete
        };
        let diversity = maestria_domain::SearchTraceDiversity {
            distinct_sources: coverage.distinct_sources,
            distinct_documents: coverage.distinct_documents,
            distinct_sections: coverage.distinct_sections,
            required_claims: coverage.required_claims.clone(),
            required_subquestions: coverage.required_subquestions.clone(),
            covered_keys: coverage.candidate_coverage_keys.clone(),
            stop_reason: stop_reason.clone(),
            candidates: evidence
                .iter()
                .enumerate()
                .map(
                    |(rank, candidate)| maestria_domain::SearchTraceDiversityCandidate {
                        candidate_id: candidate.evidence_id,
                        original_rank: rank,
                        selected_rank: Some(rank),
                        duplicate_cluster: candidate.duplicate_cluster,
                        marginal_coverage: 100,
                        coverage_keys: candidate.coverage_keys.clone(),
                    },
                )
                .collect(),
        };
        let policy_fingerprint = match experiment.plan.authorization().as_ref() {
            Some(authorization) => authorization.canonical_fingerprint(),
            None => {
                return Err(RetrievalError::Internal(
                    "search plan authorization snapshot is missing".to_string(),
                ));
            }
        };
        let mut trace = maestria_domain::SearchTrace::from_plan(
            &experiment.plan,
            vec!["evidence".to_string()],
            &evidence,
            Vec::new(),
            None,
            Vec::new(),
            stop_reason.clone(),
        )
        .with_policy_fingerprint(policy_fingerprint);
        trace.diversity = Some(diversity);
        let outcome = SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: experiment.plan.fingerprint().clone(),
            index_generation: experiment.plan.index_generation(),
            status,
            evidence,
            coverage,
            conflicts: Vec::new(),
        };
        outcome.verify_compatibility(&experiment.plan)?;
        let _ = &self.evidence;
        Ok(RetrievalEvaluationReport {
            evaluated_candidates: outcome.evidence.len(),
            outcome,
        })
    }
}
