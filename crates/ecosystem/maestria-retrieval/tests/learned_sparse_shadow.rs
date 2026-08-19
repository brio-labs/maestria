use async_trait::async_trait;
use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, DuplicateClusterId,
    EvidenceCandidate, EvidenceCandidateDto, EvidenceCoverage, EvidenceCoverageDto, EvidenceId,
    EvidenceRequirements, EvidenceSpan, FreshnessRequirement, FreshnessStatus, IndexGenerationId,
    LearnedSparseContribution, LearnedSparseReason, Modality, ModalitySet, QueryId,
    RepresentationName, RetrievalModelFingerprint, RetrievalReason, RetrievalScoreSet,
    SearchBudget, SearchIntent, SearchOutcome, SearchPlan, SearchStatus, SearchTraceId,
    SourceLocation, SparseNamespace, StopConditions, TrustLabel, TrustZone,
};
use maestria_retrieval::types::{
    CandidateBatch, CandidateRequest, RetrievalError, RetrievalEvaluationReport,
    RetrievalExperiment, RetrieverDescriptor,
};
use maestria_retrieval::{
    CandidateRetriever, LearnedSparseExecutionPolicy, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowStore, LearnedSparseShadowStoreError, RetrievalEngine, RetrievalEvaluator,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

fn fixture_scores(
    bm25: u32,
    dense: u32,
) -> Result<RetrievalScoreSet, maestria_domain::SearchCompatibilityError> {
    let mut lanes = Vec::new();
    if bm25 != 0 {
        let representation = maestria_domain::RepresentationName::new("lexical_text_v1");
        lanes.push(maestria_domain::RetrievalLaneScore::new(
            maestria_domain::RetrievalScoreKind::LexicalBm25,
            i64::from(bm25),
            maestria_domain::RetrievalRawRank::ranked(1),
            maestria_domain::RetrievalScoreScale::unbounded("fixture_bm25"),
            representation.clone(),
            maestria_domain::RetrievalScoreFingerprint::new(
                maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:lexical-bm25:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        ));
    }
    if dense != 0 {
        let representation = maestria_domain::RepresentationName::new("dense_text_v1");
        lanes.push(maestria_domain::RetrievalLaneScore::new(
            maestria_domain::RetrievalScoreKind::DenseSimilarity,
            i64::from(dense),
            maestria_domain::RetrievalRawRank::ranked(1),
            maestria_domain::RetrievalScoreScale::bounded_fixed_point(
                "fixture_dense_micros",
                1_000_000,
                0,
                1_000_000,
            ),
            representation.clone(),
            maestria_domain::RetrievalScoreFingerprint::new(
                maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:dense-similarity:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        ));
    }
    RetrievalScoreSet::new(lanes)
}

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn shadow_identity() -> Option<maestria_ports::SparseIdentity> {
    let hash = maestria_test_support::content_hash(10).ok()?;
    let namespace =
        SparseNamespace::new("fixture-instance-a", TrustZone::Verified, "sparse_text_v1").ok()?;
    Some(maestria_ports::SparseIdentity {
        generation_id: IndexGenerationId::new(1),
        corpus_snapshot: CorpusSnapshotId::new(1),
        representation: RepresentationName::new("sparse_text_v1"),
        namespace,
        fingerprint: maestria_ports::SparseFingerprint {
            provider: "fixture-provider".to_string(),
            model: "fixture-model".to_string(),
            revision: "fixture-revision".to_string(),
            artifact_hash: hash.clone(),
            tokenizer_hash: hash.clone(),
            vocabulary_hash: hash.clone(),
            vocabulary_size: 1_024,
            term_namespace: "fixture-terms".to_string(),
            query_template_hash: hash.clone(),
            document_template_hash: hash,
            preprocessing_version: "v1".to_string(),
            weighting_version: "v1".to_string(),
            quantization: "fp32".to_string(),
            pruning_threshold: 0.0,
            max_terms: 16,
        },
    })
}

struct FixedRetriever {
    descriptor: RetrieverDescriptor,
    candidate: EvidenceCandidate,
}

#[async_trait]
impl CandidateRetriever for FixedRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn sparse_namespace(&self) -> Option<SparseNamespace> {
        (self.descriptor.modality == "sparse-shadow").then(|| {
            SparseNamespace::new("fixture-instance-a", TrustZone::Verified, "sparse_text_v1").ok()
        })?
    }

    fn sparse_identity(&self) -> Option<maestria_ports::SparseIdentity> {
        (self.descriptor.modality == "sparse-shadow")
            .then(shadow_identity)
            .flatten()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        Ok(CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates: vec![self.candidate.clone()],
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(self.descriptor.generation),
            execution: maestria_domain::SearchExecution::new(
                request.execution_budget,
                maestria_domain::SearchExecutionUsage::new(1, 1, 1, 1),
                maestria_domain::SearchExecutionCompletion::Complete,
            ),
        })
    }
}

struct SlowRetriever {
    descriptor: RetrieverDescriptor,
    candidate: EvidenceCandidate,
    started: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
}

#[async_trait]
impl CandidateRetriever for SlowRetriever {
    fn descriptor(&self) -> &RetrieverDescriptor {
        &self.descriptor
    }

    fn sparse_namespace(&self) -> Option<SparseNamespace> {
        (self.descriptor.modality == "sparse-shadow").then(|| {
            SparseNamespace::new("fixture-instance-a", TrustZone::Verified, "sparse_text_v1").ok()
        })?
    }

    fn sparse_identity(&self) -> Option<maestria_ports::SparseIdentity> {
        (self.descriptor.modality == "sparse-shadow")
            .then(shadow_identity)
            .flatten()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.completed.fetch_add(1, Ordering::SeqCst);
        Ok(CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates: vec![self.candidate.clone()],
            status: maestria_domain::SearchLaneStatus::Succeeded,
            generation: Some(self.descriptor.generation),
            execution: maestria_domain::SearchExecution::new(
                request.execution_budget,
                maestria_domain::SearchExecutionUsage::new(1, 1, 1, 1),
                maestria_domain::SearchExecutionCompletion::Complete,
            ),
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
                fingerprint: experiment.plan.fingerprint().clone(),
                index_generation: experiment.plan.index_generation(),
                status: SearchStatus::Answerable,
                evidence: experiment.candidates,
                coverage: EvidenceCoverage::new(EvidenceCoverageDto {
                    percent_covered: 100,
                    gaps_identified: Vec::new(),
                    required_claims: Vec::new(),
                    required_subquestions: Vec::new(),
                    distinct_sources: evaluated_candidates,
                    distinct_documents: evaluated_candidates,
                    distinct_sections: evaluated_candidates,
                    candidate_coverage_keys: Vec::new(),
                })?,
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
        SourceLocation::file("fixture.md".to_string(), 1, 1)?,
        ContentRange::new(1, 1)?,
    )?)
}

fn lexical_candidate() -> TestResult<EvidenceCandidate> {
    Ok(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(1),
        source_span: source_span()?,
        scores: fixture_scores(9_000, 0)?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(1)),
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: Vec::new(),
    })?)
}

