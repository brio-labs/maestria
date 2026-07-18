use std::sync::Arc;

use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, Evidence, EvidenceId, EvidenceKind, IndexGenerationId,
    IndexStatus, SearchPlan, SearchStatus, SourceSpan, StructureNodeId, CorpusSnapshotId,
};
use maestria_domain::{RetrievalModelFingerprint};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EmbeddingProvider,
    EmbeddingRequest, EmbeddingResponse, EvidenceRepository, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryChunkRepository, InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryVectorIndex,
    PortError, ProviderDisclosure, RetentionPolicy, VectorIndex,
};
use maestria_retrieval::{
    adapters::{
        DenseChunkRetriever, DenseChunkRetrieverParts, EvidenceOutcomeEvaluator,
        LexicalChunkRetriever, LexicalChunkRetrieverParts,
    },
    traits::CandidateRetriever,
    HybridExecutionPolicy, HybridPromotionRecord, RetrievalEngine, SearchPlannerContext,
};

struct VectorFixture {
    artifacts: Arc<InMemoryArtifactRepository>,
    chunks: Arc<InMemoryChunkRepository>,
    evidence: Arc<InMemoryEvidenceRepository>,
    blobs: Arc<InMemoryBlobStore>,
    search_index: Arc<InMemoryFullTextIndex>,
    vector_index: Arc<InMemoryVectorIndex>,
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    evidence_id: EvidenceId,
}

fn seed_vector_artifact(
    context: &VectorFixture,
    source: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = context.artifact_id;
    let chunk_id = context.chunk_id;
    let evidence_id = context.evidence_id;
    let blob_id = context.blobs.put(source.as_bytes().to_vec())?;
    context.artifacts.put(Artifact {
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
    context.chunks.put(Chunk {
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
    context.evidence.put(Evidence {
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
    Ok(())
}

fn seed_vector_index(
    vector_index: &InMemoryVectorIndex,
    chunk_id: ChunkId,
) -> Result<(), Box<dyn std::error::Error>> {
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
    Ok(())
}

fn seed_vector_fixture() -> Result<VectorFixture, Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(800);
    let chunk_id = ChunkId::new(801);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    let source = "literal source text\n";

    let fixture = VectorFixture {
        artifacts: Arc::new(InMemoryArtifactRepository::new()),
        chunks: Arc::new(InMemoryChunkRepository::new()),
        evidence: Arc::new(InMemoryEvidenceRepository::new()),
        blobs: Arc::new(InMemoryBlobStore::new()),
        search_index: Arc::new(InMemoryFullTextIndex::new()),
        vector_index: Arc::new(InMemoryVectorIndex::new()),
        artifact_id,
        chunk_id,
        evidence_id,
    };

    seed_vector_artifact(&fixture, source)?;
    seed_vector_index(&fixture.vector_index, chunk_id)?;
    Ok(fixture)
}

fn planner_context() -> Result<SearchPlannerContext, Box<dyn std::error::Error>> {
    Ok(SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(1),
        primary_generation: IndexGenerationId::new(1),
        fingerprint: RetrievalModelFingerprint::new("maestria-core:hybrid-shadow-vector-fixture".to_string())?,
    })
}

struct DenseVectorFixtureEmbeddingProvider;

impl EmbeddingProvider for DenseVectorFixtureEmbeddingProvider {
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
        EmbeddingIdentity::legacy("test-model", 2).ok()
    }
}

fn build_search_engine(
    policy: HybridExecutionPolicy,
) -> Result<(RetrievalEngine, SearchPlannerContext, VectorFixture), Box<dyn std::error::Error>> {
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
    retrievers.push(Arc::new(DenseChunkRetriever::new(
        DenseChunkRetrieverParts {
            index: fixture.vector_index.clone(),
            artifacts: fixture.artifacts.clone(),
            chunks: fixture.chunks.clone(),
            evidence: fixture.evidence.clone(),
            blobs: fixture.blobs.clone(),
            embedding_provider: Arc::new(DenseVectorFixtureEmbeddingProvider),
        },
        RetrievalSecurityPolicy::default(),
        context.primary_generation,
    )));

    let engine = RetrievalEngine::new(
        retrievers,
        Arc::new(EvidenceOutcomeEvaluator::new(fixture.evidence.clone())),
    )
    .with_hybrid_policy(policy);

    Ok((engine, context, fixture))
}

fn execute_search(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
) -> Result<maestria_domain::SearchOutcome, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let outcome = runtime.block_on(engine.search(plan))?;
    Ok(outcome)
}

#[test]
fn vector_search_returns_grounded_nonliteral_match() -> Result<(), Box<dyn std::error::Error>> {
    let promotion_record = HybridPromotionRecord::new("eval-test".to_string(), "2026-07-16".to_string())
        .ok_or("promotion record requires non-empty evaluation metadata")?;
    let (engine, context, fixture) =
        build_search_engine(HybridExecutionPolicy::Active(promotion_record))?;
    let plan = engine.plan("unrelated query", 5, &context)?;
    let outcome = execute_search(&engine, &plan)?;
    assert_eq!(outcome.status, SearchStatus::Answerable);
    assert_eq!(outcome.evidence.len(), 1);
    assert_eq!(outcome.evidence[0].artifact_version.value(), fixture.artifact_id.value());
    assert_eq!(outcome.evidence[0].evidence_id, fixture.evidence_id);
    Ok(())
}
