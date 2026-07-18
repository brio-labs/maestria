use async_trait::async_trait;
use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, EvidenceCandidate,
    EvidenceCoverage, EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus,
    IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint, RetrievalReason,
    RetrievalScoreSet, ScopeId, SearchBudget, SearchIntent, SearchOutcome, SearchPlan, SearchStage,
    SearchStatus, SearchTraceId, SourceLocation, StopConditions, StructureNodeId, TrustLabel,
};
use maestria_retrieval::{
    CandidateRetriever, FixedKRrf, RetrievalEngine, RetrievalError, RetrievalEvaluator,
    RetrievalResult, SyncRetrievalEngine,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn dummy_plan() -> RetrievalResult<SearchPlan> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "test query".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(1000, 100)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 50,
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
        fingerprint: RetrievalModelFingerprint::new("dummy-model".into())?,
    })
}

fn dummy_outcome() -> RetrievalResult<SearchOutcome> {
    Ok(SearchOutcome {
        trace: SearchTraceId::new(1),
        trace_data: None,
        fingerprint: RetrievalModelFingerprint::new("dummy-model".into())?,
        index_generation: IndexGenerationId::new(1),
        status: SearchStatus::Answerable,
        evidence: vec![],
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
        conflicts: vec![],
    })
}

fn candidate_fixture() -> RetrievalResult<EvidenceCandidate> {
    Ok(EvidenceCandidate {
        coverage_keys: vec![],
        evidence_id: maestria_domain::EvidenceId::new(23),
        artifact_version: ArtifactVersionId::new(19),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(29)),
            SourceLocation::File {
                path: "notes/research.md".to_string(),
                start_line: 4,
                end_line: 8,
            },
            ContentRange { start: 32, end: 96 },
        )?,
        scores: RetrievalScoreSet {
            bm25: 91,
            semantic_similarity: 88,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(maestria_domain::DuplicateClusterId::new(31)),
        reasons: vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink],
    })
}

fn one_candidate(_: &SearchPlan) -> RetrievalResult<Vec<EvidenceCandidate>> {
    Ok(vec![candidate_fixture()?])
}
#[test]
fn test_sync_engine_orchestration() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let outcome = dummy_outcome()?;
    let retriever = |_: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> { Ok(vec![]) };
    let evaluator = move |_: Vec<EvidenceCandidate>, _: &SearchPlan| Ok(outcome.clone());
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let result = engine.search_sync(&plan)?;
    assert_eq!(result.status, SearchStatus::NoEvidenceFound);
    Ok(())
}

#[test]
fn sync_trace_preserves_rewritten_lane_queries() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.original_query = "test PR".to_string();
    plan.budgets = SearchBudget::with_limits(1000, 1_000, 4, 1, 0)?;
    let retriever = |_: &SearchPlan| Ok(Vec::<EvidenceCandidate>::new());
    let query_retriever = |_: &SearchPlan, query: &str| {
        let mut candidate = candidate_fixture()?;
        candidate.coverage_keys = vec![query.to_string()];
        Ok(vec![candidate])
    };
    let evaluator = |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.fingerprint = plan.fingerprint.clone();
        outcome.status = if outcome.evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else {
            SearchStatus::Answerable
        };
        outcome.coverage.percent_covered = if outcome.evidence.is_empty() { 0 } else { 100 };
        Ok(outcome)
    };
    let engine =
        SyncRetrievalEngine::new(vec![retriever], evaluator).with_query_retriever(query_retriever);
    let outcome = engine.search_sync(&plan)?;
    let trace = outcome
        .trace_data
        .ok_or(RetrievalError::Internal("missing search trace".into()))?;
    assert!(
        trace
            .lanes
            .iter()
            .any(|lane| lane.query == "test Pull Request")
    );
    Ok(())
}

#[test]
fn test_timeout_cancellation() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.budgets = SearchBudget::new(1000, 1)?;
    let retriever = |_: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> {
        std::thread::sleep(std::time::Duration::from_millis(10));
        Ok(vec![])
    };
    let evaluator = move |_: Vec<EvidenceCandidate>, _: &SearchPlan| dummy_outcome();
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let result = engine.search_sync(&plan);
    assert!(matches!(result, Err(RetrievalError::Timeout)));
    Ok(())
}

