use super::*;
use maestria_domain::{IndexGeneration, IndexLifecycle};
use maestria_ports::{
    EmbeddingResponse, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryVectorIndex, PortError, VisualEmbeddingRequest,
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
    let batch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            identity: Some(identity.clone()),
            provider_id: None,
            model: None,
            model_version: None,
        },
        request(maestria_domain::SearchIntent::VisualDocument, generation)?,
        &identity,
    )?;
    assert_eq!(index.filter_calls(), 1);
    assert_eq!(index.score_calls(), 0);
    assert!(batch.candidates.is_empty());
    Ok(())
}
