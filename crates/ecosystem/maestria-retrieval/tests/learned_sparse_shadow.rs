use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, DuplicateClusterId,
    EvidenceCandidate, EvidenceCoverage, EvidenceId, EvidenceRequirements, EvidenceSpan,
    FreshnessRequirement, FreshnessStatus, IndexGenerationId, LearnedSparseContribution,
    LearnedSparseReason, Modality, ModalitySet, QueryId, RepresentationName,
    RetrievalModelFingerprint, RetrievalReason, RetrievalScoreSet, SearchBudget, SearchIntent,
    SearchOutcome, SearchPlan, SearchStatus, SearchTraceId, SourceLocation, StopConditions,
    TrustLabel,
};
use maestria_retrieval::types::{
    CandidateBatch, CandidateRequest, RetrievalError, RetrievalEvaluationReport,
    RetrievalExperiment, RetrieverDescriptor,
};
use maestria_retrieval::{
    CandidateRetriever, LearnedSparseExecutionPolicy, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowStore, RetrievalEngine, RetrievalEvaluator,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct FixedRetriever {
    descriptor: RetrieverDescriptor,
    candidate: EvidenceCandidate,
}

#[async_trait]
impl CandidateRetriever for FixedRetriever {
    fn descriptor(&self) -> RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        Ok(CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates: vec![self.candidate.clone()],
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(self.descriptor.generation),
            bytes_read: 1,
        })
    }
}

struct PassthroughEvaluator;

#[async_trait]
impl RetrievalEvaluator for PassthroughEvaluator {
    async fn evaluate(
        &self,
        experiment: RetrievalExperiment,
    ) -> Result<RetrievalEvaluationReport, RetrievalError> {
        let evaluated_candidates = experiment.candidates.len();
        Ok(RetrievalEvaluationReport {
            outcome: SearchOutcome {
                trace: SearchTraceId::new(1),
                trace_data: None,
                fingerprint: experiment.plan.fingerprint.clone(),
                index_generation: experiment.plan.index_generation,
                status: SearchStatus::Answerable,
                evidence: experiment.candidates,
                coverage: EvidenceCoverage {
                    percent_covered: 100,
                    gaps_identified: Vec::new(),
                    required_claims: Vec::new(),
                    required_subquestions: Vec::new(),
                    distinct_sources: evaluated_candidates,
                    distinct_documents: evaluated_candidates,
                    distinct_sections: evaluated_candidates,
                    candidate_coverage_keys: Vec::new(),
                },
                conflicts: Vec::new(),
            },
            evaluated_candidates,
        })
    }
}

fn descriptor(id: &str, modality: &str, representation: &str) -> RetrieverDescriptor {
    RetrieverDescriptor {
        id: id.to_string(),
        modality: modality.to_string(),
        representation: RepresentationName::new(representation),
        generation: IndexGenerationId::new(1),
    }
}

fn source_span() -> TestResult<EvidenceSpan> {
    Ok(EvidenceSpan::new(
        None,
        SourceLocation::File {
            path: "fixture.md".to_string(),
            start_line: 1,
            end_line: 1,
        },
        ContentRange { start: 1, end: 1 },
    )?)
}

fn lexical_candidate() -> TestResult<EvidenceCandidate> {
    Ok(EvidenceCandidate {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(1),
        source_span: source_span()?,
        scores: RetrievalScoreSet {
            bm25: 9_000,
            semantic_similarity: 0,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(1)),
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: Vec::new(),
    })
}

fn sparse_candidate() -> TestResult<EvidenceCandidate> {
    Ok(EvidenceCandidate {
        evidence_id: EvidenceId::new(2),
        artifact_version: ArtifactVersionId::new(2),
        source_span: source_span()?,
        scores: RetrievalScoreSet {
            bm25: 0,
            semantic_similarity: 0,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(2)),
        reasons: vec![RetrievalReason::LearnedSparse(Box::new(
            LearnedSparseReason {
                score_micros: 10_000,
                representation: RepresentationName::new("sparse_text_v1"),
                fingerprint: RetrievalModelFingerprint::new("fixture-sparse-v1".to_string())?,
                contributions: vec![LearnedSparseContribution {
                    term_id: 7,
                    contribution_micros: 10_000,
                }],
            },
        ))],
        coverage_keys: Vec::new(),
    })
}

fn plan() -> TestResult<SearchPlan> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "discover related concepts".to_string(),
        intent: SearchIntent::SemanticDiscovery,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![maestria_domain::SearchStage::InitialRetrieval],
        budgets: SearchBudget::with_resource_limits(64, 1_000, 1, 2, 0, 1_024, 1)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 1,
            minimum_documents: 1,
            minimum_sections: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("fixture-search-v1".to_string())?,
        original_intent: None,
        route_decision: None,
    })
}

fn engine(
    policy: LearnedSparseExecutionPolicy,
    store: LearnedSparseShadowStore,
) -> TestResult<RetrievalEngine> {
    Ok(RetrievalEngine::new(
        vec![
            Arc::new(FixedRetriever {
                descriptor: descriptor("lexical", "text", "lexical_text_v1"),
                candidate: lexical_candidate()?,
            }),
            Arc::new(FixedRetriever {
                descriptor: descriptor("learned_sparse_chunks", "sparse-shadow", "sparse_text_v1"),
                candidate: sparse_candidate()?,
            }),
        ],
        Arc::new(PassthroughEvaluator),
    )
    .with_learned_sparse_execution_policy(policy)
    .with_learned_sparse_shadow_store(store))
}

#[tokio::test]
async fn shadow_sparse_observation_cannot_change_served_evidence() -> TestResult {
    let store = LearnedSparseShadowStore::new(4)?;
    let engine = engine(LearnedSparseExecutionPolicy::Shadow, store.clone())?;
    let outcome = engine.search(&plan()?).await?;

    assert_eq!(outcome.evidence.len(), 1);
    assert_eq!(outcome.evidence[0].evidence_id, EvidenceId::new(1));
    assert!(
        outcome.evidence[0]
            .reasons
            .iter()
            .all(|reason| !matches!(reason, RetrievalReason::LearnedSparse(_)))
    );

    let mut observations = Vec::new();
    for _ in 0..50 {
        observations = store.snapshot();
        if !observations.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let Some(observation) = observations.first() else {
        return Err("shadow execution produced no observation".into());
    };
    let Some(lane) = observation.lanes.first() else {
        return Err("shadow observation contains no lane".into());
    };
    assert_eq!(lane.status, LearnedSparseShadowLaneStatus::Succeeded);
    assert_eq!(lane.candidates.len(), 1);
    assert_eq!(lane.candidates[0].evidence_id, EvidenceId::new(2));
    assert_eq!(lane.candidates[0].reason.score_micros, 10_000);
    Ok(())
}

#[tokio::test]
async fn disabled_sparse_policy_executes_no_shadow_lane() -> TestResult {
    let store = LearnedSparseShadowStore::new(4)?;
    let engine = engine(LearnedSparseExecutionPolicy::Disabled, store.clone())?;
    let _outcome = engine.search(&plan()?).await?;
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(store.snapshot().is_empty());
    Ok(())
}

#[test]
fn shadow_observations_round_trip_through_bounded_json() -> TestResult {
    let store = LearnedSparseShadowStore::new(4)?;
    let empty = store.export_json()?;
    let replay = LearnedSparseShadowStore::new(4)?;
    replay.replace_from_json(&empty)?;
    assert!(replay.snapshot().is_empty());
    Ok(())
}
