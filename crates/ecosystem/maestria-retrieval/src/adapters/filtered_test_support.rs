use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, CorpusScope, EvidenceRequirements, FreshnessRequirement,
    IndexGenerationId, IndexStatus, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    SearchBudget, SearchCompatibilityError, SearchIntent, SearchPlan, SearchStage, SourceSpan,
    StopConditions,
};
use maestria_ports::FullTextIndex;
use maestria_ports::{
    CardHit, ChunkRepository, EvidenceRepository, InMemoryChunkRepository,
    InMemoryEvidenceRepository, IndexedCard, IndexedChunk, PortError, SearchHit, SearchQuery,
    VectorEmbedding, VectorIndex, VectorSearchHit, VectorSearchQuery,
};

pub struct FilteredFullTextSpy {
    chunk_filter_calls: AtomicUsize,
    card_filter_calls: AtomicUsize,
    chunk_score_calls: AtomicUsize,
    card_score_calls: AtomicUsize,
    chunk_id: ChunkId,
    card_id: maestria_domain::CardId,
    artifact_id: ArtifactId,
}

impl FilteredFullTextSpy {
    pub fn new(
        chunk_id: ChunkId,
        card_id: maestria_domain::CardId,
        artifact_id: ArtifactId,
    ) -> Self {
        Self {
            chunk_filter_calls: AtomicUsize::new(0),
            card_filter_calls: AtomicUsize::new(0),
            chunk_score_calls: AtomicUsize::new(0),
            card_score_calls: AtomicUsize::new(0),
            chunk_id,
            card_id,
            artifact_id,
        }
    }

    pub fn chunk_filter_calls(&self) -> usize {
        self.chunk_filter_calls.load(Ordering::SeqCst)
    }

    pub fn card_filter_calls(&self) -> usize {
        self.card_filter_calls.load(Ordering::SeqCst)
    }

    pub fn chunk_score_calls(&self) -> usize {
        self.chunk_score_calls.load(Ordering::SeqCst)
    }

    pub fn card_score_calls(&self) -> usize {
        self.card_score_calls.load(Ordering::SeqCst)
    }
}

impl FullTextIndex for FilteredFullTextSpy {
    fn index_chunks(&self, _chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        Ok(())
    }

    fn search(&self, _query: SearchQuery) -> Result<Vec<SearchHit>, PortError> {
        Err(PortError::InternalContext {
            context: "test unfiltered chunk search",
            source: "governed adapter bypassed filtered search".to_string(),
        })
    }

    fn index_cards(&self, _cards: Vec<IndexedCard>) -> Result<(), PortError> {
        Ok(())
    }

    fn search_cards(&self, _query: SearchQuery) -> Result<Vec<CardHit>, PortError> {
        Err(PortError::InternalContext {
            context: "test unfiltered card search",
            source: "governed adapter bypassed filtered search".to_string(),
        })
    }

    fn search_filtered(
        &self,
        _query: SearchQuery,
        filter: &dyn Fn(ChunkId, ArtifactId) -> bool,
    ) -> Result<Vec<SearchHit>, PortError> {
        self.chunk_filter_calls.fetch_add(1, Ordering::SeqCst);
        if filter(self.chunk_id, self.artifact_id) {
            self.chunk_score_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Vec::new())
    }

    fn search_cards_filtered(
        &self,
        _query: SearchQuery,
        filter: &dyn Fn(maestria_domain::CardId, ArtifactId) -> bool,
    ) -> Result<Vec<CardHit>, PortError> {
        self.card_filter_calls.fetch_add(1, Ordering::SeqCst);
        if filter(self.card_id, self.artifact_id) {
            self.card_score_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Vec::new())
    }
}

pub struct FilteredVectorSpy {
    filter_calls: AtomicUsize,
    score_calls: AtomicUsize,
    chunk_id: ChunkId,
}

impl FilteredVectorSpy {
    pub fn new(chunk_id: ChunkId) -> Self {
        Self {
            filter_calls: AtomicUsize::new(0),
            score_calls: AtomicUsize::new(0),
            chunk_id,
        }
    }

    pub fn filter_calls(&self) -> usize {
        self.filter_calls.load(Ordering::SeqCst)
    }

    pub fn score_calls(&self) -> usize {
        self.score_calls.load(Ordering::SeqCst)
    }
}

impl VectorIndex for FilteredVectorSpy {
    fn index_embeddings(&self, _embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        Ok(())
    }

