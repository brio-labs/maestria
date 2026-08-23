use super::*;
use crate::adapters::filtered_test_support::{
    CountingChunkRepository, CountingEvidenceRepository, request,
};
use crate::adapters::visual_access::visual_pdf_prerequisites;
use crate::adapters::visual_projection::{VisualProjectionRebuildParts, rebuild_visual_projection};
use crate::types::CandidateSourceFilter;
use maestria_domain::{EvidenceKind, IndexGeneration, IndexLifecycle, IndexStatus, SourceSpan};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    BlobStore, ChunkRepository, EmbeddingProvenance, EmbeddingResponse, EvidenceRepository,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryVectorIndex, PortError, VectorEmbedding, VectorIndex,
    VisualEmbeddingRequest,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct UnavailableVisualProvider;

impl VisualEmbeddingProvider for UnavailableVisualProvider {
    fn disclosure(&self) -> maestria_ports::ProviderDisclosure {
        maestria_ports::ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        }
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

struct DeniedVisualProvider {
    identity: EmbeddingIdentity,
    post_count: AtomicUsize,
}

impl VisualEmbeddingProvider for DeniedVisualProvider {
    fn disclosure(&self) -> maestria_ports::ProviderDisclosure {
        maestria_ports::ProviderDisclosure {
            remote: true,
            retention: maestria_ports::RetentionPolicy::ProviderDefined,
        }
    }

    fn embed_query(
        &self,
        _query: &str,
        _identity: EmbeddingIdentity,
    ) -> Result<EmbeddingResponse, PortError> {
        self.post_count.fetch_add(1, Ordering::Relaxed);
        Err(PortError::Downstream {
            message: "denied visual transport must not be called".to_string(),
        })
    }

    fn embed_source(
        &self,
        _request: VisualEmbeddingRequest,
    ) -> Result<EmbeddingResponse, PortError> {
        self.post_count.fetch_add(1, Ordering::Relaxed);
        Err(PortError::Downstream {
            message: "denied visual transport must not be called".to_string(),
        })
    }

    fn identity(&self) -> Option<EmbeddingIdentity> {
        Some(self.identity.clone())
    }
}

struct CountingBlobStore {
    gets: AtomicUsize,
}

impl BlobStore for CountingBlobStore {
    fn put(&self, _bytes: Vec<u8>) -> Result<maestria_domain::BlobId, PortError> {
        Ok(maestria_domain::BlobId::new(1))
    }

    fn get(&self, _id: maestria_domain::BlobId) -> Result<Vec<u8>, PortError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        Ok(vec![1])
    }
}

/// Shared counting fixture construction for visual-lane tests (R26): the
/// counting repos wrap a fresh in-memory backing store.
fn counting_chunk_repository() -> Arc<CountingChunkRepository> {
    Arc::new(CountingChunkRepository::new(Arc::new(
        InMemoryChunkRepository::new(),
    )))
}

fn counting_evidence_repository() -> Arc<CountingEvidenceRepository> {
    Arc::new(CountingEvidenceRepository::new(Arc::new(
        InMemoryEvidenceRepository::new(),
    )))
}

#[test]
fn visual_lane_is_named_and_generation_aware() -> Result<(), Box<dyn std::error::Error>> {
    let generation = IndexGenerationId::new(42);
    let corpus_snapshot = CorpusSnapshotId::new(7);
    let mut identity = maestria_ports::contract_tests::fixture_embedding_identity("visual", 2)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        sparse_namespace: None,
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
        capability,
    );
    let descriptor = retriever.descriptor();
    assert_eq!(descriptor.modality, "image");
    assert_eq!(descriptor.representation.0, "visual_page_v1");
    assert_eq!(descriptor.generation, generation);
    Ok(())
}