#[test]
fn test_fingerprint_compatibility_pass() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let retriever = |p: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> {
        if p.fingerprint.as_str() != "dummy-model" {
            return Err(RetrievalError::Internal("fingerprint mismatch".into()));
        }
        Ok(vec![])
    };
    let evaluator = move |_: Vec<EvidenceCandidate>, p: &SearchPlan| {
        let mut out = dummy_outcome()?;
        out.fingerprint = p.fingerprint.clone();
        Ok(out)
    };
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    assert!(engine.search_sync(&plan).is_ok());
    Ok(())
}

#[test]
fn test_scope_acl_filtering() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.scope = CorpusScope::Restricted(vec![ScopeId::new(1)]);
    let retriever = |p: &SearchPlan| -> RetrievalResult<Vec<EvidenceCandidate>> {
        match p.scope {
            CorpusScope::Restricted(_) => Ok(vec![]),
            _ => Err(RetrievalError::Internal("unexpected scope".into())),
        }
    };
    let evaluator = move |_: Vec<EvidenceCandidate>, _: &SearchPlan| dummy_outcome();
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    assert!(engine.search_sync(&plan).is_ok());
    Ok(())
}

#[test]
fn test_provenance_scores_reasons_and_determinism() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let candidate = candidate_fixture()?;
    let retriever = move |_: &SearchPlan| Ok(vec![candidate.clone()]);
    let evaluator = move |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.fingerprint = plan.fingerprint.clone();
        outcome.status = SearchStatus::Answerable;
        outcome.coverage.percent_covered = if outcome.evidence.is_empty() { 0 } else { 100 };
        Ok(outcome)
    };
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let first = engine.search_sync(&plan)?;
    let second = engine.search_sync(&plan)?;
    assert_eq!(first, second);
    let evidence = first.evidence.first().ok_or(RetrievalError::Internal(
        "candidate was not preserved".into(),
    ))?;
    assert_eq!(evidence.scores.bm25, 91);
    assert_eq!(
        evidence.reasons,
        vec![RetrievalReason::ExactMatch, RetrievalReason::CitationLink]
    );
    assert_eq!(
        evidence.source_span.range(),
        ContentRange { start: 32, end: 96 }
    );
    Ok(())
}

#[test]
fn test_missing_artifacts_return_no_evidence() -> RetrievalResult<()> {
    let plan = dummy_plan()?;
    let retriever = |_: &SearchPlan| Ok(Vec::<EvidenceCandidate>::new());
    let evaluator = move |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.status = SearchStatus::NoEvidenceFound;
        outcome.fingerprint = plan.fingerprint.clone();
        outcome.coverage.percent_covered = 0;
        Ok(outcome)
    };
    let engine = SyncRetrievalEngine::new(vec![retriever], evaluator);
    let outcome = engine.search_sync(&plan)?;
    assert_eq!(outcome.status, SearchStatus::NoEvidenceFound);
    assert!(outcome.evidence.is_empty());
    Ok(())
}

#[test]
fn test_sync_pipeline_bounds_multiple_retrievers() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.stop_conditions.max_results = 1;
    let evaluator = |candidates: Vec<EvidenceCandidate>, plan: &SearchPlan| {
        assert!(candidates.len() <= plan.stop_conditions.max_results as usize);
        let mut outcome = dummy_outcome()?;
        outcome.evidence = candidates;
        outcome.fingerprint = plan.fingerprint.clone();
        outcome.coverage.percent_covered = if outcome.evidence.is_empty() { 0 } else { 100 };
        Ok(outcome)
    };
    let engine = SyncRetrievalEngine::new(vec![one_candidate, one_candidate], evaluator);
    let outcome = engine.search_sync(&plan)?;
    assert_eq!(outcome.evidence.len(), 1);
    Ok(())
}

