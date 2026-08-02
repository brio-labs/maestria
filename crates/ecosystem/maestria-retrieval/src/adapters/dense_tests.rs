use super::*;
use crate::adapters::filtered_test_support::{
    CountingChunkRepository, CountingEvidenceRepository, FilteredVectorSpy, chunk, denied_artifact,
    request,
};
use maestria_domain::IndexStatus;
use maestria_ports::{
    EmbeddingProvenance, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryVectorIndex, VectorEmbedding, VectorIndex,
};

struct UnusedEmbeddingProvider;

impl EmbeddingProvider for UnusedEmbeddingProvider {
    fn disclosure(&self) -> maestria_ports::ProviderDisclosure {
        maestria_ports::ProviderDisclosure {
            remote: false,
            retention: maestria_ports::RetentionPolicy::NoRetention,
        }
    }
    fn embed(
        &self,
        _request: EmbeddingRequest,
    ) -> Result<EmbeddingResponse, maestria_ports::PortError> {
        Err(maestria_ports::PortError::Downstream {
            message: "embedding provider must not be called".to_string(),
        })
    }
}

#[test]
fn denied_dense_candidates_are_filtered_before_scoring() -> Result<(), Box<dyn std::error::Error>> {
    let generation = IndexGenerationId::new(1);
    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
    let index = Arc::new(FilteredVectorSpy::new(chunk_id));
    let artifacts = InMemoryArtifactRepository::new();
    artifacts.put(denied_artifact(artifact_id))?;
    let chunk_store = Arc::new(InMemoryChunkRepository::new());
    chunk_store.put(chunk(
        chunk_id,
        artifact_id,
        maestria_domain::SourceSpan::text_span(1, 1)?,
    ))?;
    let chunks = Arc::new(CountingChunkRepository::new(chunk_store));
    let evidence = Arc::new(CountingEvidenceRepository::new(Arc::new(
        InMemoryEvidenceRepository::new(),
    )));
    let retriever = DenseChunkRetriever::new(
        DenseChunkRetrieverParts {
            index: index.clone(),
            artifacts: Arc::new(artifacts),
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs: Arc::new(InMemoryBlobStore::new()),
            embedding_provider: Arc::new(UnusedEmbeddingProvider),
        },
        generation,
    );
    let identity = maestria_ports::contract_tests::fixture_embedding_identity("dense-test", 1)?;
    let batch = retriever.retrieve_with_vector(
        request(maestria_domain::SearchIntent::FactualLocal, generation)?,
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            identity: Some(identity.clone()),
            provider_id: None,
            model: None,
            model_version: None,
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 300, 10, 0)?,
        },
    )?;
    assert_eq!(index.filter_calls(), 1);
    assert_eq!(index.score_calls(), 0);
    assert!(batch.candidates.is_empty());
    assert_eq!(chunks.owner_gets(), 1);
    assert_eq!(chunks.full_gets(), 0);
    assert_eq!(evidence.gets(), 0);
    Ok(())
}

#[test]
fn dense_batch_reports_bounded_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let generation = IndexGenerationId::new(1);
    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
    let source = b"alpha\nbeta\n";
    let blobs = InMemoryBlobStore::new();
    let snapshot = blobs.put(source.to_vec())?;
    let content_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(source))?;
    let artifacts = InMemoryArtifactRepository::new();
    artifacts.put(maestria_domain::Artifact {
        id: artifact_id,
        title: "dense".to_string(),
        chunk_ids: std::iter::once(chunk_id).collect(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(content_hash.clone()),
        parse_status: None,
        security: Default::default(),
    })?;
    let chunk_store = Arc::new(InMemoryChunkRepository::new());
    chunk_store.put(maestria_domain::Chunk {
        id: chunk_id,
        artifact_id,
        node_id: maestria_domain::StructureNodeId::new(1),
        source_span: maestria_domain::SourceSpan::text_span(1, 2)?,
        representations: Vec::new(),
        order: 0,
        text: "alpha".to_string(),
    })?;
    let chunks = Arc::new(CountingChunkRepository::new(chunk_store));
    let evidence_store = Arc::new(InMemoryEvidenceRepository::new());
    evidence_store.put(maestria_domain::Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: maestria_domain::EvidenceKind::FileSpan {
            path: "dense.md".to_string(),
            range: maestria_domain::LineRange::new(1, 2)?,
            snapshot: maestria_domain::SnapshotRef::new(snapshot, content_hash.clone()),
        },
        excerpt: "alpha".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    })?;
    let evidence = Arc::new(CountingEvidenceRepository::new(evidence_store));
    let identity = maestria_ports::contract_tests::fixture_embedding_identity("dense-test", 1)?;
    let index = InMemoryVectorIndex::new();
    index.index_embeddings(vec![VectorEmbedding {
        chunk_id,
        vector: vec![1.0],
        provenance: EmbeddingProvenance {
            content_hash: "embedding".to_string(),
            identity: identity.clone(),
            provider_id: "dense-test".to_string(),
            model: "dense-test".to_string(),
            model_version: "1".to_string(),
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: maestria_ports::RetentionPolicy::NoRetention,
            },
        },
    }])?;
    let retriever = DenseChunkRetriever::new(
        DenseChunkRetrieverParts {
            index: Arc::new(index),
            artifacts: Arc::new(artifacts),
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs: Arc::new(blobs),
            embedding_provider: Arc::new(UnusedEmbeddingProvider),
        },
        generation,
    );
    let batch = retriever.retrieve_with_vector(
        request(maestria_domain::SearchIntent::FactualLocal, generation)?,
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            provider_id: Some("dense-test".to_string()),
            model: Some("dense-test".to_string()),
            model_version: Some("1".to_string()),
            identity: Some(identity),
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 300, 10, 0)?,
        },
    )?;
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.execution.usage.bytes_read, 4);
    assert_eq!(chunks.owner_gets(), 1);
    assert_eq!(chunks.full_gets(), 1);
    assert_eq!(evidence.gets(), 1);
    Ok(())
}