#[test]
fn denied_visual_projection_reads_no_blob_and_posts_no_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let generation = IndexGenerationId::new(84);
    let corpus_snapshot = CorpusSnapshotId::new(9);
    let mut identity = maestria_ports::contract_tests::fixture_embedding_identity("visual", 1)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        sparse_namespace: None,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    let capability =
        VisualGenerationCapability::activate(&registry, identity.clone(), corpus_snapshot)?;
    let provider = Arc::new(DeniedVisualProvider {
        identity,
        post_count: AtomicUsize::new(0),
    });
    let blobs = Arc::new(CountingBlobStore {
        gets: AtomicUsize::new(0),
    });
    let result = rebuild_visual_projection(
        VisualProjectionRebuildParts {
            index: &InMemoryVectorIndex::new(),
            artifacts: &InMemoryArtifactRepository::new(),
            chunks: &InMemoryChunkRepository::new(),
            evidence: &InMemoryEvidenceRepository::new(),
            blobs: blobs.as_ref(),
            policy: &RetrievalSecurityPolicy::default(),
            provider: provider.as_ref(),
        },
        &[maestria_domain::ArtifactId::new(1)],
        &capability,
    );
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message))
            if message.contains("local and no-retention")
    ));
    assert_eq!(provider.post_count.load(Ordering::Relaxed), 0);
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    Ok(())
}

fn setup_test_capability(
    generation: IndexGenerationId,
    corpus_snapshot: CorpusSnapshotId,
    identity: &EmbeddingIdentity,
) -> Result<VisualGenerationCapability, Box<dyn std::error::Error>> {
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        sparse_namespace: None,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    Ok(VisualGenerationCapability::activate(
        &registry,
        identity.clone(),
        corpus_snapshot,
    )?)
}

#[test]
fn denied_visual_candidates_are_authorized_before_content_reads()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::adapters::filtered_test_support::{
        FilteredVectorSpy, chunk, denied_artifact, request,
    };
    use maestria_ports::InMemoryArtifactRepository;

    let generation = IndexGenerationId::new(42);
    let corpus_snapshot = CorpusSnapshotId::new(7);
    let mut identity = maestria_ports::contract_tests::fixture_embedding_identity("visual", 1)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let capability = setup_test_capability(generation, corpus_snapshot, &identity)?;

    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
    let index = Arc::new(FilteredVectorSpy::new(chunk_id));
    let artifacts = InMemoryArtifactRepository::new();
    artifacts.put(denied_artifact(artifact_id))?;
    let chunks = counting_chunk_repository();
    chunks.put(chunk(chunk_id, artifact_id, SourceSpan::pdf_span(1)?))?;
    let evidence = counting_evidence_repository();
    let blobs = Arc::new(CountingBlobStore {
        gets: AtomicUsize::new(0),
    });
    let provider = Arc::new(DeniedVisualProvider {
        identity: identity.clone(),
        post_count: AtomicUsize::new(0),
    });
    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: index.clone(),
            artifacts: Arc::new(artifacts),
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs: blobs.clone(),
            embedding_provider: provider.clone(),
        },
        capability,
    );
    let mut mismatched_request =
        request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    mismatched_request.plan = std::sync::Arc::new(
        (*mismatched_request.plan)
            .clone()
            .with_corpus_snapshot(CorpusSnapshotId::new(8))?,
    );
    let mismatch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            identity: Some(identity.clone()),
            provider_id: None,
            model: None,
            model_version: None,
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 300, 10, 0)?,
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
    request.plan = std::sync::Arc::new(
        (*request.plan)
            .clone()
            .with_corpus_snapshot(corpus_snapshot)?,
    );
    request.source_filter = Some(CandidateSourceFilter::try_new(
        std::collections::BTreeSet::from([maestria_domain::ArtifactId::new(999)]),
    )?);
    let batch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            identity: Some(identity.clone()),
            provider_id: None,
            model: None,
            model_version: None,
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 300, 10, 0)?,
        },
        request,
        &identity,
    )?;
    assert_eq!(index.filter_calls(), 1);
    assert_eq!(index.score_calls(), 0);
    assert!(batch.candidates.is_empty());
    assert_eq!(chunks.owner_gets(), 1);
    assert_eq!(chunks.full_gets(), 0);
    assert_eq!(evidence.gets(), 0);
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    assert_eq!(provider.post_count.load(Ordering::Relaxed), 0);
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
    let mut identity = maestria_ports::contract_tests::fixture_embedding_identity("visual", 1)?;
    identity.generation_id = generation;
    identity.representation = RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        sparse_namespace: None,
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
    let bytes = b"pdf bytes".to_vec();
    let blob = blob_store.put(bytes.clone())?;
    let source_hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(&bytes))?;
    let artifacts = InMemoryArtifactRepository::new();
    artifacts.put(maestria_domain::Artifact {
        id: artifact_id,
        title: "visual".to_string(),
        chunk_ids: std::iter::once(chunk_id).collect(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(source_hash.clone()),
        parse_status: None,
        security: Default::default(),
    })?;
    let chunks = counting_chunk_repository();
    chunks.put(maestria_domain::Chunk {
        id: chunk_id,
        artifact_id,
        node_id: maestria_domain::StructureNodeId::new(1),
        source_span: SourceSpan::pdf_span(1)?,
        representations: Vec::new(),
        representations_digest: "sha256:fixture".to_string(),
        order: 0,
        text: "figure".to_string(),
    })?;
    let evidence = counting_evidence_repository();
    evidence.put(maestria_domain::Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::PdfSpan {
            snapshot: maestria_domain::SnapshotRef::new(blob, source_hash.clone()),
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
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs: Arc::new(blob_store),
            embedding_provider: Arc::new(UnavailableVisualProvider),
        },
        capability,
    );
    let mut request = request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    request.plan = std::sync::Arc::new(
        (*request.plan)
            .clone()
            .with_corpus_snapshot(corpus_snapshot)?,
    );
    let batch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            provider_id: Some("visual-test".to_string()),
            model: Some("visual-test".to_string()),
            model_version: Some("1".to_string()),
            identity: Some(identity.clone()),
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 300, 10, 0)?,
        },
        request,
        &identity,
    )?;
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.execution.usage.bytes_read, 4);
    assert_eq!(chunks.full_gets(), 1);
    assert_eq!(evidence.gets(), 1);
    Ok(())
}

