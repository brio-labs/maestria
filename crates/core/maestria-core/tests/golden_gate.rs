use std::sync::Arc;

use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, Evidence, EvidenceId, EvidenceKind, IndexGenerationId,
    IndexStatus, SearchStatus, SourceSpan, StructureNodeId,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, ChunkRepository, EvidenceRepository, FullTextIndex,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, IndexedChunk,
};
use maestria_retrieval::{
    MonotonicInstant, RetrievalEngine, SearchPlannerContext,
    adapters::{EvidenceOutcomeEvaluator, LexicalChunkRetriever, LexicalChunkRetrieverParts},
    golden::{
        GoldenCorpus, GoldenFixture, GoldenGate, GoldenGateConfig, GoldenJudgment,
        GoldenObservation, GoldenProfile, GoldenQuery, Metric, ResourceMetrics, SecurityMetrics,
    },
};

fn with_indexed_retrieval(
    f: impl FnOnce(
        &RetrievalEngine,
        &SearchPlannerContext,
        ArtifactId,
        EvidenceId,
    ) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let blobs = Arc::new(InMemoryBlobStore::new());

    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(11);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    let content_hash = maestria_core::content_hash(b"alpha-token paragraph.");

    artifacts.put(Artifact {
        id: artifact_id,
        title: "notes.md".to_owned(),
        chunk_ids: [chunk_id].into(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id].into(),
        index_status: IndexStatus::Indexed,
        content_hash: None,
        parse_status: None,
        security: Default::default(),
    })?;
    chunks.put(Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(0),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: vec![],
        order: 0,
        text: "alpha-token paragraph.".to_owned(),
    })?;
    evidence.put(Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_owned(),
            range: maestria_domain::ContentRange { start: 1, end: 1 },
            content_hash,
            snapshot: None,
        },
        excerpt: "alpha-token paragraph.".to_owned(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    })?;
    search_index.index_chunks(vec![IndexedChunk {
        artifact_id,
        chunk_id,
        text: "alpha-token paragraph.".to_owned(),
    }])?;

    let context = SearchPlannerContext {
        corpus_snapshot: maestria_domain::CorpusSnapshotId::new(1),
        primary_generation: IndexGenerationId::new(13),
        fingerprint: maestria_domain::RetrievalModelFingerprint::new(
            "golden-gate-fixture".to_owned(),
        )?,
    };

    let lexical = Arc::new(LexicalChunkRetriever::new(
        LexicalChunkRetrieverParts {
            index: search_index,
            artifacts: artifacts.clone(),
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs: blobs.clone(),
        },
        RetrievalSecurityPolicy::default(),
        context.primary_generation,
    ));
    let engine = RetrievalEngine::new(
        vec![lexical],
        Arc::new(EvidenceOutcomeEvaluator::new(evidence)),
    );
    f(&engine, &context, artifact_id, evidence_id)
}

fn gate() -> GoldenGate {
    GoldenGate {
        k: 5,
        config: GoldenGateConfig {
            profile: GoldenProfile::V0_4,
            min_recall_at_k: Metric::ONE,
            min_ndcg_at_k: Metric::ONE,
            min_mrr: Metric::ONE,
            min_exact_span_recall: Metric::ONE,
            min_material_quality_delta: Metric::ZERO,
            max_latency_ms: 1_000,
            max_memory_bytes: 1_000_000,
            max_disk_bytes: 1_000_000,
            max_ingest_update_ms: None,
            max_energy_millijoules: None,
            max_acl_leakage: 0,
            max_attack_successes: 0,
            max_privacy_violations: 0,
        },
    }
}

#[test]
fn golden_fixture_gates_a_real_core_search_trace() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    with_indexed_retrieval(|engine, context, _artifact_id, evidence_id| {
        let plan = engine.plan("alpha-token".to_string(), 5, context)?;
        let started = MonotonicInstant::now();
        let outcome = runtime.block_on(engine.search(&plan))?;
        let latency_ms = match u64::try_from(started.elapsed().as_millis()) {
            Ok(value) => value,
            Err(e) => {
                let _ = e;
                u64::MAX
            }
        };
        let trace = outcome
            .trace_data
            .as_deref()
            .ok_or_else(|| std::io::Error::other("core search did not persist trace data"))?;
        let candidate = outcome.evidence.first().ok_or_else(|| {
            std::io::Error::other("core search returned no source-grounded evidence")
        })?;
        assert_eq!(candidate.evidence_id, evidence_id);
        assert_eq!(outcome.status, SearchStatus::Answerable);

        let fixture = GoldenFixture {
            corpus: GoldenCorpus {
                schema_version: GoldenGate::CURRENT_SCHEMA_VERSION,
                corpus_snapshot: plan.corpus_snapshot,
                index_generation: plan.index_generation,
                fingerprint: plan.fingerprint.clone(),
                queries: vec![GoldenQuery {
                    query_id: plan.query_id,
                    original_query: plan.original_query.clone(),
                    expected_plan: plan,
                    expected_status: SearchStatus::Answerable,
                    judgments: vec![GoldenJudgment {
                        evidence_id: candidate.evidence_id,
                        relevance: 3,
                        exact_span: Some(candidate.source_span.clone()),
                    }],
                    expected_trace: Some(trace.clone()),
                }],
            },
            observations: vec![GoldenObservation {
                query_id: trace.query_id,
                profile: GoldenProfile::V0_4,
                outcome,
                resources: ResourceMetrics {
                    latency_ms,
                    memory_bytes: 1,
                    disk_bytes: 1,
                    ingest_update_ms: None,
                    energy_millijoules: None,
                    telemetry_complete: true,
                },
                security: SecurityMetrics::measured(),
            }],
        };

        let reports = fixture.evaluate(&gate())?;
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].recall_at_k[&5], Metric::ONE);
        assert_eq!(reports[0].mrr, Metric::ONE);
        assert_eq!(reports[0].exact_span_recall, Metric::ONE);
        Ok(())
    })
}
