use std::{fmt, future::Future, path::PathBuf, pin::Pin, time::Duration};

pub use maestria_domain::HarnessRunId;
use maestria_domain::{
    ApprovalId, Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, ClaimId,
    DomainEventEnvelope, Evidence, EvidenceId, IndexFingerprint, IndexGenerationId, LogicalTick,
    MemoryCandidateId, Relation, RelationEndpoint, RelationId, RepresentationName, ScopeId,
    SearchOutcome, SearchPlan, TaskId, WebEvidenceMetadata,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    NotFound,
    Conflict { message: String },
    InvalidInput { message: String },
    Downstream { message: String },
    Internal { message: String },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::Conflict { message } => write!(f, "conflict: {message}"),
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::Downstream { message } => write!(f, "downstream error: {message}"),
            Self::Internal { message } => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for PortError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFilter {
    pub artifact_id: Option<ArtifactId>,
}

pub trait ArtifactRepository: Send + Sync {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError>;
    fn put(&self, artifact: Artifact) -> Result<(), PortError>;
}

pub trait ChunkRepository: Send + Sync {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError>;
    fn put(&self, chunk: Chunk) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError>;
}

pub trait CardRepository: Send + Sync {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError>;
    fn put(&self, card: Card) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError>;
}

pub trait EvidenceRepository: Send + Sync {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError>;
    /// Insert evidence only if it does not already exist.
    /// Returns `Ok(())` on identical retries; returns `PortError::Conflict`
    /// when a different value already exists for this `EvidenceId`.
    fn put(&self, evidence: Evidence) -> Result<(), PortError>;
    /// Unconditionally store evidence, replacing any existing row.
    fn replace(&self, evidence: Evidence) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError>;
}

pub trait EventLog: Send + Sync {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError>;
    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError>;
}

/// Lifecycle state for a supervised non-idempotent effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectJournalStatus {
    Intent,
    Started,
    FeedbackAccepted,
    Completed,
    Failed,
    Paused,
    Superseded,
}

/// Runtime-owned request persisted before a harness effect starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectJournalIntent {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub capability: String,
    pub command: String,
    pub scope_id: ScopeId,
    pub requested_generation: Option<u64>,
}

/// Durable lifecycle entry for one supervised effect generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectJournalEntry {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub capability: String,
    pub command: String,
    pub scope_id: ScopeId,
    pub generation: u64,
    pub status: EffectJournalStatus,
}

