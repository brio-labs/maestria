use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, EvidenceCandidateDto, EvidenceCoverage, EvidenceCoverageDto,
    IndexGenerationId, SearchIntent, SearchOutcome, SearchStatus, SearchTraceId,
};
use maestria_retrieval::{
    CandidateRetriever, FixedKRrf, HybridExecutionPolicy, HybridPromotionRecord, RetrievalEngine,
    RetrievalError, RetrievalEvaluator, RetrievalResult,
    golden::Metric,
    repository_benchmark::{
        MeasurementStatus, RepositoryBenchmarkComparison, RepositoryBenchmarkCorpus,
        RepositoryBenchmarkObservation, RepositoryExecutionPolicy, RepositoryExpectedOutcome,
        RepositoryQueryClass, RepositoryRoute,
    },
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::common::{candidate_fixture, dummy_plan};

fn execution(
    request: &maestria_retrieval::types::CandidateRequest,
    results: u64,
    bytes_read: u64,
) -> maestria_domain::SearchExecution {
    maestria_domain::SearchExecution::new(
        request.execution_budget,
        maestria_domain::SearchExecutionUsage::new(results, results, results, bytes_read),
        maestria_domain::SearchExecutionCompletion::Complete,
    )
}

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
        request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, maestria_retrieval::RetrievalError> {
        if self.fail {
            return Err(RetrievalError::Internal("dense unavailable".to_string()));
        }
        let candidate_count = if self.candidate.is_some() { 1 } else { 0 };
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            query: "test query".to_string(),
            candidates: self.candidate.clone().into_iter().collect(),
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(IndexGenerationId::new(1)),
            execution: execution(&request, candidate_count, 0),
        })
    }
}

struct StaleCodeLane {
    candidate: EvidenceCandidate,
}

#[async_trait]
impl CandidateRetriever for StaleCodeLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: "code_intel".to_string(),
            modality: "code".to_string(),
            representation: maestria_domain::RepresentationName::new("repository_code_v2"),
            generation: IndexGenerationId::new(1),
        }
    }

    async fn retrieve(
        &self,
        request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            query: request.query.q.clone(),
            candidates: vec![self.candidate.clone()],
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(IndexGenerationId::new(1)),
            execution: execution(&request, 1, 1),
        })
    }
}

fn promoted_exact_symbol_policy() -> Result<RepositoryExecutionPolicy, Box<dyn std::error::Error>> {
    let corpus = RepositoryBenchmarkCorpus::from_json(include_str!(
        "../fixtures/rust-repository-benchmark-v1.json"
    ))?;
    let mut observations = Vec::with_capacity(corpus.cases.len() * 2);
    for case in &corpus.cases {
        for route in [RepositoryRoute::PhaseC, RepositoryRoute::CodeSpecialized] {
            let specialized = route == RepositoryRoute::CodeSpecialized;
            let exact_symbol_win = specialized && case.class == RepositoryQueryClass::ExactSymbol;
            let expected_abstention = matches!(case.expected, RepositoryExpectedOutcome::Abstain);
            observations.push(RepositoryBenchmarkObservation {
                corpus_id: corpus.corpus_id.clone(),
                repository_revision: corpus.repository_revision.clone(),
                evaluation_date: "test".to_string(),
                index_generation: "test".to_string(),
                model_fingerprint: "test".to_string(),
                route_config: serde_json::Value::Null,
                case_id: case.case_id.clone(),
                route,
                exact_span_hits: usize::from(exact_symbol_win),
                evidence_chain_length: usize::from(exact_symbol_win),
                evidence_chain_measured: true,
                latency_ms: 1,
                freshness_error: false,
                abstained: expected_abstention,
                outcome_correct: !(case.class == RepositoryQueryClass::ExactSymbol && !specialized),
                memory_bytes: 0,
                disk_bytes: 0,
                privacy_violation: false,
                security_violation: false,
                energy_milliwatt_seconds: 0,
                citation_alignment: Metric::ZERO,
                measurement_status: MeasurementStatus::Measured,
            });
        }
    }
    let comparison = RepositoryBenchmarkComparison::evaluate(&corpus, &observations)?;
    Ok(RepositoryExecutionPolicy::Active(
        comparison.promotion("test-exact-symbol".to_string())?,
    ))
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
        request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            candidates: Vec::new(),
            query: String::new(),
            status: maestria_domain::SearchLaneStatus::Empty,
            generation: Some(IndexGenerationId::new(1)),
            execution: execution(&request, 0, 0),
        })
    }
}

