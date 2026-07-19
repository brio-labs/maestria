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