/// Durable supervision journal for non-idempotent effect execution.
pub trait EffectJournal: Send + Sync {
    fn record_intent(&self, intent: EffectJournalIntent) -> Result<EffectJournalEntry, PortError>;
    fn record_started(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError>;
    /// Atomically claims feedback for the current generation before enqueueing it.
    fn claim_feedback(&self, run_id: HarnessRunId, generation: u64) -> Result<(), PortError>;
    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError>;
    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError>;
    fn is_feedback_accepted(
        &self,
        run_id: HarnessRunId,
        generation: u64,
    ) -> Result<bool, PortError>;
    fn is_current(&self, run_id: HarnessRunId, generation: u64) -> Result<bool, PortError>;
}

/// Durable per-namespace ID allocation.
///
/// Each allocation is atomic and persisted so that concurrent or
/// post-restart callers never receive the same ID within a namespace.
pub trait IdAllocator: Send + Sync {
    fn allocate_claim_id(&self) -> Result<ClaimId, PortError>;
    fn allocate_memory_candidate_id(&self) -> Result<MemoryCandidateId, PortError>;
    fn allocate_approval_id(&self) -> Result<ApprovalId, PortError>;
}

pub trait BlobStore: Send + Sync {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError>;
    fn get(&self, id: BlobId) -> Result<Vec<u8>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedChunk {
    pub artifact_id: ArtifactId,
    pub chunk_id: ChunkId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedCard {
    pub artifact_id: ArtifactId,
    pub card_id: CardId,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub q: String,
    pub limit: usize,
    /// Number of matching documents to skip before applying `limit`.
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub chunk: IndexedChunk,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardHit {
    pub card: IndexedCard,
    pub score: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingIdentity {
    pub generation_id: IndexGenerationId,
    pub fingerprint: IndexFingerprint,
    pub representation: RepresentationName,
}

impl EmbeddingIdentity {
    pub fn legacy(model: impl Into<String>, dimensions: usize) -> Result<Self, PortError> {
        let artifact_hash = maestria_domain::ContentHash::new(format!("sha256:{}", "0".repeat(64)))
            .map_err(|error| PortError::Internal {
                message: format!("create legacy embedding fingerprint: {error}"),
            })?;
        Ok(Self {
            generation_id: IndexGenerationId::new(1),
            fingerprint: IndexFingerprint {
                provider: "legacy-local".to_string(),
                model: model.into(),
                revision: "legacy".to_string(),
                artifact_hash,
                dimensions: dimensions as u32,
                quantization: "f32".to_string(),
                query_template_hash: "legacy-query".to_string(),
                document_template_hash: "legacy-document".to_string(),
                preprocessing_version: "legacy".to_string(),
            },
            representation: RepresentationName::new("dense_text_v1"),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddingInputKind {
    Document,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetentionPolicy {
    NoRetention,
    ProviderDefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDisclosure {
    pub remote: bool,
    pub retention: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingProvenance {
    pub content_hash: String,
    pub identity: EmbeddingIdentity,
    pub provider_id: String,
    pub model: String,
    pub model_version: String,
    pub disclosure: ProviderDisclosure,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorEmbedding {
    pub chunk_id: ChunkId,
    pub vector: Vec<f32>,
    pub provenance: EmbeddingProvenance,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VectorSearchQuery {
    pub vector: Vec<f32>,
    pub limit: u32,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub model_version: Option<String>,
    pub identity: Option<EmbeddingIdentity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchHit {
    pub chunk_id: ChunkId,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingRequest {
    pub text: String,
    pub model: String,
    pub kind: EmbeddingInputKind,
    pub identity: EmbeddingIdentity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingResponse {
    pub vector: Vec<f32>,
    pub provider_id: String,
    pub model: String,
    pub model_version: String,
    pub identity: EmbeddingIdentity,
    pub disclosure: ProviderDisclosure,
}
pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError>;
    fn identity(&self) -> Option<EmbeddingIdentity> {
        None
    }
}

/// Executes a typed knowledge search and returns one provenance-bearing outcome.
pub trait SearchKnowledgeExecutor: Send + Sync {
    fn search(
        &self,
        plan: SearchPlan,
    ) -> Pin<Box<dyn Future<Output = Result<SearchOutcome, PortError>> + Send + '_>>;
}

pub trait VectorIndex: Send + Sync {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError>;
    fn search_similar(&self, query: VectorSearchQuery) -> Result<Vec<VectorSearchHit>, PortError>;

    /// Execute a vector search, applying a pre-score filter.
    fn search_similar_filtered(
        &self,
        query: VectorSearchQuery,
        filter: &dyn Fn(ChunkId) -> bool,
    ) -> Result<Vec<VectorSearchHit>, PortError> {
        let _ = (query, filter);
        Err(PortError::Internal {
            message: "search_similar_filtered not supported by this index".into(),
        })
    }
    fn delete_chunks(&self, chunk_ids: &[ChunkId]) -> Result<(), PortError>;
    fn clear(&self) -> Result<(), PortError>;
    fn rebuild(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        self.clear()?;
        self.index_embeddings(embeddings)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub size: usize,
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHandle {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessCommandClass {
    Shell,
    Browser,
    Fetch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessCapabilities {
    pub command_classes: Vec<HarnessCommandClass>,
    pub write_enabled: bool,
    pub read_enabled: bool,
    pub web_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRequest {
    pub run_id: HarnessRunId,
    pub command: String,
    pub working_directory: PathBuf,
    pub duration_budget: Duration,
    pub class: HarnessCommandClass,
    pub readable_roots: Vec<PathBuf>,
    pub blocked_paths: Vec<PathBuf>,
    pub blocked_patterns: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessOutcome {
    pub run_id: HarnessRunId,
    pub command: String,
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
    pub artifacts_created: Vec<BlobId>,
    pub diff_summary: Option<String>,
    pub validation_hints: Vec<String>,
}

pub trait HarnessAdapter: Send + Sync {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError>;
    fn execute(
        &self,
        request: HarnessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HarnessOutcome, PortError>> + Send + '_>>;
}
pub trait GraphIndex: Send + Sync {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError>;
    fn get_relations_for(&self, endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError>;
    fn delete_relations(&self, relation_ids: &[RelationId]) -> Result<(), PortError>;
    fn clear(&self) -> Result<(), PortError>;
    fn rebuild(&self, relations: Vec<Relation>) -> Result<(), PortError> {
        self.clear()?;
        for relation in relations {
            self.insert_relation(relation)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSnapshotData {
    pub url: String,
    pub html: String,
    pub content_hash: String,
    pub metadata: WebEvidenceMetadata,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchOptions {
    pub max_bytes: usize,
    pub max_latency_ms: u32,
    pub allowed_domains: Vec<String>,
    pub allowed_content_types: Vec<String>,
}

impl WebFetchOptions {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            max_latency_ms: 15_000,
            allowed_domains: Vec::new(),
            allowed_content_types: Vec::new(),
        }
    }
}

pub trait WebFetcher: Send + Sync {
    /// Fetches at most `options.max_bytes` of response body data.
    fn fetch(&self, url: &str, max_bytes: usize) -> Result<WebSnapshotData, PortError>;

    fn fetch_with_options(
        &self,
        url: &str,
        options: &WebFetchOptions,
    ) -> Result<WebSnapshotData, PortError> {
        self.fetch(url, options.max_bytes)
    }
}

// ── Durable approval request persistence ────────────────────────────────

/// Risk level at the port boundary, independent of governance crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Lifecycle status of a durable approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
}

/// A durable record of an approval request stored in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub id: ApprovalId,
    pub task_id: TaskId,
    pub effect_kind: String,
    pub risk_level: ApprovalRiskLevel,
    pub capability: String,
    pub scope_id: ScopeId,
    pub tick: LogicalTick,
    pub status: ApprovalStatus,
}

/// Repository for durable approval requests, independent of governance crate.
pub trait ApprovalRepository: Send + Sync {
    fn save(&self, record: &ApprovalRecord) -> Result<(), PortError>;
    fn find_pending(&self) -> Result<Vec<ApprovalRecord>, PortError>;
    fn find_by_id(&self, id: ApprovalId) -> Result<Option<ApprovalRecord>, PortError>;
    fn resolve(&self, id: ApprovalId, approved: bool) -> Result<Option<ApprovalRecord>, PortError>;
    fn find_by_task_id(&self, task_id: TaskId) -> Result<Vec<ApprovalRecord>, PortError>;
}