#[test]
fn test_fixed_k_rrf_overlap_non_summing() -> RetrievalResult<()> {
    use maestria_domain::{DuplicateClusterId, EvidenceId, SearchLaneStatus};
    use maestria_retrieval::FixedKRrf;
    use maestria_retrieval::traits::RankFusion;
    use maestria_retrieval::types::{CandidateBatch, RetrieverDescriptor};

    let plan = dummy_plan()?;
    let query = maestria_ports::SearchQuery {
        q: plan.original_query.clone(),
        limit: 10,
        offset: 0,
    };

    let mut c1 = candidate_fixture()?;
    c1.evidence_id = EvidenceId::new(1);
    c1.duplicate_cluster = Some(DuplicateClusterId::new(100));
    c1.scores.bm25 = 100;
    c1.scores.semantic_similarity = 0;

    let mut c2 = candidate_fixture()?;
    c2.evidence_id = EvidenceId::new(2);
    c2.duplicate_cluster = None;
    c2.scores.bm25 = 90;

    let mut c3 = candidate_fixture()?;
    c3.evidence_id = EvidenceId::new(3);
    c3.duplicate_cluster = Some(DuplicateClusterId::new(100)); // overlap with c1
    c3.scores.semantic_similarity = 80;

    let batch1 = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "lexical".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        query: query.q.clone(),
        candidates: vec![c1.clone(), c2.clone()],
        status: SearchLaneStatus::Succeeded,
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };

    let batch2 = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "dense".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        query: query.q.clone(),
        candidates: vec![c3.clone()],
        status: SearchLaneStatus::Succeeded,
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };

    let rrf = FixedKRrf::new(60);
    let fused = rrf.fuse(&query, &[batch1, batch2])?;

    assert_eq!(
        fused.len(),
        2,
        "Cluster 100 merged c1 and c3 into one candidate"
    );
    let best = &fused[0];

    // RRF of cluster 100: rank 0 in batch 1 (1/61) + rank 0 in batch 2 (1/61) = 2/61 = 0.032786
    // RRF of exact 2: rank 1 in batch 1 (1/62) = 0.016129
    let cluster = best
        .candidate
        .duplicate_cluster
        .ok_or(RetrievalError::Internal("missing duplicate cluster".into()))?;
    assert_eq!(cluster, DuplicateClusterId::new(100));
    assert!(best.fused_score > fused[1].fused_score);

    // Assert non-summing: the fused score is NOT a sum of bm25 + semantic_similarity.
    // 0.032786 * 10_000_000 = 327868
    assert_eq!(best.fused_score, 327868);
    Ok(())
}

#[test]
fn test_fixed_k_rrf_deterministic_tie_ordering() -> RetrievalResult<()> {
    use maestria_domain::{EvidenceId, SearchLaneStatus};
    use maestria_retrieval::FixedKRrf;
    use maestria_retrieval::traits::RankFusion;
    use maestria_retrieval::types::{CandidateBatch, RetrieverDescriptor};

    let query = maestria_ports::SearchQuery {
        q: "".into(),
        limit: 10,
        offset: 0,
    };

    let mut c1 = candidate_fixture()?;
    c1.evidence_id = EvidenceId::new(2);
    c1.duplicate_cluster = None;

    let mut c2 = candidate_fixture()?;
    c2.evidence_id = EvidenceId::new(1);
    c2.duplicate_cluster = None;

    // Both at rank 0 in their respective batches, identical scores, so RRF is identical.
    // Should tie-break by Identity ascending (EvidenceId 1 before 2).
    let batch1 = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "lane1".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        query: query.q.clone(),
        candidates: vec![c1.clone()],
        status: SearchLaneStatus::Succeeded,
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };
    let batch2 = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "lane2".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        candidates: vec![c2.clone()],
        query: query.q.clone(),
        status: SearchLaneStatus::Succeeded,
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };

    let rrf = FixedKRrf::new(60);
    let fused = rrf.fuse(&query, &[batch1, batch2])?;

    assert_eq!(fused.len(), 2);
    assert_eq!(fused[0].fused_score, fused[1].fused_score);
    assert_eq!(fused[1].candidate.evidence_id, EvidenceId::new(2));
    Ok(())
}