#[test]
fn visual_pdf_prefilter_requires_exact_kind_and_ranges() -> Result<(), Box<dyn std::error::Error>> {
    let hash = maestria_test_support::content_hash(0)?;
    let snapshot = maestria_domain::SnapshotRef::new(maestria_domain::BlobId::new(1), hash);
    assert!(visual_pdf_prerequisites(
        &SourceSpan::pdf_span(2)?,
        &EvidenceKind::PdfSpan {
            snapshot: snapshot.clone(),
            page_start: 1,
            page_end: 3,
        },
    ));
    assert!(!visual_pdf_prerequisites(
        &SourceSpan::pdf_span(2)?,
        &EvidenceKind::PdfRegion {
            snapshot: snapshot.clone(),
            page: 2,
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        },
    ));
    assert!(visual_pdf_prerequisites(
        &SourceSpan::pdf_region(2, 1, 2, 3, 4)?,
        &EvidenceKind::PdfRegion {
            snapshot: snapshot.clone(),
            page: 2,
            x: 1,
            y: 2,
            width: 3,
            height: 4,
        },
    ));
    assert!(!visual_pdf_prerequisites(
        &SourceSpan::pdf_region(2, 1, 2, 3, 4)?,
        &EvidenceKind::PdfRegion {
            snapshot,
            page: 2,
            x: 1,
            y: 2,
            width: 4,
            height: 4,
        },
    ));
    Ok(())
}

#[test]
fn visual_evidence_owner_mismatch_is_typed_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let (generation, corpus_snapshot, identity, capability) = visual_batch_generation_fixture()?;
    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
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
        source_span: SourceSpan::pdf_span(1)?,
        representations: Vec::new(),
        representations_digest: "sha256:fixture".to_string(),
        order: 0,
        text: "figure".to_string(),
    })?;
    let evidence = InMemoryEvidenceRepository::new();
    evidence.put(maestria_domain::Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id: maestria_domain::ArtifactId::new(99),
        claim_id: None,
        kind: EvidenceKind::PdfSpan {
            snapshot: maestria_domain::SnapshotRef::new(
                maestria_domain::BlobId::new(1),
                maestria_test_support::content_hash(0)?,
            ),
            page_start: 1,
            page_end: 1,
        },
        excerpt: "figure".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    })?;
    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: Arc::new(InMemoryVectorIndex::new()),
            artifacts: Arc::new(artifacts),
            chunks: Arc::new(chunks),
            evidence: Arc::new(evidence),
            blobs: Arc::new(InMemoryBlobStore::new()),
            embedding_provider: Arc::new(UnavailableVisualProvider),
        },
        capability,
    );
    let request = request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    let result = retriever.authorized_record(chunk_id, &request.authorization);
    assert!(matches!(
        result,
        Err(RetrievalError::Internal(message)) if message.contains("visual evidence")
    ));
    let _ = (corpus_snapshot, identity);
    Ok(())
}