fn sparse_candidate() -> TestResult<EvidenceCandidate> {
    Ok(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(2),
        artifact_version: ArtifactVersionId::new(2),
        source_span: source_span()?,
        scores: maestria_domain::RetrievalScoreSet::single(
            maestria_domain::RetrievalLaneScore::new(
                maestria_domain::RetrievalScoreKind::LearnedSparse,
                10_000,
                maestria_domain::RetrievalRawRank::ranked(1),
                maestria_domain::RetrievalScoreScale::fixed_point(
                    "fixture_sparse_micros",
                    1_000_000,
                ),
                RepresentationName::new("sparse_text_v1"),
                maestria_domain::RetrievalScoreFingerprint::new(
                    RetrievalModelFingerprint::new("fixture-sparse-v1".to_string())?,
                    std::collections::BTreeMap::from([(
                        "fixture".to_string(),
                        "learned_sparse_shadow".to_string(),
                    )]),
                ),
            ),
        )?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: Some(DuplicateClusterId::new(2)),
        reasons: vec![RetrievalReason::LearnedSparse(Box::new(
            LearnedSparseReason::new(vec![LearnedSparseContribution {
                term_id: 7,
                contribution_micros: 10_000,
            }]),
        ))],
        coverage_keys: Vec::new(),
    })?)
}

