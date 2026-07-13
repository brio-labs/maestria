use std::{fmt, future::Future, path::PathBuf, pin::Pin, time::Duration};

use maestria_domain::{
    ApprovalId, Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, ClaimId,
    CreateCardInput, DomainEventEnvelope, Evidence, EvidenceId, LogicalTick, MemoryCandidateId,
    Relation, RelationEndpoint, ScopeId, TaskId,
};

pub use maestria_domain::HarnessRunId;

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
    fn record_terminal(
        &self,
        run_id: HarnessRunId,
        generation: u64,
        status: EffectJournalStatus,
    ) -> Result<(), PortError>;
    fn scan_in_flight(&self) -> Result<Vec<EffectJournalEntry>, PortError>;
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

pub trait FullTextIndex: Send + Sync {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError>;
    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError>;
    fn index_cards(&self, cards: Vec<IndexedCard>) -> Result<(), PortError>;
    fn search_cards(&self, query: SearchQuery) -> Result<Vec<CardHit>, PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingProvenance {
    pub content_hash: String,
    pub model_version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorEmbedding {
    pub chunk_id: ChunkId,
    pub vector: Vec<f32>,
    pub provenance: EmbeddingProvenance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchQuery {
    pub vector: Vec<f32>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchHit {
    pub chunk_id: ChunkId,
    pub score: f32,
}

pub trait VectorIndex: Send + Sync {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError>;
    fn search_similar(&self, query: VectorSearchQuery) -> Result<Vec<VectorSearchHit>, PortError>;
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
pub struct ParseContext {
    pub artifact_id: ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpan {
    /// Text chunks carry a 1-based line span (start_line, end_line), both inclusive.
    TextSpan { start_line: usize, end_line: usize },
    /// PDF chunks carry the physical page number (1-based).
    PdfSpan { page: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChunk {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub text: String,
    pub source_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArtifact {
    pub artifact_id: ArtifactId,
    pub chunks: Vec<ParsedChunk>,
    pub cards: Vec<CreateCardInput>,
}

pub trait Parser: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, file: &FileMetadata) -> bool;
    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError>;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSnapshotData {
    pub url: String,
    pub html: String,
}
pub trait WebFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<WebSnapshotData, PortError>;
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
