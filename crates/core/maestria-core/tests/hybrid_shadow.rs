use std::sync::Arc;

use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, CorpusSnapshotId, Evidence, EvidenceKind,
    IndexGenerationId, IndexStatus, RetrievalModelFingerprint, SearchLaneStatus, SearchPlan,
    SearchRewriteOrigin, SearchStatus, SourceSpan, StructureNodeId,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EmbeddingProvider,
    EmbeddingRequest, EmbeddingResponse, EvidenceRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryChunkRepository, InMemoryEvidenceRepository, InMemoryFullTextIndex,
    InMemoryVectorIndex, PortError, ProviderDisclosure, RetentionPolicy, VectorIndex,
};
use maestria_retrieval::{
    FixedKRrf, HybridExecutionPolicy, HybridPromotionRecord, RetrievalEngine, SearchPlannerContext,
    adapters::{
        DenseChunkRetriever, DenseChunkRetrieverParts, EvidenceOutcomeEvaluator,
        LexicalChunkRetriever, LexicalChunkRetrieverParts,
    },
    traits::CandidateRetriever,
};
struct VectorFixture {
    artifacts: Arc<InMemoryArtifactRepository>,
    chunks: Arc<InMemoryChunkRepository>,
    evidence: Arc<InMemoryEvidenceRepository>,
    blobs: Arc<InMemoryBlobStore>,
    search_index: Arc<InMemoryFullTextIndex>,
    vector_index: Arc<InMemoryVectorIndex>,
}

fn seed_vector_fixture() -> Result<VectorFixture, Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(800);
    let chunk_id = ChunkId::new(801);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    let source = "literal source text\n";
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let search_index = Arc::new(InMemoryFullTextIndex::new());
    let vector_index = Arc::new(InMemoryVectorIndex::new());

    let blob_id = blobs.put(source.as_bytes().to_vec())?;
    artifacts.put(Artifact {
        id: artifact_id,
        title: "semantic.md".to_string(),
        chunk_ids: [chunk_id].into(),
        security: maestria_domain::SecurityMetadata::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [evidence_id].into(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(maestria_core::content_hash(source.as_bytes())),
        parse_status: None,
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
        text: "semantic token".to_string(),
    })?;
    evidence.put(Evidence {
        id: evidence_id,
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "semantic.md".to_string(),
            range: maestria_domain::ContentRange { start: 1, end: 1 },
            content_hash: maestria_core::content_hash(source.as_bytes()),
            snapshot: Some(blob_id),
        },
        excerpt: "literal source text".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata::default(),
    })?;
    vector_index.index_embeddings(vec![maestria_ports::VectorEmbedding {
        chunk_id,
        vector: vec![0.0, 1.0],
        provenance: maestria_ports::EmbeddingProvenance {
            content_hash: "hash".to_string(),
            identity: maestria_ports::EmbeddingIdentity::legacy("test-model", 2)?,
            provider_id: "test-provider".to_string(),
            model: "test-model".to_string(),
            model_version: "test-v1".to_string(),
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        },
    }])?;

    Ok(VectorFixture {
        artifacts,
        chunks,
        evidence,
        blobs,
        search_index,
        vector_index,
    })
}

fn planner_context() -> Result<SearchPlannerContext, Box<dyn std::error::Error>> {
    Ok(SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(1),
        primary_generation: IndexGenerationId::new(1),
        fingerprint: RetrievalModelFingerprint::new(
            "maestria-core:hybrid-shadow-fixture".to_string(),
        )?,
    })
}

struct DenseFixtureEmbeddingProvider;

impl EmbeddingProvider for DenseFixtureEmbeddingProvider {
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        Ok(EmbeddingResponse {
            vector: vec![0.0, 1.0],
            provider_id: "test-provider".to_string(),
            model: request.model,
            model_version: "test-v1".to_string(),
            identity: request.identity,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        })
    }

    fn identity(&self) -> Option<EmbeddingIdentity> {
        maestria_ports::EmbeddingIdentity::legacy("test-model", 2).ok()
    }
}

