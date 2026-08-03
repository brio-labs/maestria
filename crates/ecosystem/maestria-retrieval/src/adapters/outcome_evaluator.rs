use std::sync::Arc;

use async_trait::async_trait;
use maestria_domain::SearchOutcome;
use maestria_ports::EvidenceRepository;

use crate::diversity::select_candidates;
use crate::traits::RetrievalEvaluator;
use crate::types::{RankedCandidate, RetrievalError, RetrievalEvaluationReport};

/// Evaluates already-filtered evidence candidates into a durable outcome.
///
/// Coverage, diversity, and status are derived by the shared diversity
/// selector ([`select_candidates`]) so the standalone evaluator reports the
/// same evidence-grounded metrics as the engine pipeline: distinct counts are
/// deduplicated, marginal coverage counts new coverage keys, and the status
/// reflects requirements satisfaction rather than mere candidate presence
/// (R46 — domain state must not be presented as established external facts).
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
        let candidates = experiment.candidates;
        let ranked = candidates
            .into_iter()
            .enumerate()
            .map(|(rank, candidate)| RankedCandidate { candidate, rank })
            .collect::<Vec<_>>();
        let selection = select_candidates(&ranked, &experiment.plan)?;
        let selected_evidence = selection
            .candidates
            .iter()
            .map(|ranked| ranked.candidate.clone())
            .collect::<Vec<_>>();
        let coverage = selection.coverage;
        let diversity = selection.trace;
        let status = selection.status;
        let stop_reason = diversity.stop_reason.clone();
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
            &selected_evidence,
            Vec::new(),
            None,
            Vec::new(),
            stop_reason,
        )?
        .with_policy_fingerprint(policy_fingerprint);
        trace.diversity = Some(diversity);
        let outcome = SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: experiment.plan.fingerprint().clone(),
            index_generation: experiment.plan.index_generation(),
            status,
            evidence: selected_evidence,
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
