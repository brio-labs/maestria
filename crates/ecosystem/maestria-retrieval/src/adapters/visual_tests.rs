use super::*;
use crate::adapters::filtered_test_support::request;
use maestria_domain::{IndexGeneration, IndexLifecycle};
use maestria_ports::{
    EmbeddingProvenance, EmbeddingResponse, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryChunkRepository, InMemoryEvidenceRepository, InMemoryVectorIndex, PortError,
    VectorEmbedding, VectorIndex, VisualEmbeddingRequest,
};

struct UnavailableVisualProvider;

impl VisualEmbeddingProvider for UnavailableVisualProvider {
    fn disclosure(&self) -> Option<maestria_ports::ProviderDisclosure> {
        Some(maestria_ports::ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        })
    }

    fn embed_query(
        &self,
        _query: &str,
        _identity: EmbeddingIdentity,
    ) -> Result<EmbeddingResponse, PortError> {
        Err(PortError::Downstream {
            message: "visual provider unavailable".to_string(),
        })
    }

    fn embed_source(
        &self,
        _request: VisualEmbeddingRequest,
    ) -> Result<EmbeddingResponse, PortError> {
        Err(PortError::Downstream {
            message: "visual provider unavailable".to_string(),
        })
    }

    fn identity(&self) -> Option<EmbeddingIdentity> {
        None
    }
}

#[test]
fn visual_lane_is_named_and_generation_aware() -> Result<(), Box<dyn std::error::Error>> {
    let generation = IndexGenerationId::new(42);
    let corpus_snapshot = CorpusSnapshotId::new(7);
    let mut identity = EmbeddingIdentity::legacy("visual", 2)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    let capability = VisualGenerationCapability::activate(&registry, identity, corpus_snapshot)?;
    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: Arc::new(InMemoryVectorIndex::new()),
            artifacts: Arc::new(InMemoryArtifactRepository::new()),
            chunks: Arc::new(InMemoryChunkRepository::new()),
            evidence: Arc::new(InMemoryEvidenceRepository::new()),
            blobs: Arc::new(InMemoryBlobStore::new()),
            embedding_provider: Arc::new(UnavailableVisualProvider),
        },
        RetrievalSecurityPolicy::default(),
        capability,
    );
    let descriptor = retriever.descriptor();
    assert_eq!(descriptor.modality, "image");
    assert_eq!(descriptor.representation.0, "visual_page_v1");
    assert_eq!(descriptor.generation, generation);
    Ok(())
}

#[test]
fn denied_visual_candidates_are_filtered_before_scoring() -> Result<(), Box<dyn std::error::Error>>
{
    use crate::adapters::filtered_test_support::{
        FilteredVectorSpy, chunk, denied_artifact, request,
    };
    use maestria_ports::{
        InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
        InMemoryEvidenceRepository,
    };

    let generation = IndexGenerationId::new(42);
    let corpus_snapshot = CorpusSnapshotId::new(7);
    let mut identity = EmbeddingIdentity::legacy("visual", 1)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    let capability =
        VisualGenerationCapability::activate(&registry, identity.clone(), corpus_snapshot)?;

    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
    let index = Arc::new(FilteredVectorSpy::new(chunk_id));
    let artifacts = InMemoryArtifactRepository::new();
    artifacts.put(denied_artifact(artifact_id))?;
    let chunks = InMemoryChunkRepository::new();
    chunks.put(chunk(
        chunk_id,
        artifact_id,
        SourceSpan::PdfSpan { page: 1 },
    ))?;
    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: index.clone(),
            artifacts: Arc::new(artifacts),
            chunks: Arc::new(chunks),
            evidence: Arc::new(InMemoryEvidenceRepository::new()),
            blobs: Arc::new(InMemoryBlobStore::new()),
            embedding_provider: Arc::new(UnavailableVisualProvider),
        },
        RetrievalSecurityPolicy::default(),
        capability,
    );
    let mut mismatched_request =
        request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    mismatched_request.plan.corpus_snapshot = CorpusSnapshotId::new(8);
    let mismatch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            identity: Some(identity.clone()),
            provider_id: None,
            model: None,
            model_version: None,
        },
        mismatched_request,
        &identity,
    );
    assert!(matches!(
        mismatch,
        Err(RetrievalError::Internal(message))
            if message.contains("visual corpus snapshot mismatch")
    ));
    assert_eq!(index.filter_calls(), 0);
    let mut request = request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    request.plan.corpus_snapshot = corpus_snapshot;
    let batch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            identity: Some(identity.clone()),
            provider_id: None,
            model: None,
            model_version: None,
        },
        request,
        &identity,
    )?;
    assert_eq!(index.filter_calls(), 1);
    assert_eq!(index.score_calls(), 0);
    assert!(batch.candidates.is_empty());
    Ok(())
}