fn plan() -> TestResult<SearchPlan> {
    Ok(SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query("discover related concepts".to_string())
        .intent(SearchIntent::SemanticDiscovery)
        .scope(CorpusScope::Global)
        .corpus_snapshot(CorpusSnapshotId::new(1))
        .index_generation(IndexGenerationId::new(1))
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Text]))
        .stages(vec![maestria_domain::SearchStage::InitialRetrieval])
        .budgets(SearchBudget::with_resource_limits(
            64, 1_000, 1, 2, 0, 1_024, 1,
        )?)
        .stop_conditions(StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 1,
            minimum_documents: 1,
            minimum_sections: 1,
        })
        .fingerprint(RetrievalModelFingerprint::new(
            "fixture-search-v1".to_string(),
        )?)
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()?)
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
        maestria_governance::RetrievalSecurityPolicy::default(),
    )
    .with_learned_sparse_execution_policy(policy)
    .with_learned_sparse_shadow_store(store))
}

async fn populated_store() -> TestResult<LearnedSparseShadowStore> {
    let store = LearnedSparseShadowStore::new(4)?;
    let engine = engine(LearnedSparseExecutionPolicy::Shadow, store.clone())?;
    let _outcome = engine.search(&plan()?).await?;
    for _ in 0..50 {
        if !store.snapshot().is_empty() {
            return Ok(store);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    Err("shadow execution produced no observation".into())
}

#[tokio::test]
async fn shadow_sparse_observation_cannot_change_served_evidence() -> TestResult {
    let store = LearnedSparseShadowStore::new(4)?;
    let engine = engine(LearnedSparseExecutionPolicy::Shadow, store.clone())?;
    let outcome = engine.search(&plan()?).await?;

    assert_eq!(outcome.evidence.len(), 1);
    assert_eq!(outcome.evidence[0].evidence_id(), EvidenceId::new(1));
    assert!(
        outcome.evidence[0]
            .reasons()
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
    assert_eq!(lane.candidates[0].score.raw_score, 10_000);
    assert_eq!(
        lane.candidates[0].score.score_kind,
        maestria_domain::RetrievalScoreKind::LearnedSparse
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_search_aborts_shadow_provider_and_discards_observation() -> TestResult<()> {
    let store = LearnedSparseShadowStore::new(4)?;
    let started = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let engine = RetrievalEngine::new(
        vec![
            Arc::new(SlowRetriever {
                descriptor: descriptor("lexical", "text", "lexical_text_v1"),
                candidate: lexical_candidate()?,
                started: Arc::clone(&started),
                completed: Arc::clone(&completed),
            }),
            Arc::new(SlowRetriever {
                descriptor: descriptor("learned_sparse_chunks", "sparse-shadow", "sparse_text_v1"),
                candidate: sparse_candidate()?,
                started: Arc::clone(&started),
                completed: Arc::clone(&completed),
            }),
        ],
        Arc::new(PassthroughEvaluator),
        maestria_governance::RetrievalSecurityPolicy::default(),
    )
    .with_learned_sparse_execution_policy(LearnedSparseExecutionPolicy::Shadow)
    .with_learned_sparse_shadow_store(store.clone());
    let plan = plan()?;
    let search = tokio::spawn(async move { engine.search(&plan).await });
    for _ in 0..100 {
        if started.load(Ordering::SeqCst) == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    if started.load(Ordering::SeqCst) != 2 {
        search.abort();
        let _ = search.await;
        return Err("cancellation fixture did not start both providers".into());
    }
    search.abort();
    let _ = search.await;
    // Aborting the search task drops its JoinSet, which aborts the in-flight
    // retriever tasks; give a misbehaving runner scheduler turns to (wrongly)
    // complete them before asserting, without depending on wall clock.
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    assert_eq!(completed.load(Ordering::SeqCst), 0);
    assert!(store.snapshot().is_empty());
    Ok(())
}

#[tokio::test]
async fn disabled_sparse_policy_executes_no_shadow_lane() -> TestResult {
    let store = LearnedSparseShadowStore::new(4)?;
    let engine = engine(LearnedSparseExecutionPolicy::Disabled, store.clone())?;
    let _outcome = engine.search(&plan()?).await?;
    assert!(
        store.snapshot().is_empty(),
        "disabled shadow policy must not record observations"
    );
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

#[tokio::test]
async fn rejects_shadow_observation_without_sparse_namespace() -> TestResult<()> {
    let store = populated_store().await?;
    let mut value: serde_json::Value = serde_json::from_str(&store.export_json()?)?;
    value[0]["lanes"][0]["namespace"] = serde_json::Value::Null;
    let replay = LearnedSparseShadowStore::new(4)?;
    let result = replay.replace_from_json(&serde_json::to_string(&value)?);
    assert!(
        matches!(
            result.as_ref(),
            Err(LearnedSparseShadowStoreError::InvalidObservation(message))
                if message.contains("identity")
        ),
        "expected namespace rejection, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_shadow_observation_with_cross_namespace_identity() -> TestResult<()> {
    let store = populated_store().await?;
    let mut value: serde_json::Value = serde_json::from_str(&store.export_json()?)?;
    value[0]["lanes"][0]["sparse_identity"]["namespace"]["instance_id"] =
        serde_json::Value::String("fixture-instance-b".to_string());
    let replay = LearnedSparseShadowStore::new(4)?;
    let result = replay.replace_from_json(&serde_json::to_string(&value)?);
    assert!(
        matches!(
            result.as_ref(),
            Err(LearnedSparseShadowStoreError::InvalidObservation(message))
                if message.contains("identity")
        ),
        "expected cross-namespace rejection, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_shadow_observation_with_stale_generation() -> TestResult<()> {
    let store = populated_store().await?;
    let mut value: serde_json::Value = serde_json::from_str(&store.export_json()?)?;
    value[0]["index_generation"] = serde_json::Value::from(2_u64);
    let replay = LearnedSparseShadowStore::new(4)?;
    let result = replay.replace_from_json(&serde_json::to_string(&value)?);
    assert!(
        matches!(
            result.as_ref(),
            Err(LearnedSparseShadowStoreError::InvalidObservation(message))
                if message.contains("identity")
        ),
        "expected stale-generation rejection, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_shadow_observation_with_excessive_latency() -> TestResult {
    let store = populated_store().await?;
    let mut value: serde_json::Value = serde_json::from_str(&store.export_json()?)?;
    value[0]["elapsed_ms"] = serde_json::Value::from(5_001_u64);
    let replay = LearnedSparseShadowStore::new(4)?;
    let result = replay.replace_from_json(&serde_json::to_string(&value)?);
    assert!(
        matches!(
            result.as_ref(),
            Err(LearnedSparseShadowStoreError::InvalidObservation(message))
                if message.contains("latency")
        ),
        "expected bounded latency rejection, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_shadow_observation_with_inconsistent_status() -> TestResult {
    let store = populated_store().await?;
    let mut value: serde_json::Value = serde_json::from_str(&store.export_json()?)?;
    value[0]["lanes"][0]["status"] = serde_json::Value::String("Empty".to_string());
    let replay = LearnedSparseShadowStore::new(4)?;
    let result = replay.replace_from_json(&serde_json::to_string(&value)?);
    assert!(
        matches!(
            result.as_ref(),
            Err(LearnedSparseShadowStoreError::InvalidObservation(message))
                if message.contains("status")
        ),
        "expected status consistency rejection, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_shadow_observation_with_invalid_lane_rank() -> TestResult {
    let store = populated_store().await?;
    let mut value: serde_json::Value = serde_json::from_str(&store.export_json()?)?;
    value[0]["lanes"][0]["candidates"][0]["lane_rank"] = serde_json::Value::from(0_u64);
    let replay = LearnedSparseShadowStore::new(4)?;
    let result = replay.replace_from_json(&serde_json::to_string(&value)?);
    assert!(
        matches!(
            result.as_ref(),
            Err(LearnedSparseShadowStoreError::InvalidObservation(message))
                if message.contains("rank")
        ),
        "expected lane rank rejection, got {result:?}"
    );
    Ok(())
}