fn build_search_engine(
    policy: HybridExecutionPolicy,
    include_dense: bool,
) -> Result<(RetrievalEngine, SearchPlannerContext), Box<dyn std::error::Error>> {
    let fixture = seed_vector_fixture()?;
    let context = planner_context()?;
    let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = Vec::new();
    retrievers.push(Arc::new(LexicalChunkRetriever::new(
        LexicalChunkRetrieverParts {
            index: fixture.search_index.clone(),
            artifacts: fixture.artifacts.clone(),
            chunks: fixture.chunks.clone(),
            evidence: fixture.evidence.clone(),
            blobs: fixture.blobs.clone(),
        },
        RetrievalSecurityPolicy::default(),
        context.primary_generation,
    )));
    if include_dense {
        retrievers.push(Arc::new(DenseChunkRetriever::new(
            DenseChunkRetrieverParts {
                index: fixture.vector_index.clone(),
                artifacts: fixture.artifacts,
                chunks: fixture.chunks,
                evidence: fixture.evidence.clone(),
                blobs: fixture.blobs,
                embedding_provider: Arc::new(DenseFixtureEmbeddingProvider),
            },
            RetrievalSecurityPolicy::default(),
            context.primary_generation,
        )));
    }

    let engine = RetrievalEngine::new(
        retrievers,
        Arc::new(EvidenceOutcomeEvaluator::new(fixture.evidence)),
    )
    .with_fusion(Arc::new(FixedKRrf::new(60)))
    .with_hybrid_policy(policy);
    Ok((engine, context))
}

fn execute_search(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
) -> Result<maestria_domain::SearchOutcome, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let output = runtime.block_on(engine.search(plan))?;
    Ok(output)
}

#[test]
fn shadow_executes_dense_lane_but_suppresses_fusion() -> Result<(), Box<dyn std::error::Error>> {
    let (engine, context) = build_search_engine(HybridExecutionPolicy::Shadow, true)?;
    let plan = engine.plan("unrelated query", 5, &context)?;
    let output = execute_search(&engine, &plan)?;
    let trace = output.trace_data.as_deref().ok_or("trace data missing")?;
    let dense_report = trace
        .lanes
        .iter()
        .find(|report| report.retriever_id == "dense_chunks")
        .ok_or("dense lane report missing")?;
    assert_eq!(dense_report.status, SearchLaneStatus::Succeeded);
    assert!(!dense_report.candidates.is_empty());
    assert_eq!(output.evidence.len(), 0);
    assert_eq!(output.status, SearchStatus::NoEvidenceFound);
    Ok(())
}

#[test]
fn active_mode_serves_dense_fusion() -> Result<(), Box<dyn std::error::Error>> {
    let record = HybridPromotionRecord::new("eval-test".to_string(), "2026-07-16".to_string())
        .ok_or("promotion record must be non-empty")?;
    let (engine, context) = build_search_engine(HybridExecutionPolicy::Active(record), true)?;
    let plan = engine.plan("unrelated query", 5, &context)?;
    let output = execute_search(&engine, &plan)?;
    let trace = output.trace_data.as_deref().ok_or("trace data missing")?;
    let dense_report = trace
        .lanes
        .iter()
        .find(|report| report.retriever_id == "dense_chunks")
        .ok_or("dense lane report missing")?;
    assert_eq!(dense_report.status, SearchLaneStatus::Succeeded);
    assert!(!dense_report.candidates.is_empty());
    assert_eq!(output.evidence.len(), 1);
    assert_eq!(output.status, SearchStatus::Answerable);
    Ok(())
}

#[test]
fn knowledge_search_trace_contains_deterministic_rewrites() -> Result<(), Box<dyn std::error::Error>>
{
    let (engine, context) = build_search_engine(HybridExecutionPolicy::Shadow, false)?;
    let mut invalid_plan = engine.plan("find PR test", 5, &context)?;
    invalid_plan.original_query.clear();
    assert!(execute_search(&engine, &invalid_plan).is_err());

    let plan = engine.plan("find PR test".to_string(), 5, &context)?;
    let outcome = execute_search(&engine, &plan)?;
    let trace = outcome.trace_data.as_deref().ok_or("trace data missing")?;
    assert_eq!(trace.original_query, "find PR test");
    assert!(
        trace.expansions.is_empty(),
        "initial-only plans must not claim context expansion"
    );
    assert!(
        trace
            .rewrites
            .iter()
            .any(|rewrite| rewrite.origin == SearchRewriteOrigin::Original)
    );
    assert!(trace.rewrites.iter().any(|rewrite| {
        rewrite.origin == SearchRewriteOrigin::Deterministic
            && rewrite.query.contains("Pull Request")
    }));
    Ok(())
}