#[test]
fn test_empty_and_failed_lanes_skip_without_error() -> RetrievalResult<()> {
    use maestria_domain::{EvidenceId, SearchLaneStatus};
    use maestria_retrieval::FixedKRrf;
    use maestria_retrieval::traits::RankFusion;
    use maestria_retrieval::types::{CandidateBatch, RetrieverDescriptor};

    let query = maestria_ports::SearchQuery {
        q: "".into(),
        limit: 10,
        offset: 0,
    };

    let mut c1 = candidate_fixture()?;
    c1.evidence_id = EvidenceId::new(1);
    c1.duplicate_cluster = None;

    let batch_failed = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "failed".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        query: query.q.clone(),
        candidates: vec![],
        status: SearchLaneStatus::Failed {
            error: "timeout".into(),
        },
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };

    let batch_empty = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "empty".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        query: query.q.clone(),
        candidates: vec![],
        status: SearchLaneStatus::Empty,
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };

    let batch_succ = CandidateBatch {
        descriptor: RetrieverDescriptor {
            id: "succ".into(),
            modality: "text".into(),
            representation: maestria_domain::RepresentationName::new("text"),
            generation: maestria_domain::IndexGenerationId::new(1),
        },
        query: query.q.clone(),
        candidates: vec![c1.clone()],
        status: SearchLaneStatus::Succeeded,
        generation: Some(IndexGenerationId::new(1)),
        bytes_read: 0,
    };

    let rrf = FixedKRrf::new(60);
    let fused = rrf.fuse(&query, &[batch_failed, batch_empty, batch_succ])?;

    assert_eq!(fused[0].candidate.evidence_id, EvidenceId::new(1));
    Ok(())
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
async fn planner_accepts_context_snapshot_and_generation() -> RetrievalResult<()> {
    let context = maestria_retrieval::SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(7),
        primary_generation: IndexGenerationId::new(9),
        fingerprint: RetrievalModelFingerprint::new("contextual-model".to_string())?,
    };
    let engine = RetrievalEngine::new(
        vec![Arc::new(AsyncLane {
            id: "lexical",
            fail: false,
            candidate: None,
        })],
        Arc::new(AsyncEvaluator),
    );
    let plan = engine.plan("context snapshot", 1, &context)?;
    assert_eq!(plan.corpus_snapshot, context.corpus_snapshot);
    assert_eq!(plan.index_generation, context.primary_generation);
    let result = engine.search(&plan).await;
    assert!(result.is_ok(), "contextual search failed: {result:?}");
    Ok(())
}

#[tokio::test]
async fn web_budget_applies_across_deterministic_rewrites() -> RetrievalResult<()> {
    let mut plan = dummy_plan()?;
    plan.original_query = "latest web PR".to_string();
    plan.intent = SearchIntent::CurrentWeb;
    plan.modalities = ModalitySet::new(vec![Modality::Web]);
    plan.budgets = SearchBudget::with_resource_limits(1000, 1000, 8, 3, 1, 16_384, 1)?;
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

use maestria_domain::RerankCandidateStatus;
use maestria_retrieval::bounded_reranker::BoundedReranker;
use maestria_retrieval::traits::{CandidateReranker, RerankScorer};
use maestria_retrieval::types::{
    RankedCandidate, RerankLimits, RerankRequest, RerankScoreComponents, RerankScorerInput,
};

struct MockScorer {
    model: String,
    fingerprint: maestria_domain::RetrievalModelFingerprint,
}

#[async_trait]
impl RerankScorer for MockScorer {
    fn model(&self) -> String {
        self.model.clone()
    }
    fn fingerprint(&self) -> maestria_domain::RetrievalModelFingerprint {
        self.fingerprint.clone()
    }

    fn compatible_with(&self, _plan: &maestria_domain::RetrievalModelFingerprint) -> bool {
        true
    }
    async fn score(
        &self,
        input: RerankScorerInput,
    ) -> Result<RerankScoreComponents, RetrievalError> {
        let id_val = input.candidate.evidence_id.value();
        if id_val == 999 {
            return Err(RetrievalError::Timeout);
        }
        if id_val == 998 {
            return Err(RetrievalError::Cancelled);
        }
        Ok(RerankScoreComponents {
            relevance: (id_val * 10) as u32,
            constraints: vec![maestria_retrieval::types::RerankConstraintScore {
                name: "query".into(),
                score: (id_val * 5) as u32,
            }],
        })
    }
}

fn create_test_candidate(id: u64, rank: usize) -> RetrievalResult<RankedCandidate> {
    let mut candidate = candidate_fixture()?;
    candidate.evidence_id = maestria_domain::EvidenceId::new(id);
    Ok(RankedCandidate { candidate, rank })
}

#[tokio::test]
async fn test_bounded_reranker_limits_and_trace() -> RetrievalResult<()> {
    let scorer = Arc::new(MockScorer {
        model: "mock-scorer".into(),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("v1".to_string())?,
    });
    let limits = RerankLimits {
        input_cap: 5,
        score_cap: 3,
        output_cap: 2,
    };
    let reranker = BoundedReranker::new(scorer, limits);

    let plan = dummy_plan()?;
    let candidates = vec![
        create_test_candidate(1, 0)?,
        create_test_candidate(2, 1)?,
        create_test_candidate(3, 2)?,
        create_test_candidate(4, 3)?,
        create_test_candidate(5, 4)?,
        create_test_candidate(6, 5)?, // Exceeds input cap
    ];

    let request = RerankRequest {
        plan,
        candidates,
        max_latency_ms: 100,
    };

    let result = reranker.rerank(request).await?;

    // output cap is 2
    assert_eq!(result.candidates.len(), 2);
    // Highest scores should be id 3 (score 45) and id 2 (score 30)
    assert_eq!(result.candidates[0].candidate.evidence_id.value(), 3);
    assert_eq!(result.candidates[1].candidate.evidence_id.value(), 2);

    // Every input candidate is traced, including candidates beyond the input cap.
    let trace = result.trace;
    assert_eq!(trace.candidates.len(), 6);
    assert_eq!(trace.input_cap, 5);
    assert_eq!(trace.score_cap, 3);
    assert_eq!(trace.output_cap, 2);

    let c3 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 3)
        .ok_or(RetrievalError::Internal(
            "missing reranked candidate".into(),
        ))?;
    assert_eq!(c3.status, RerankCandidateStatus::Reranked);
    assert_eq!(c3.relevance_score, Some(30));
    assert_eq!(c3.constraint_score, Some(15));
    assert_eq!(
        c3.constraint_scores,
        vec![maestria_domain::SearchTraceConstraintScore {
            name: "query".into(),
            score: 15,
        }]
    );

    let c1 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 1)
        .ok_or(RetrievalError::Internal(
            "missing first candidate trace".into(),
        ))?;
    assert_eq!(c1.status, RerankCandidateStatus::SkippedCap);

    let c4 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 4)
        .ok_or(RetrievalError::Internal(
            "missing skipped candidate trace".into(),
        ))?;
    assert_eq!(c4.status, RerankCandidateStatus::SkippedCap); // Because score cap is 3
    let c6 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 6)
        .ok_or(RetrievalError::Internal(
            "missing capped candidate trace".into(),
        ))?;
    assert_eq!(c6.status, RerankCandidateStatus::SkippedCap);
    Ok(())
}

