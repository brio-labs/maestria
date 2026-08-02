use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, ContentHash, CorpusScope, CorpusSnapshotId, Evidence,
    EvidenceKind, EvidenceRequirements, FreshnessRequirement, IndexFingerprint, IndexGeneration,
    IndexGenerationId, IndexGenerationRegistry, IndexLifecycle, IndexStatus, LineRange,
    LogicalTick, Modality, ModalitySet, QueryId, RepresentationName, RetrievalModelFingerprint,
    RetrievalReason, SearchBudget, SearchExecutionBudget, SearchIntent, SearchPlan, SearchStage,
    SnapshotRef, SourceSpan, SparseNamespace, StopConditions, StructureNodeId, TrustZone,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryChunkRepository, InMemoryEvidenceRepository,
    InMemoryLearnedSparseIndex, InMemoryLearnedSparseProvider, LearnedSparseIndex,
    LearnedSparseProvider, PortError, SPARSE_REPRESENTATION_V1, SearchQuery, SparseDocument,
    SparseFingerprint, SparseIdentity, SparseInputKind, SparseSearchQuery,
};
use maestria_retrieval::adapters::{
    LearnedSparseChunkRetriever, LearnedSparseChunkRetrieverParts,
    LearnedSparseGenerationCapability,
};
use maestria_retrieval::{CandidateRetriever, types::CandidateRequest};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingChunkRepository {
    inner: Arc<InMemoryChunkRepository>,
    owner_gets: Arc<AtomicUsize>,
    full_gets: Arc<AtomicUsize>,
    owner_error: Arc<Mutex<Option<PortError>>>,
    reported_owner: Arc<Mutex<Option<ArtifactId>>>,
}

impl CountingChunkRepository {
    fn new(inner: Arc<InMemoryChunkRepository>) -> Self {
        Self {
            inner,
            owner_gets: Arc::new(AtomicUsize::new(0)),
            full_gets: Arc::new(AtomicUsize::new(0)),
            owner_error: Arc::new(Mutex::new(None)),
            reported_owner: Arc::new(Mutex::new(None)),
        }
    }

    fn owner_gets(&self) -> usize {
        self.owner_gets.load(Ordering::SeqCst)
    }

    fn full_gets(&self) -> usize {
        self.full_gets.load(Ordering::SeqCst)
    }

    fn set_owner_error(&self, error: PortError) -> Result<(), PortError> {
        let mut guard = self
            .owner_error
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "sparse test owner error lock",
                source: "mutex is poisoned".to_string(),
            })?;
        *guard = Some(error);
        Ok(())
    }

    fn set_reported_owner(&self, artifact_id: ArtifactId) -> Result<(), PortError> {
        let mut guard = self
            .reported_owner
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "sparse test reported owner lock",
                source: "mutex is poisoned".to_string(),
            })?;
        *guard = Some(artifact_id);
        Ok(())
    }
}

impl ChunkRepository for CountingChunkRepository {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        self.full_gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(chunk_id)
    }

    fn find_artifact_id(&self, chunk_id: ChunkId) -> Result<Option<ArtifactId>, PortError> {
        self.owner_gets.fetch_add(1, Ordering::SeqCst);
        let owner_error = self
            .owner_error
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "sparse test owner error lock",
                source: "mutex is poisoned".to_string(),
            })?;
        if let Some(error) = owner_error.clone() {
            return Err(error);
        }
        drop(owner_error);
        let reported_owner =
            self.reported_owner
                .lock()
                .map_err(|_| PortError::InternalContext {
                    context: "sparse test reported owner lock",
                    source: "mutex is poisoned".to_string(),
                })?;
        if let Some(owner) = *reported_owner {
            return Ok(Some(owner));
        }
        self.inner.find_artifact_id(chunk_id)
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        self.inner.put(chunk)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        self.inner.list_for_artifact(artifact_id)
    }
}

struct CountingEvidenceRepository {
    inner: Arc<InMemoryEvidenceRepository>,
    gets: Arc<AtomicUsize>,
}