struct StaleGenerationLane {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl CandidateRetriever for StaleGenerationLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: "stale".to_string(),
            modality: "text".to_string(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: IndexGenerationId::new(2),
        }
    }

    async fn retrieve(
        &self,
        _request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(RetrievalError::Internal(
            "stale lane should not be dispatched".to_string(),
        ))
    }
}

struct SpecializedGenerationLane {
    calls: Arc<AtomicUsize>,
    candidate: EvidenceCandidate,
}

#[async_trait]
impl CandidateRetriever for SpecializedGenerationLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: "dense_chunks".to_string(),
            modality: "dense".to_string(),
            representation: maestria_domain::RepresentationName::new("dense_text_v1"),
            generation: IndexGenerationId::new(2),
        }
    }

    async fn retrieve(
        &self,
        request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        let descriptor = self.descriptor();
        if request.expected_generation != descriptor.generation {
            return Err(RetrievalError::Internal(
                "specialized lane received the wrong generation".to_string(),
            ));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor,
            query: request.query.q.clone(),
            candidates: vec![self.candidate.clone()],
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(IndexGenerationId::new(2)),
            execution: execution(&request, 1, 1),
        })
    }
}

struct ByteOverrunLane {
    candidate: EvidenceCandidate,
    bytes_read: u64,
}

#[async_trait]
impl CandidateRetriever for ByteOverrunLane {
    fn descriptor(&self) -> maestria_retrieval::types::RetrieverDescriptor {
        maestria_retrieval::types::RetrieverDescriptor {
            id: "local".to_string(),
            modality: "text".to_string(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: IndexGenerationId::new(1),
        }
    }

    async fn retrieve(
        &self,
        request: maestria_retrieval::types::CandidateRequest,
    ) -> Result<maestria_retrieval::types::CandidateBatch, RetrievalError> {
        Ok(maestria_retrieval::types::CandidateBatch {
            descriptor: self.descriptor(),
            query: "test query".to_string(),
            candidates: vec![self.candidate.clone()],
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(IndexGenerationId::new(1)),
            execution: execution(&request, 1, self.bytes_read),
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
                fingerprint: experiment.plan.fingerprint().clone(),
                index_generation: experiment.plan.index_generation(),
                status,
                evidence,
                coverage: EvidenceCoverage::new(EvidenceCoverageDto {
                    required_claims: vec![],
                    required_subquestions: vec![],
                    distinct_sources: 0,
                    distinct_documents: 0,
                    distinct_sections: 0,
                    candidate_coverage_keys: vec![],
                    percent_covered: coverage,
                    gaps_identified: vec![],
                })?,
                conflicts: vec![],
            },
        })
    }
}

#[tokio::test]
async fn failed_lane_is_degraded_without_losing_successful_evidence() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let mut authorization = plan.authorization().clone();
    if let Some(authorization) = authorization.as_mut() {
        authorization.allow_unscoped_items = true;
    }
    let plan = plan.with_authorization(authorization)?;
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
        maestria_governance::RetrievalSecurityPolicy::new()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )
    .with_fusion(Arc::new(FixedKRrf::new(60)));

    let outcome = engine.search(&plan).await?;
    assert_eq!(outcome.evidence.len(), 1);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert_eq!(
        trace.policy_fingerprint.as_deref(),
        Some("trust=None;sensitivity=None;read_allowed=true;scope=None;unscoped=true")
    );
    assert_eq!(
        trace.filters,
        vec![
            maestria_domain::SearchTraceFilter::Quarantine,
            maestria_domain::SearchTraceFilter::PromptInjection,
            maestria_domain::SearchTraceFilter::Acl,
        ]
    );
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
            .all(|lane| lane.generation == Some(plan.index_generation()))
    );
    Ok(())
}

