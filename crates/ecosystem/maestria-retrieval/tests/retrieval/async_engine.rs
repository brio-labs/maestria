use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, EvidenceCoverage, IndexGenerationId, SearchIntent, SearchOutcome,
    SearchStatus, SearchTraceId,
};
use maestria_retrieval::{
    CandidateRetriever, FixedKRrf, RetrievalEngine, RetrievalError, RetrievalEvaluator,
    RetrievalResult,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::common::{candidate_fixture, dummy_plan};

struct AsyncLane {
    id: &'static str,
    fail: bool,
    candidate: Option<EvidenceCandidate>,
}

#[async_trait]
impl CandidateRetriever for AsyncLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: self.id.to_string(),
            modality: "text".to_string(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        }
    }

    async fn retrieve(
        &self,
        _request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, maestria_retrieval::RetrievalError> {
        if self.fail {
            return Err(RetrievalError::Internal("dense unavailable".to_string()));
        }
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            query: "test query".to_string(),
            candidates: self.candidate.clone().into_iter().collect(),
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(IndexGenerationId::new(1)),
            bytes_read: 0,
        })
    }
}

struct CountingWebLane {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl CandidateRetriever for CountingWebLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: "web".to_string(),
            modality: "web".to_string(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        }
    }

    async fn retrieve(
        &self,
        _request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            candidates: Vec::new(),
            query: String::new(),
            status: maestria_domain::SearchLaneStatus::Empty,
            generation: Some(IndexGenerationId::new(1)),
            bytes_read: 0,
        })
    }
}

struct AsyncEvaluator;

#[async_trait]
impl RetrievalEvaluator for AsyncEvaluator {
    async fn evaluate(
        &self,
        experiment: maestria_retrieval::types::RetrievalExperiment,
    ) -> RetrievalResult<maestria_retrieval::types::RetrievalEvaluationReport> {
        let evidence = experiment.candidates;
        let status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else {
            SearchStatus::Answerable
        };
        let coverage = if evidence.is_empty() { 0 } else { 100 };
        Ok(maestria_retrieval::types::RetrievalEvaluationReport {
            evaluated_candidates: evidence.len(),
            outcome: SearchOutcome {
                trace: SearchTraceId::new(0),
                trace_data: None,
                fingerprint: experiment.plan.fingerprint.clone(),
                index_generation: experiment.plan.index_generation,
                status,
                evidence,
                coverage: EvidenceCoverage {
                    required_claims: vec![],
                    required_subquestions: vec![],
                    distinct_sources: 0,
                    distinct_documents: 0,
                    distinct_sections: 0,
                    candidate_coverage_keys: vec![],
                    percent_covered: coverage,
                    gaps_identified: vec![],
                },
                conflicts: vec![],
            },
        })
    }
}

#[tokio::test]
async fn failed_lane_is_degraded_without_losing_successful_evidence() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let engine = RetrievalEngine::new(
        vec![
            Arc::new(AsyncLane {
                id: "lexical",
                fail: false,
                candidate: Some(candidate_fixture()?),
            }),
            Arc::new(AsyncLane {
                id: "dense",
                fail: true,
                candidate: None,
            }),
        ],
        Arc::new(AsyncEvaluator),
    )
    .with_fusion(Arc::new(FixedKRrf::new(60)));

    let outcome = engine.search(&plan).await?;
    assert_eq!(outcome.evidence.len(), 1);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert_eq!(
        trace
            .lanes
            .iter()
            .map(|lane| lane.retriever_id.as_str())
            .collect::<Vec<_>>(),
        vec!["lexical", "dense"]
    );
    assert!(matches!(
        trace.lanes[0].status,
        maestria_domain::SearchLaneStatus::Succeeded
    ));
    assert!(matches!(
        trace.lanes[1].status,
        maestria_domain::SearchLaneStatus::Failed { .. }
    ));

    assert!(
        trace
            .lanes
            .iter()
            .all(|lane| lane.generation == Some(plan.index_generation))
    );
    Ok(())
}

#[tokio::test]
async fn web_budget_applies_across_deterministic_rewrites() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.original_query = "latest web PR".to_string();
    plan.intent = SearchIntent::CurrentWeb;
    plan.modalities = maestria_domain::ModalitySet::new(vec![maestria_domain::Modality::Web]);
    plan.budgets =
        maestria_domain::SearchBudget::with_resource_limits(1000, 1000, 8, 3, 1, 16_384, 1)?;
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = RetrievalEngine::new(
        vec![Arc::new(CountingWebLane {
            calls: Arc::clone(&calls),
        })],
        Arc::new(AsyncEvaluator),
    );

    let outcome = engine.search(&plan).await?;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert!(trace.lanes.iter().any(|lane| {
        matches!(
            lane.status,
            maestria_domain::SearchLaneStatus::Failed { .. }
        )
    }));
    Ok(())
}