    fn search_similar(&self, _query: VectorSearchQuery) -> Result<Vec<VectorSearchHit>, PortError> {
        Err(PortError::InternalContext {
            context: "test unfiltered vector search",
            source: "governed adapter bypassed filtered search".to_string(),
        })
    }

    fn search_similar_filtered(
        &self,
        _query: VectorSearchQuery,
        filter: &dyn Fn(ChunkId) -> bool,
    ) -> Result<Vec<VectorSearchHit>, PortError> {
        self.filter_calls.fetch_add(1, Ordering::SeqCst);
        if filter(self.chunk_id) {
            self.score_calls.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Vec::new())
    }

    fn delete_chunks(&self, _chunk_ids: &[ChunkId]) -> Result<(), PortError> {
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        Ok(())
    }
}

pub struct CountingChunkRepository {
    inner: Arc<InMemoryChunkRepository>,
    owner_gets: Arc<AtomicUsize>,
    full_gets: Arc<AtomicUsize>,
}

impl CountingChunkRepository {
    pub fn new(inner: Arc<InMemoryChunkRepository>) -> Self {
        Self {
            inner,
            owner_gets: Arc::new(AtomicUsize::new(0)),
            full_gets: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn owner_gets(&self) -> usize {
        self.owner_gets.load(Ordering::SeqCst)
    }

    pub fn full_gets(&self) -> usize {
        self.full_gets.load(Ordering::SeqCst)
    }
}

impl ChunkRepository for CountingChunkRepository {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        self.full_gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(chunk_id)
    }

    fn find_artifact_id(&self, chunk_id: ChunkId) -> Result<Option<ArtifactId>, PortError> {
        self.owner_gets.fetch_add(1, Ordering::SeqCst);
        self.inner.find_artifact_id(chunk_id)
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        self.inner.put(chunk)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        self.inner.list_for_artifact(artifact_id)
    }
}

pub struct CountingEvidenceRepository {
    inner: Arc<InMemoryEvidenceRepository>,
    gets: Arc<AtomicUsize>,
}

impl CountingEvidenceRepository {
    pub fn new(inner: Arc<InMemoryEvidenceRepository>) -> Self {
        Self {
            inner,
            gets: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn gets(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
}

impl EvidenceRepository for CountingEvidenceRepository {
    fn get(
        &self,
        evidence_id: maestria_domain::EvidenceId,
    ) -> Result<Option<maestria_domain::Evidence>, PortError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(evidence_id)
    }

    fn put(&self, evidence: maestria_domain::Evidence) -> Result<(), PortError> {
        self.inner.put(evidence)
    }

    fn replace(&self, evidence: maestria_domain::Evidence) -> Result<(), PortError> {
        self.inner.replace(evidence)
    }

    fn list_for_artifact(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<Vec<maestria_domain::Evidence>, PortError> {
        self.inner.list_for_artifact(artifact_id)
    }
}

pub fn denied_artifact(id: ArtifactId) -> Artifact {
    Artifact {
        id,
        title: "denied".to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Indexed,
        content_hash: None,
        parse_status: None,
        security: maestria_domain::SecurityMetadata {
            read_allowed: false,
            ..maestria_domain::SecurityMetadata::default()
        },
    }
}

pub fn chunk(id: ChunkId, artifact_id: ArtifactId, source_span: SourceSpan) -> Chunk {
    Chunk {
        id,
        artifact_id,
        node_id: maestria_domain::StructureNodeId::new(1),
        source_span,
        representations: Vec::new(),
        order: 0,
        text: "needle".to_string(),
    }
}

pub fn plan(intent: SearchIntent) -> Result<SearchPlan, SearchCompatibilityError> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "needle".to_string(),
        intent,
        scope: CorpusScope::Global,
        corpus_snapshot: maestria_domain::CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::with_limits(100, 300, 10, 1, 0)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("maestria:test".to_string())?,
        authorization: Some(maestria_domain::RetrievalPolicySnapshot::global_default()),
        original_intent: None,
        route_decision: None,
    })
}

pub fn request(
    intent: SearchIntent,
    generation: IndexGenerationId,
) -> Result<crate::types::CandidateRequest, SearchCompatibilityError> {
    let plan = plan(intent)?;
    let authorization = maestria_governance::RetrievalSecurityPolicy::default()
        .authorization_context(&plan.scope)
        .map_err(|_| SearchCompatibilityError::InvalidPlan("authorization context"))?;
    Ok(crate::types::CandidateRequest {
        plan,
        query: SearchQuery {
            q: "needle".to_string(),
            limit: 5,
            offset: 0,
        },
        expected_generation: generation,
        authorization,
    })
}
