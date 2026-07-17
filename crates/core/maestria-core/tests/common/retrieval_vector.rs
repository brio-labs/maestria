use super::*;
use maestria_ports::{InMemoryVectorIndex, VectorIndex, VectorSearchQuery};

struct VectorSeedContext<'a> {
    artifact_repo: &'a InMemoryArtifactRepository,
    chunk_repo: &'a InMemoryChunkRepository,
    evidence_repo: &'a InMemoryEvidenceRepository,
    blob_store: &'a InMemoryBlobStore,
}

fn seed_vector_artifact(
    context: &VectorSeedContext<'_>,
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    evidence_id: EvidenceId,
    source: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let blob_id = context.blob_store.put(source.as_bytes().to_vec())?;
    context.artifact_repo.put(Artifact {
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
    context.chunk_repo.put(Chunk {
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
    context.evidence_repo.put(Evidence {
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

#[test]
fn vector_search_returns_grounded_nonliteral_match() -> Result<(), Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(800);
    let chunk_id = ChunkId::new(801);
    let evidence_id = maestria_domain::evidence_id_for(artifact_id, 0);
    let source = "literal source text\n";
    let artifact_repo = InMemoryArtifactRepository::new();
    let chunk_repo = InMemoryChunkRepository::new();
    let card_repo = InMemoryCardRepository::new();
    let evidence_repo = InMemoryEvidenceRepository::new();
    let blob_store = InMemoryBlobStore::new();
    let events = InMemoryEventLog::new();
    let parser = InMemoryParser::new();
    let search_index = InMemoryFullTextIndex::new();
    let vector_index = InMemoryVectorIndex::new();

    let vector_context = VectorSeedContext {
        artifact_repo: &artifact_repo,
        chunk_repo: &chunk_repo,
        evidence_repo: &evidence_repo,
        blob_store: &blob_store,
    };
    seed_vector_artifact(&vector_context, artifact_id, chunk_id, evidence_id, source)?;
    seed_vector_index(&vector_index, chunk_id)?;
    let promotion_record = maestria_core::HybridPromotionRecord::new(
        "eval-test".to_string(),
        "2026-07-16".to_string(),
    )
    .ok_or("promotion record requires non-empty evaluation metadata")?;
    let core = CoreServices::new(CorePorts {
        artifacts: &artifact_repo,
        chunks: &chunk_repo,
        cards: &card_repo,
        evidence: &evidence_repo,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blob_store,
        vector_index: Some(&vector_index),
        graph_index: None,
    })
    .with_hybrid_policy(maestria_core::HybridExecutionPolicy::Active(
        promotion_record,
    ));

    let output = core.search_with_vector(
        SearchInput {
            query: "unrelated query".to_string(),
            limit: 5,
        },
        VectorSearchQuery {
            vector: vec![0.0, 1.0],
            limit: 5,
            provider_id: Some("test-provider".to_string()),
            model: Some("test-model".to_string()),
            model_version: Some("test-v1".to_string()),
            identity: None,
        },
    )?;
    assert_eq!(output.mode, maestria_core::RetrievalMode::Hybrid);
    let pack = output.pack;
    assert_eq!(pack.chunks().len(), 1);
    assert_eq!(pack.chunks()[0].chunk.id, chunk_id);
    assert_eq!(pack.chunks()[0].evidence.id, evidence_id);
    assert_eq!(pack.evidence_ids(), &[evidence_id]);
    Ok(())
}