fn assert_visual_evidence_denied_before_score(
    evidence_record: Option<maestria_domain::Evidence>,
    chunk_text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::adapters::filtered_test_support::FilteredVectorSpy;

    let (generation, corpus_snapshot, identity, capability) = visual_batch_generation_fixture()?;
    let artifact_id = maestria_domain::ArtifactId::new(7);
    let chunk_id = maestria_domain::ChunkId::new(11);
    let index = Arc::new(FilteredVectorSpy::new(chunk_id));
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
    let chunks = counting_chunk_repository();
    chunks.put(maestria_domain::Chunk {
        id: chunk_id,
        artifact_id,
        node_id: maestria_domain::StructureNodeId::new(1),
        source_span: SourceSpan::pdf_span(1)?,
        representations: Vec::new(),
        representations_digest: "sha256:fixture".to_string(),
        order: 0,
        text: chunk_text.to_string(),
    })?;
    let evidence = counting_evidence_repository();
    if let Some(record) = evidence_record {
        evidence.put(record)?;
    }
    let blobs = Arc::new(CountingBlobStore {
        gets: AtomicUsize::new(0),
    });
    let provider = Arc::new(DeniedVisualProvider {
        identity: identity.clone(),
        post_count: AtomicUsize::new(0),
    });
    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: index.clone(),
            artifacts: Arc::new(artifacts),
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs: blobs.clone(),
            embedding_provider: provider.clone(),
        },
        capability,
    );
    let mut request = request(maestria_domain::SearchIntent::VisualDocument, generation)?;
    request.plan = std::sync::Arc::new(
        (*request.plan)
            .clone()
            .with_corpus_snapshot(corpus_snapshot)?,
    );
    let batch = retriever.retrieve_with_vector(
        VectorSearchQuery {
            vector: vec![1.0],
            limit: 5,
            provider_id: None,
            model: None,
            model_version: None,
            identity: Some(identity.clone()),
            execution_budget: maestria_domain::SearchExecutionBudget::new(5, 300, 10, 0)?,
        },
        request,
        &identity,
    )?;
    assert!(batch.candidates.is_empty());
    assert_eq!(index.filter_calls(), 1);
    assert_eq!(index.score_calls(), 0);
    assert_eq!(blobs.gets.load(Ordering::Relaxed), 0);
    assert_eq!(provider.post_count.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn visual_denied_secret_and_missing_evidence_never_score() -> Result<(), Box<dyn std::error::Error>>
{
    let artifact_id = maestria_domain::ArtifactId::new(7);
    let hash = maestria_test_support::content_hash(0)?;
    let snapshot = maestria_domain::SnapshotRef::new(maestria_domain::BlobId::new(1), hash);
    let denied = maestria_domain::Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::PdfSpan {
            snapshot: snapshot.clone(),
            page_start: 1,
            page_end: 1,
        },
        excerpt: "figure".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: maestria_domain::SecurityMetadata {
            read_allowed: false,
            ..Default::default()
        },
    };
    assert_visual_evidence_denied_before_score(Some(denied), "figure")?;
    let secret = maestria_domain::Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::PdfSpan {
            snapshot,
            page_start: 1,
            page_end: 1,
        },
        excerpt: "password=super-secret-value".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: Default::default(),
    };
    assert_visual_evidence_denied_before_score(Some(secret), "figure")?;
    assert_visual_evidence_denied_before_score(None, "figure")?;
    assert_visual_evidence_denied_before_score(None, "password=super-secret-value")?;
    Ok(())
}