impl CountingEvidenceRepository {
    fn new(inner: Arc<InMemoryEvidenceRepository>) -> Self {
        Self {
            inner,
            gets: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn gets(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
}

impl EvidenceRepository for CountingEvidenceRepository {
    fn get(&self, evidence_id: maestria_domain::EvidenceId) -> Result<Option<Evidence>, PortError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(evidence_id)
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        self.inner.put(evidence)
    }

    fn replace(&self, evidence: Evidence) -> Result<(), PortError> {
        self.inner.replace(evidence)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        self.inner.list_for_artifact(artifact_id)
    }
}

struct OverproducingSparseIndex {
    inner: Arc<InMemoryLearnedSparseIndex>,
}

impl LearnedSparseIndex for OverproducingSparseIndex {
    fn identity(&self) -> Option<SparseIdentity> {
        self.inner.identity()
    }

    fn index_documents(&self, documents: Vec<SparseDocument>) -> Result<(), PortError> {
        self.inner.index_documents(documents)
    }

    fn search(
        &self,
        mut query: SparseSearchQuery,
    ) -> Result<maestria_ports::BoundedSearch<maestria_ports::SparseSearchHit>, PortError> {
        query.limit = u32::MAX;
        query.execution_budget = SearchExecutionBudget::new(
            u64::from(u32::MAX),
            u64::from(u32::MAX),
            u64::from(u32::MAX),
            0,
        )
        .map_err(|error| PortError::InvalidInputContext {
            context: "overproducing sparse test budget",
            source: error.to_string(),
        })?;
        self.inner.search(query)
    }

    fn search_filtered(
        &self,
        mut query: SparseSearchQuery,
        filter: &dyn Fn(ChunkId) -> Result<bool, PortError>,
    ) -> Result<maestria_ports::BoundedSearch<maestria_ports::SparseSearchHit>, PortError> {
        query.limit = u32::MAX;
        query.execution_budget = SearchExecutionBudget::new(
            u64::from(u32::MAX),
            u64::from(u32::MAX),
            u64::from(u32::MAX),
            0,
        )
        .map_err(|error| PortError::InvalidInputContext {
            context: "overproducing sparse test budget",
            source: error.to_string(),
        })?;
        self.inner.search_filtered(query, filter)
    }

    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        self.inner.delete_chunks(chunk_ids)
    }

    fn clear(&self) -> Result<(), PortError> {
        self.inner.clear()
    }
}

struct RetrieverFixture {
    identity: SparseIdentity,
    artifact_id: ArtifactId,
    retriever: LearnedSparseChunkRetriever,
    chunks: Arc<CountingChunkRepository>,
    evidence: Arc<CountingEvidenceRepository>,
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
        namespace: SparseNamespace::new(
            "fixture-instance-a",
            TrustZone::Verified,
            SPARSE_REPRESENTATION_V1,
        )?,
        fingerprint: SparseFingerprint {
            provider: "fixture-local".to_string(),
            model: "fixture-sparse".to_string(),
            revision: "v1".to_string(),
            artifact_hash: fixture_hash('1')?,
            tokenizer_hash: fixture_hash('2')?,
            vocabulary_hash: fixture_hash('3')?,
            vocabulary_size: 65_536,
            term_namespace: "fixture-vocabulary-v1".to_string(),
            query_template_hash: fixture_hash('5')?,
            document_template_hash: fixture_hash('6')?,
            preprocessing_version: "fixture-preprocess-v1".to_string(),
            weighting_version: "fixture-log-frequency-v1".to_string(),
            quantization: "f32".to_string(),
            pruning_threshold: 0.0,
            max_terms: 128,
        },
    })
}

fn fixture_registry(
    identity: &SparseIdentity,
    activate: bool,
) -> Result<IndexGenerationRegistry, Box<dyn std::error::Error>> {
    let sparse = &identity.fingerprint;
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: identity.generation_id,
        name: identity.representation.clone(),
        corpus_snapshot: identity.corpus_snapshot,
        sparse_namespace: Some(identity.namespace.clone()),
        fingerprint: IndexFingerprint {
            provider: sparse.provider.clone(),
            model: sparse.model.clone(),
            revision: sparse.revision.clone(),
            artifact_hash: sparse.artifact_hash.clone(),
            dimensions: sparse.vocabulary_size,
            quantization: sparse.quantization.clone(),
            query_template_hash: sparse.query_template_hash.as_str().to_string(),
            document_template_hash: sparse.document_template_hash.as_str().to_string(),
            preprocessing_version: sparse.preprocessing_version.clone(),
        },
        lifecycle: IndexLifecycle::Building,
    })?;
    let _previous_active =
        registry.transition_lifecycle(identity.generation_id, IndexLifecycle::Evaluated)?;
    let _previous_active =
        registry.transition_lifecycle(identity.generation_id, IndexLifecycle::Shadow)?;
    if activate {
        let _previous_active =
            registry.transition_lifecycle(identity.generation_id, IndexLifecycle::Active)?;
    }
    Ok(registry)
}