#[tokio::test]
async fn stale_code_only_evidence_is_not_served_and_retains_stale_trace() -> RetrievalResult<()> {
    let plan = dummy_plan()?
        .with_original_query("find function symbol compute".to_string())?
        .with_intent(SearchIntent::RepositoryCode)?
        .with_modalities(maestria_domain::ModalitySet::new(vec![
            maestria_domain::Modality::Code,
        ]))?;
    let candidate = candidate_fixture()?;
    let candidate = EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: candidate.evidence_id(),
        artifact_version: candidate.artifact_version(),
        source_span: candidate.source_span().clone(),
        scores: candidate.scores().clone(),
        trust: candidate.trust(),
        freshness: maestria_domain::FreshnessStatus::Stale,
        duplicate_cluster: candidate.duplicate_cluster(),
        reasons: candidate.reasons().to_vec(),
        coverage_keys: candidate.coverage_keys().to_vec(),
    })?;
    let engine = RetrievalEngine::new(
        vec![Arc::new(StaleCodeLane { candidate })],
        Arc::new(AsyncEvaluator),
        maestria_governance::RetrievalSecurityPolicy::default(),
    )
    .with_repository_execution_policy(
        promoted_exact_symbol_policy()
            .map_err(|error| RetrievalError::Internal(error.to_string()))?,
    );

    let outcome = engine.search(&plan).await?;
    assert!(outcome.evidence.is_empty());
    assert_eq!(outcome.status, SearchStatus::StaleEvidenceOnly);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert_eq!(
        trace.stop_reason,
        maestria_domain::SearchStopReason::RequirementsUnmet
    );
    let lane = trace
        .lanes
        .iter()
        .find(|lane| lane.retriever_id == "code_intel")
        .ok_or(RetrievalError::Internal("missing code lane trace".into()))?;
    assert_eq!(lane.candidates.len(), 1);
    Ok(())
}

#[tokio::test]
async fn stale_generation_lane_is_rejected_before_dispatch() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = RetrievalEngine::new(
        vec![Arc::new(StaleGenerationLane {
            calls: Arc::clone(&calls),
        })],
        Arc::new(AsyncEvaluator),
        maestria_governance::RetrievalSecurityPolicy::default(),
    )
    .with_capabilities(
        maestria_governance::SearchCapabilities::core_defaults(
            maestria_domain::CorpusSnapshotId::new(1),
            maestria_domain::IndexGenerationId::new(1),
            (1_000, 30_000),
        )
        .with_generation(maestria_domain::IndexGenerationId::new(2)),
    );

    let outcome = engine.search(&plan).await?;
    assert!(outcome.evidence.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    let lane = trace
        .lanes
        .first()
        .ok_or(RetrievalError::Internal("missing stale lane trace".into()))?;
    assert!(matches!(
        &lane.status,
        maestria_domain::SearchLaneStatus::Failed { error }
            if error.contains("stale retriever generation")
    ));
    assert_eq!(
        trace.stop_reason,
        maestria_domain::SearchStopReason::NoEvidence
    );
    Ok(())
}

