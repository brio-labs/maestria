use std::sync::Arc;

use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, ContentHash, ContentRange, CorpusScope, CorpusSnapshotId,
    Evidence, EvidenceKind, EvidenceRequirements, FreshnessRequirement, IndexGenerationId,
    IndexStatus, LearnedSparseReason, LogicalTick, Modality, ModalitySet, QueryId,
    RepresentationName, RetrievalModelFingerprint, RetrievalReason, SearchBudget, SearchIntent,
    SearchPlan, SearchStage, SourceSpan, StopConditions, StructureNodeId,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryChunkRepository, InMemoryEvidenceRepository,
    InMemoryLearnedSparseIndex, InMemoryLearnedSparseProvider, LearnedSparseIndex,
    LearnedSparseProvider, SPARSE_REPRESENTATION_V1, SearchQuery, SparseDocument,
    SparseFingerprint, SparseIdentity, SparseInputKind,
};
use maestria_retrieval::adapters::{LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts};
use maestria_retrieval::{CandidateRetriever, types::CandidateRequest};

struct RetrieverFixture {
    identity: SparseIdentity,
    artifact_id: ArtifactId,
    retriever: LearnedSparseChunkRetriever,
}

fn fixture_hash(digit: char) -> Result<ContentHash, Box<dyn std::error::Error>> {
    Ok(ContentHash::new(format!(
        "sha256:{}",
        digit.to_string().repeat(64)
    ))?)
}

fn fixture_identity() -> Result<SparseIdentity, Box<dyn std::error::Error>> {
    Ok(SparseIdentity {
        generation_id: IndexGenerationId::new(7),
        corpus_snapshot: CorpusSnapshotId::new(11),
        representation: RepresentationName::new(SPARSE_REPRESENTATION_V1),
        fingerprint: SparseFingerprint {
            provider: "fixture-local".to_string(),
            model: "fixture-sparse".to_string(),
            revision: "v1".to_string(),
            artifact_hash: fixture_hash('1')?,
            tokenizer_hash: fixture_hash('2')?,
            vocabulary_hash: fixture_hash('3')?,
            vocabulary_size: 65_536,
            term_namespace: "fixture-vocabulary-v1".to_string(),
            query_template_hash: "sha256:query-template".to_string(),
            document_template_hash: "sha256:document-template".to_string(),
            preprocessing_version: "fixture-preprocess-v1".to_string(),
            weighting_version: "fixture-log-frequency-v1".to_string(),
            quantization: "f32".to_string(),
            pruning_threshold: 0.0,
            max_terms: 128,
        },
    })
}

fn fixture_plan(
    identity: &SparseIdentity,
    query: &str,
) -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: query.to_string(),
        intent: SearchIntent::SemanticDiscovery,
        scope: CorpusScope::Global,
        corpus_snapshot: identity.corpus_snapshot,
        index_generation: identity.generation_id,
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::with_resource_limits(64, 1_000, 1, 1, 0, 1_024 * 1_024, 1)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 1,
            minimum_documents: 1,
            minimum_sections: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("fixture-search-v1".to_string())?,
        original_intent: None,
        route_decision: None,
    })
}

fn request(
    identity: &SparseIdentity,
    query: &str,
) -> Result<CandidateRequest, Box<dyn std::error::Error>> {
    Ok(CandidateRequest {
        plan: fixture_plan(identity, query)?,
        query: SearchQuery {
            q: query.to_string(),
            limit: 5,
            offset: 0,
        },
        expected_generation: identity.generation_id,
    })
}