fn visual_batch_generation_fixture() -> Result<
    (
        IndexGenerationId,
        CorpusSnapshotId,
        EmbeddingIdentity,
        VisualGenerationCapability,
    ),
    Box<dyn std::error::Error>,
> {
    let generation = IndexGenerationId::new(42);
    let corpus_snapshot = CorpusSnapshotId::new(7);
    let mut identity = EmbeddingIdentity::legacy("visual", 1)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    let capability =
        VisualGenerationCapability::activate(&registry, identity.clone(), corpus_snapshot)?;
    Ok((generation, corpus_snapshot, identity, capability))
}

#[test]
fn visual_batch_reports_bounded_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let (generation, corpus_snapshot, identity, capability) = visual_batch_generation_fixture()?;

    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
    let blob_store = InMemoryBlobStore::new();
    let blob = blob_store.put(b"pdf bytes".to_vec())?;
    let artifacts = InMemoryArtifactRepository::new();
    artifacts.put(maestria_domain::Artifact {
        id: artifact_id,
        title: "visual".to_string(),
        chunk_ids: std::iter::once(chunk_id).collect(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: IndexStatus::Indexed,
        content_hash: None,
        parse_status: None,
        security: Default::default(),
    })?;
    let chunks = InMemoryChunkRepository::new();
    chunks.put(maestria_domain::Chunk {
        id: chunk_id,
        artifact_id,
        node_id: maestria_domain::StructureNodeId::new(1),
        source_span: SourceSpan::PdfSpan { page: 1 },
        representations: Vec::new(),
        order: 0,
        text: "figure".to_string(),
    })?;
    let evidence = InMemoryEvidenceRepository::new();
    evidence.put(maestria_domain::Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::PdfSpan {
            blob,
            page_start: 1,
            page_end: 1,
        },
        excerpt: "figure".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    })?;
    let index = InMemoryVectorIndex::new();
    index.index_embeddings(vec![VectorEmbedding {
        chunk_id,
        vector: vec![1.0],
        provenance: EmbeddingProvenance {
            content_hash: "embedding".to_string(),
            identity: identity.clone(),
            provider_id: "visual-test".to_string(),
            model: "visual-test".to_string(),
            model_version: "1".to_string(),
            disclosure: maestria_ports::ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        },
    }])?;
    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: Arc::new(index),
            artifacts: Arc::new(artifacts),
            chunks: Arc::new(chunks),
            evidence: Arc::new(evidence),
            blobs: Arc::new(blob_store),
            embedding_provider: Arc::new(UnavailableVisualProvider),
        },
        RetrievalSecurityPolicy::default(),
        capability,
    );
    let mut request = request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    request.plan.corpus_snapshot = corpus_snapshot;
    let batch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            provider_id: Some("visual-test".to_string()),
            model: Some("visual-test".to_string()),
            model_version: Some("1".to_string()),
            identity: Some(identity.clone()),
        },
        request,
        &identity,
    )?;
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.bytes_read, 1);
    Ok(())
}