fn fixture_capability(
    identity: &SparseIdentity,
) -> Result<LearnedSparseGenerationCapability, Box<dyn std::error::Error>> {
    Ok(LearnedSparseGenerationCapability::activate(
        &fixture_registry(identity, true)?,
        identity.clone(),
    )?)
}

fn fixture_plan(
    identity: &SparseIdentity,
    query: &str,
) -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query(query.to_string())
        .intent(SearchIntent::SemanticDiscovery)
        .scope(CorpusScope::Global)
        .corpus_snapshot(identity.corpus_snapshot)
        .index_generation(identity.generation_id)
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Text]))
        .stages(vec![SearchStage::InitialRetrieval])
        .budgets(SearchBudget::with_resource_limits(
            64,
            1_000,
            1,
            1,
            0,
            1_024 * 1_024,
            1,
        )?)
        .stop_conditions(StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 1,
            minimum_documents: 1,
            minimum_sections: 1,
        })
        .fingerprint(RetrievalModelFingerprint::new(
            "fixture-search-v1".to_string(),
        )?)
        .authorization(Some(
            maestria_domain::RetrievalPolicySnapshot::global_default(),
        ))
        .build()?)
}

fn request(
    identity: &SparseIdentity,
    query: &str,
) -> Result<CandidateRequest, Box<dyn std::error::Error>> {
    request_with_limit(identity, query, 5)
}

fn request_with_limit(
    identity: &SparseIdentity,
    query: &str,
    limit: usize,
) -> Result<CandidateRequest, Box<dyn std::error::Error>> {
    let plan = fixture_plan(identity, query)?;
    let authorization = RetrievalSecurityPolicy::default().authorization_context(plan.scope())?;
    let execution_budget =
        maestria_domain::SearchExecutionBudget::new(limit as u64, limit as u64, limit as u64, 0)?;
    Ok(CandidateRequest {
        plan,
        query: SearchQuery {
            q: query.to_string(),
            limit,
            offset: 0,
            execution_budget,
        },
        execution_budget,
        expected_generation: identity.generation_id,
        authorization,
    })
}

fn fixture_with_document() -> Result<RetrieverFixture, Box<dyn std::error::Error>> {
    fixture_with_security(maestria_domain::SecurityMetadata::default())
}

fn fixture_with_security(
    security: maestria_domain::SecurityMetadata,
) -> Result<RetrieverFixture, Box<dyn std::error::Error>> {
    fixture_with_security_index(security, false)
}

fn fixture_with_security_index(
    security: maestria_domain::SecurityMetadata,
    overproduce: bool,
) -> Result<RetrieverFixture, Box<dyn std::error::Error>> {
    let identity = fixture_identity()?;
    let provider = Arc::new(InMemoryLearnedSparseProvider::new(identity.clone())?);
    let index_store = Arc::new(InMemoryLearnedSparseIndex::new(identity.clone())?);
    let index: Arc<dyn LearnedSparseIndex + Send + Sync> = if overproduce {
        Arc::new(OverproducingSparseIndex {
            inner: index_store.clone(),
        })
    } else {
        index_store.clone()
    };
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let chunk_store = Arc::new(InMemoryChunkRepository::new());
    let chunks = Arc::new(CountingChunkRepository::new(chunk_store.clone()));
    let evidence_store = Arc::new(InMemoryEvidenceRepository::new());
    let evidence = Arc::new(CountingEvidenceRepository::new(evidence_store.clone()));
    let blobs = Arc::new(InMemoryBlobStore::new());
    let artifact_id = ArtifactId::new(1);
    let chunk_id = ChunkId::new(10);
    let source = b"semantic expansion evidence".to_vec();
    let snapshot = blobs.put(source.clone())?;
    artifacts.put(fixture_artifact(
        artifact_id,
        chunk_id,
        &source,
        security.clone(),
    )?)?;
    chunk_store.put(fixture_chunk(artifact_id, chunk_id)?)?;
    evidence_store.put(fixture_evidence(
        artifact_id,
        snapshot,
        &source,
        security.clone(),
    )?)?;
    let mut documents = vec![SparseDocument {
        chunk_id,
        content_hash: fixture_hash('4')?,
        vector: provider.encode(
            "semantic expansion evidence",
            SparseInputKind::Document,
            identity.clone(),
        )?,
    }];
    if overproduce {
        let second_artifact_id = ArtifactId::new(2);
        let second_chunk_id = ChunkId::new(20);
        let second_snapshot = blobs.put(source.clone())?;
        artifacts.put(fixture_artifact(
            second_artifact_id,
            second_chunk_id,
            &source,
            security.clone(),
        )?)?;
        chunk_store.put(fixture_chunk(second_artifact_id, second_chunk_id)?)?;
        evidence_store.put(fixture_evidence(
            second_artifact_id,
            second_snapshot,
            &source,
            security,
        )?)?;
        documents.push(SparseDocument {
            chunk_id: second_chunk_id,
            content_hash: fixture_hash('7')?,
            vector: provider.encode(
                "semantic expansion evidence",
                SparseInputKind::Document,
                identity.clone(),
            )?,
        });
    }
    index_store.index_documents(documents)?;
    let retriever = LearnedSparseChunkRetriever::new(
        LearnedSparseChunkRetrieverParts {
            index,
            artifacts,
            chunks: chunks.clone(),
            evidence: evidence.clone(),
            blobs,
            provider,
        },
        fixture_capability(&identity)?,
    )?;
    Ok(RetrieverFixture {
        identity,
        artifact_id,
        retriever,
        chunks,
        evidence,
    })
}