#[tokio::test]
async fn specialized_generation_is_served_while_primary_stale_lane_is_rejected()
-> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let mut authorization = plan.authorization().clone();
    if let Some(authorization) = authorization.as_mut() {
        authorization.allow_unscoped_items = true;
    }
    let plan = plan.with_authorization(authorization)?;
    let specialized_calls = Arc::new(AtomicUsize::new(0));
    let stale_calls = Arc::new(AtomicUsize::new(0));
    let promotion =
        HybridPromotionRecord::new("dense-generation-test".to_string(), "test".to_string())
            .ok_or_else(|| {
                RetrievalError::Internal("invalid hybrid promotion fixture".to_string())
            })?;
    let engine = RetrievalEngine::new(
        vec![
            Arc::new(SpecializedGenerationLane {
                calls: Arc::clone(&specialized_calls),
                candidate: candidate_fixture()?,
            }),
            Arc::new(StaleGenerationLane {
                calls: Arc::clone(&stale_calls),
            }),
        ],
        Arc::new(AsyncEvaluator),
        maestria_governance::RetrievalSecurityPolicy::new()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
    )
    .with_hybrid_policy(HybridExecutionPolicy::Active(promotion))
    .with_fusion(Arc::new(FixedKRrf::new(60)))
    .with_capabilities(
        maestria_governance::SearchCapabilities::core_defaults(
            maestria_domain::CorpusSnapshotId::new(1),
            maestria_domain::IndexGenerationId::new(1),
            (1_000, 30_000),
        )
        .with_generation(maestria_domain::IndexGenerationId::new(2)),
    );

    let outcome = engine.search(&plan).await?;
    assert_eq!(specialized_calls.load(Ordering::SeqCst), 1);
    assert_eq!(stale_calls.load(Ordering::SeqCst), 0);
    assert_eq!(outcome.evidence.len(), 1);
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    let specialized = trace
        .lanes
        .iter()
        .find(|lane| lane.retriever_id == "dense_chunks")
        .ok_or(RetrievalError::Internal(
            "missing specialized lane trace".into(),
        ))?;
    assert_eq!(specialized.generation, Some(IndexGenerationId::new(2)));
    assert_eq!(specialized.candidates.len(), 1);
    assert!(matches!(
        specialized.status,
        maestria_domain::SearchLaneStatus::Succeeded
    ));
    let stale = trace
        .lanes
        .iter()
        .find(|lane| lane.retriever_id == "stale")
        .ok_or(RetrievalError::Internal("missing stale lane trace".into()))?;
    assert_eq!(stale.generation, Some(IndexGenerationId::new(2)));
    assert!(stale.candidates.is_empty());
    assert!(matches!(
        stale.status,
        maestria_domain::SearchLaneStatus::Failed { .. }
    ));
    Ok(())
}

#[tokio::test]
async fn local_lane_byte_overrun_is_rejected_before_scoring() -> RetrievalResult<()> {
    let plan = dummy_plan()?.with_budgets(maestria_domain::SearchBudget::with_resource_limits(
        1_000, 1_000, 1, 1, 0, 4, 1,
    )?)?;
    let engine = RetrievalEngine::new(
        vec![Arc::new(ByteOverrunLane {
            candidate: candidate_fixture()?,
            bytes_read: 5,
        })],
        Arc::new(AsyncEvaluator),
        maestria_governance::RetrievalSecurityPolicy::default(),
    )
    .with_capabilities(
        maestria_governance::SearchCapabilities::core_defaults(
            maestria_domain::CorpusSnapshotId::new(1),
            maestria_domain::IndexGenerationId::new(1),
            (1_000, 30_000),
        )
        .max_bytes_read(4),
    );

    let outcome = engine.search(&plan).await?;
    assert!(outcome.evidence.is_empty());
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    let lane = trace
        .lanes
        .first()
        .ok_or(RetrievalError::Internal("missing local lane trace".into()))?;
    assert!(matches!(
        &lane.status,
        maestria_domain::SearchLaneStatus::Failed { error }
            if error.contains("invalid execution metadata for text lane")
    ));
    assert_eq!(
        trace.stop_reason,
        maestria_domain::SearchStopReason::NoEvidence
    );
    Ok(())
}

#[tokio::test]
async fn web_budget_applies_across_deterministic_rewrites() -> RetrievalResult<()> {
    let plan = dummy_plan()?
        .with_budgets(maestria_domain::SearchBudget::with_resource_limits(
            1000, 1000, 8, 3, 1, 16_384, 1,
        )?)?
        .with_original_query("latest web PR".to_string())?
        .with_intent(SearchIntent::CurrentWeb)?
        .with_modalities(maestria_domain::ModalitySet::new(vec![
            maestria_domain::Modality::Web,
        ]))?;
    let calls = Arc::new(AtomicUsize::new(0));
    let engine = RetrievalEngine::new(
        vec![Arc::new(CountingWebLane {
            calls: Arc::clone(&calls),
        })],
        Arc::new(AsyncEvaluator),
        maestria_governance::RetrievalSecurityPolicy::default(),
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