fn fixture_with_document() -> Result<RetrieverFixture, Box<dyn std::error::Error>> {
    let identity = fixture_identity()?;
    let provider = Arc::new(InMemoryLearnedSparseProvider::new(identity.clone())?);
    let index = Arc::new(InMemoryLearnedSparseIndex::new());
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let source = b"semantic expansion evidence".to_vec();
    let snapshot = blobs.put(source.clone())?;
    let security = maestria_domain::SecurityMetadata::default();
    artifacts.put(fixture_artifact(
        artifact_id,
        chunk_id,
        &source,
        security.clone(),
    ))?;
    chunks.put(fixture_chunk(artifact_id, chunk_id))?;
    evidence.put(fixture_evidence(artifact_id, snapshot, &source, security))?;
    index.index_documents(vec![SparseDocument {
        chunk_id,
        content_hash: fixture_hash('4')?,
        vector: provider.encode(
            "semantic expansion evidence",
            SparseInputKind::Document,
            identity.clone(),
        )?,
    }])?;
    let retriever = LearnedSparseChunkRetriever::new(
        LearnedSparseChunkRetrieverParts {
            index,
            artifacts,
            chunks,
            evidence,
            blobs,
            provider,
        },
        RetrievalSecurityPolicy::new()
            .require_read_allowed(true)
            .allow_unscoped_items(true),
        identity.clone(),
    )?;
    Ok(RetrieverFixture {
        identity,
        artifact_id,
        retriever,
    })
}

fn fixture_artifact(
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    source: &[u8],
    security: maestria_domain::SecurityMetadata,
) -> Artifact {
    Artifact {
        id: artifact_id,
        title: "fixture".to_string(),
        chunk_ids: [chunk_id].into(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [maestria_domain::evidence_id_for(artifact_id, 0)].into(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(maestria_domain::content_hash(source)),
        parse_status: None,
        security,
    }
}

fn fixture_chunk(artifact_id: ArtifactId, chunk_id: ChunkId) -> Chunk {
    Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(1),
        source_span: SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1,
        },
        representations: Vec::new(),
        order: 0,
        text: "semantic expansion evidence".to_string(),
    }
}

fn fixture_evidence(
    artifact_id: ArtifactId,
    snapshot: maestria_domain::BlobId,
    source: &[u8],
    security: maestria_domain::SecurityMetadata,
) -> Evidence {
    Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "fixture.md".to_string(),
            range: ContentRange { start: 1, end: 1 },
            content_hash: maestria_domain::content_hash(source),
            snapshot: Some(snapshot),
        },
        excerpt: "semantic expansion evidence".to_string(),
        observed_at: LogicalTick::new(1),
        security,
    }
}

#[tokio::test]
async fn learned_sparse_retriever_preserves_score_and_source_lineage()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture_with_document()?;
    let batch = fixture
        .retriever
        .retrieve(request(&fixture.identity, "semantic discovery")?)
        .await?;
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    assert_eq!(
        candidate.evidence_id,
        maestria_domain::evidence_id_for(fixture.artifact_id, 0)
    );
    assert_eq!(candidate.scores.bm25, 0);
    assert_eq!(candidate.scores.semantic_similarity, 0);
    let Some(RetrievalReason::LearnedSparse(reason)) = candidate.reasons.first() else {
        return Err("candidate is missing learned-sparse provenance".into());
    };
    let LearnedSparseReason {
        score_micros,
        representation,
        contributions,
        ..
    } = reason.as_ref();
    assert!(*score_micros > 0);
    assert_eq!(representation.0, SPARSE_REPRESENTATION_V1);
    assert!(!contributions.is_empty());
    Ok(())
}

#[tokio::test]
async fn learned_sparse_retriever_rejects_secret_queries() -> Result<(), Box<dyn std::error::Error>>
{
    let identity = fixture_identity()?;
    let provider = Arc::new(InMemoryLearnedSparseProvider::new(identity.clone())?);
    let retriever = LearnedSparseChunkRetriever::new(
        LearnedSparseChunkRetrieverParts {
            index: Arc::new(InMemoryLearnedSparseIndex::new()),
            artifacts: Arc::new(InMemoryArtifactRepository::new()),
            chunks: Arc::new(InMemoryChunkRepository::new()),
            evidence: Arc::new(InMemoryEvidenceRepository::new()),
            blobs: Arc::new(InMemoryBlobStore::new()),
            provider,
        },
        RetrievalSecurityPolicy::new().allow_unscoped_items(true),
        identity.clone(),
    )?;
    let result = retriever
        .retrieve(request(&identity, "API_KEY=secret-value")?)
        .await;
    assert!(result.is_err());
    Ok(())
}