#[tokio::test]
async fn denied_sparse_owner_reads_no_chunk_content_or_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture_with_security(maestria_domain::SecurityMetadata {
        read_allowed: false,
        ..maestria_domain::SecurityMetadata::default()
    })?;
    let batch = fixture
        .retriever
        .retrieve(request(&fixture.identity, "semantic discovery")?)
        .await?;
    assert!(batch.candidates.is_empty());
    assert_eq!(fixture.chunks.owner_gets(), 1);
    assert_eq!(fixture.chunks.full_gets(), 0);
    assert_eq!(fixture.evidence.gets(), 0);
    Ok(())
}

fn fixture_artifact(
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
    source: &[u8],
    security: maestria_domain::SecurityMetadata,
) -> Result<Artifact, Box<dyn std::error::Error>> {
    Ok(Artifact {
        id: artifact_id,
        title: "fixture".to_string(),
        chunk_ids: [chunk_id].into(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: [maestria_domain::evidence_id_for(artifact_id, 0)].into(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(maestria_domain::ContentHash::new(
            maestria_domain::content_hash(source),
        )?),
        parse_status: None,
        security,
    })
}

fn fixture_chunk(
    artifact_id: ArtifactId,
    chunk_id: ChunkId,
) -> Result<Chunk, Box<dyn std::error::Error>> {
    Ok(Chunk {
        id: chunk_id,
        artifact_id,
        node_id: StructureNodeId::new(1),
        source_span: SourceSpan::text_span(1, 1)?,
        representations: Vec::new(),
        order: 0,
        text: "semantic expansion evidence".to_string(),
    })
}

fn fixture_evidence(
    artifact_id: ArtifactId,
    snapshot: maestria_domain::BlobId,
    source: &[u8],
    security: maestria_domain::SecurityMetadata,
) -> Result<Evidence, Box<dyn std::error::Error>> {
    Ok(Evidence {
        id: maestria_domain::evidence_id_for(artifact_id, 0),
        artifact_id,
        claim_id: None,
        kind: EvidenceKind::FileSpan {
            path: "fixture.md".to_string(),
            range: LineRange::new(1, 1)?,
            snapshot: SnapshotRef::new(
                snapshot,
                ContentHash::new(maestria_domain::content_hash(source))?,
            ),
        },
        excerpt: "semantic expansion evidence".to_string(),
        observed_at: LogicalTick::new(1),
        security,
    })
}

#[tokio::test]
async fn sparse_owner_repository_errors_propagate() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture_with_document()?;
    fixture.chunks.set_owner_error(PortError::Downstream {
        message: "owner metadata unavailable".to_string(),
    })?;
    let error = match fixture
        .retriever
        .retrieve(request(&fixture.identity, "semantic discovery")?)
        .await
    {
        Ok(_) => return Err("owner lookup failure must fail retrieval".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("owner metadata unavailable"));
    assert_eq!(fixture.chunks.owner_gets(), 1);
    assert_eq!(fixture.chunks.full_gets(), 0);
    assert_eq!(fixture.evidence.gets(), 0);
    Ok(())
}

#[tokio::test]
async fn sparse_metadata_full_owner_mismatch_is_conflict() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = fixture_with_document()?;
    fixture
        .chunks
        .inner
        .put(fixture_chunk(ArtifactId::new(2), ChunkId::new(10))?)?;
    fixture.chunks.set_reported_owner(fixture.artifact_id)?;
    let error = match fixture
        .retriever
        .retrieve(request(&fixture.identity, "semantic discovery")?)
        .await
    {
        Ok(_) => return Err("owner mismatch must fail retrieval".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("conflict"));
    assert!(error.to_string().contains("owner mismatch"));
    assert_eq!(fixture.chunks.owner_gets(), 1);
    assert_eq!(fixture.chunks.full_gets(), 1);
    assert_eq!(fixture.evidence.gets(), 0);
    Ok(())
}

#[tokio::test]
async fn sparse_evidence_owner_mismatch_is_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture_with_document()?;
    let evidence_id = maestria_domain::evidence_id_for(fixture.artifact_id, 0);
    let mut evidence = fixture
        .evidence
        .inner
        .get(evidence_id)?
        .ok_or("fixture evidence missing")?;
    evidence.artifact_id = ArtifactId::new(2);
    fixture.evidence.inner.replace(evidence)?;
    let error = match fixture
        .retriever
        .retrieve(request(&fixture.identity, "semantic discovery")?)
        .await
    {
        Ok(_) => return Err("evidence owner mismatch must fail retrieval".into()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("conflict"));
    assert!(error.to_string().contains("evidence"));
    assert!(error.to_string().contains("owner mismatch"));
    assert_eq!(fixture.chunks.owner_gets(), 1);
    assert_eq!(fixture.chunks.full_gets(), 1);
    assert_eq!(fixture.evidence.gets(), 1);
    Ok(())
}

#[test]
fn sparse_generation_capability_rejects_shadow_generation() -> Result<(), Box<dyn std::error::Error>>
{
    let identity = fixture_identity()?;
    let result =
        LearnedSparseGenerationCapability::activate(&fixture_registry(&identity, false)?, identity);
    assert!(result.is_err());
    Ok(())
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
    let sparse_score = candidate
        .scores
        .lane(&maestria_domain::RetrievalScoreKind::LearnedSparse)
        .ok_or("candidate is missing its learned-sparse score")?;
    assert!(sparse_score.raw_score > 0);
    assert_eq!(sparse_score.representation.0, SPARSE_REPRESENTATION_V1);
    let Some(RetrievalReason::LearnedSparse(reason)) = candidate.reasons.first() else {
        return Err("candidate is missing learned-sparse provenance".into());
    };
    assert!(!reason.contributions.is_empty());
    assert_eq!(fixture.chunks.owner_gets(), 1);
    assert_eq!(fixture.chunks.full_gets(), 1);
    assert_eq!(fixture.evidence.gets(), 1);
    Ok(())
}

#[tokio::test]
async fn sparse_prescore_eviction_rechecks_authorized_records()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture_with_security_index(maestria_domain::SecurityMetadata::default(), true)?;
    let batch = fixture
        .retriever
        .retrieve(request_with_limit(
            &fixture.identity,
            "semantic discovery",
            1,
        )?)
        .await?;
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(fixture.chunks.owner_gets(), 3);
    assert_eq!(fixture.chunks.full_gets(), 3);
    assert_eq!(fixture.evidence.gets(), 3);
    Ok(())
}

#[tokio::test]
async fn learned_sparse_retriever_rejects_secret_queries() -> Result<(), Box<dyn std::error::Error>>
{
    let identity = fixture_identity()?;
    let provider = Arc::new(InMemoryLearnedSparseProvider::new(identity.clone())?);
    let retriever = LearnedSparseChunkRetriever::new(
        LearnedSparseChunkRetrieverParts {
            index: Arc::new(InMemoryLearnedSparseIndex::new(identity.clone())?),
            artifacts: Arc::new(InMemoryArtifactRepository::new()),
            chunks: Arc::new(InMemoryChunkRepository::new()),
            evidence: Arc::new(InMemoryEvidenceRepository::new()),
            blobs: Arc::new(InMemoryBlobStore::new()),
            provider,
        },
        fixture_capability(&identity)?,
    )?;
    let result = retriever
        .retrieve(request(&identity, "API_KEY=secret-value")?)
        .await;
    assert!(result.is_err());
    Ok(())
}