#[tokio::test]
async fn test_bounded_reranker_fallback() -> RetrievalResult<()> {
    let scorer = Arc::new(MockScorer {
        model: "mock-scorer".into(),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("v1".to_string())?,
    });
    let limits = RerankLimits {
        input_cap: 5,
        score_cap: 5,
        output_cap: 5,
    };
    let reranker = BoundedReranker::new(scorer, limits);

    let plan = dummy_plan()?;
    let candidates = vec![
        create_test_candidate(1, 0)?,
        create_test_candidate(999, 1)?, // Timeout
        create_test_candidate(2, 2)?,
    ];

    let request = RerankRequest {
        plan,
        candidates,
        max_latency_ms: 100,
    };

    let result = reranker.rerank(request).await?;

    // Output order: 2 (score 30), 1 (score 15), 999 (fallback score 0)
    assert_eq!(result.candidates[0].candidate.evidence_id.value(), 2);
    assert_eq!(result.candidates[1].candidate.evidence_id.value(), 1);
    assert_eq!(result.candidates[2].candidate.evidence_id.value(), 999);

    let trace = result.trace;
    let c999 = trace
        .candidates
        .iter()
        .find(|c| c.candidate_id.value() == 999)
        .ok_or(RetrievalError::Internal(
            "missing fallback candidate trace".into(),
        ))?;
    assert!(matches!(
        c999.status,
        RerankCandidateStatus::ErrorFallback(_)
    ));
    assert_eq!(c999.relevance_score, None);
    Ok(())
}

#[tokio::test]
async fn test_bounded_reranker_cancellation() -> RetrievalResult<()> {
    let scorer = Arc::new(MockScorer {
        model: "mock-scorer".into(),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new("v1".to_string())?,
    });
    let limits = RerankLimits {
        input_cap: 5,
        score_cap: 5,
        output_cap: 5,
    };
    let reranker = BoundedReranker::new(scorer, limits);

    let plan = dummy_plan()?;
    let candidates = vec![
        create_test_candidate(1, 0)?,
        create_test_candidate(998, 1)?, // Cancelled
    ];

    let request = RerankRequest {
        plan,
        candidates,
        max_latency_ms: 100,
    };

    let result = reranker.rerank(request).await;
    assert!(matches!(result, Err(RetrievalError::Cancelled)));
    Ok(())
}
