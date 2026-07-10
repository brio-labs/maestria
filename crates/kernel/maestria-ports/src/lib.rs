#![forbid(unsafe_code)]

//! Capability traits and deterministic in-memory contract adapters for Maestria.
//!
//! This crate defines the side-effect boundaries used by runtime/storage adapters
//! without depending on a specific runtime, database, search engine, parser, or
//! harness implementation.

use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use maestria_domain::{
    Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, CreateCardInput, DomainEvent,
    DomainEventEnvelope, Evidence, EvidenceId, HarnessRunId, Relation, RelationEndpoint,
    RelationId,
};

pub const PORTS_VERSION: &str = "0.1.0";

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
    fn put(&self, evidence: Evidence) -> Result<(), PortError>;
    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError>;
}

pub trait EventLog: Send + Sync {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError>;
    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError>;
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
pub struct SearchQuery {
    pub q: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub chunk: IndexedChunk,
    pub score: u32,
}

pub trait FullTextIndex: Send + Sync {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError>;
    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError>;
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
pub struct ParsedChunk {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub text: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessOutcome {
    pub run_id: HarnessRunId,
    pub command: String,
    pub exit_code: i32,
    pub scope_checked: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub duration: Duration,
    pub artifacts_created: Vec<BlobId>,
    pub diff_summary: Option<String>,
    pub validation_hints: Vec<String>,
}

pub trait HarnessAdapter: Send + Sync {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError>;
    fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, PortError>;
}

#[derive(Clone, Default)]
pub struct InMemoryArtifactRepository {
    artifacts: Arc<Mutex<BTreeMap<ArtifactId, Artifact>>>,
}

impl InMemoryArtifactRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Clone, Default)]
pub struct InMemoryChunkRepository {
    chunks: Arc<Mutex<BTreeMap<ChunkId, Chunk>>>,
}

impl InMemoryChunkRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChunkRepository for InMemoryChunkRepository {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "chunk store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&chunk_id).cloned())
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        let mut guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "chunk store lock poisoned".to_string(),
        })?;
        guard.insert(chunk.id, chunk);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "chunk store lock poisoned".to_string(),
        })?;
        let mut chunks = guard
            .values()
            .filter(|chunk| chunk.artifact_id == artifact_id)
            .cloned()
            .collect::<Vec<_>>();
        chunks.sort_by_key(|chunk| (chunk.order, chunk.id));
        Ok(chunks)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryCardRepository {
    cards: Arc<Mutex<BTreeMap<CardId, Card>>>,
}

impl InMemoryCardRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl CardRepository for InMemoryCardRepository {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError> {
        let guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "card store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&card_id).cloned())
    }

    fn put(&self, card: Card) -> Result<(), PortError> {
        let mut guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "card store lock poisoned".to_string(),
        })?;
        guard.insert(card.id, card);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError> {
        let guard = self.cards.lock().map_err(|_| PortError::Internal {
            message: "card store lock poisoned".to_string(),
        })?;
        Ok(guard
            .values()
            .filter(|card| card.artifact_id == artifact_id)
            .cloned()
            .collect())
    }
}

#[derive(Clone, Default)]
pub struct InMemoryEvidenceRepository {
    evidences: Arc<Mutex<BTreeMap<EvidenceId, Evidence>>>,
}

impl InMemoryEvidenceRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl EvidenceRepository for InMemoryEvidenceRepository {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError> {
        let guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&evidence_id).cloned())
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        let mut guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
        })?;
        guard.insert(evidence.id, evidence);
        Ok(())
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        let guard = self.evidences.lock().map_err(|_| PortError::Internal {
            message: "evidence store lock poisoned".to_string(),
        })?;
        Ok(guard
            .values()
            .filter(|evidence| evidence.artifact_id == artifact_id)
            .cloned()
            .collect())
    }
}

impl ArtifactRepository for InMemoryArtifactRepository {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError> {
        let guard = self.artifacts.lock().map_err(|_| PortError::Internal {
            message: "artifact store lock poisoned".to_string(),
        })?;
        Ok(guard.get(&artifact_id).cloned())
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        let mut guard = self.artifacts.lock().map_err(|_| PortError::Internal {
            message: "artifact store lock poisoned".to_string(),
        })?;
        guard.insert(artifact.id, artifact);
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct InMemoryEventLog {
    events: Arc<Mutex<Vec<DomainEventEnvelope>>>,
}

impl InMemoryEventLog {
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventLog for InMemoryEventLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let mut guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        guard.push(event);
        Ok(())
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        let mut entries = guard.clone();
        if let Some(artifact_id) = filter.artifact_id {
            entries.retain(|entry| match &entry.event {
                DomainEvent::ArtifactRegistered {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ChunkRegistered {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::CardCreated {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ClaimCreated {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::EvidenceRecorded {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::ArtifactParsed {
                    artifact_id: current,
                    ..
                }
                | DomainEvent::SearchCompleted {
                    artifact_id: current,
                    ..
                } => *current == artifact_id,
                _ => false,
            });
        }
        Ok(entries)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryBlobStore {
    blobs: Arc<Mutex<BTreeMap<BlobId, Vec<u8>>>>,
    ids_by_content: Arc<Mutex<BTreeMap<Vec<u8>, BlobId>>>,
    next_id: Arc<Mutex<u64>>,
}

impl InMemoryBlobStore {
    pub fn new() -> Self {
        Self {
            blobs: Default::default(),
            ids_by_content: Default::default(),
            next_id: Arc::new(Mutex::new(1)),
        }
    }
}

impl BlobStore for InMemoryBlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
        let mut index_guard = self
            .ids_by_content
            .lock()
            .map_err(|_| PortError::Internal {
                message: "blob store lock poisoned".to_string(),
            })?;
        if let Some(id) = index_guard.get(&bytes) {
            return Ok(*id);
        }

        let mut id_guard = self.next_id.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;
        let mut blob_guard = self.blobs.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;

        let id = BlobId::new(*id_guard);
        *id_guard = id.value().saturating_add(1);
        blob_guard.insert(id, bytes.clone());
        index_guard.insert(bytes, id);
        Ok(id)
    }

    fn get(&self, id: BlobId) -> Result<Vec<u8>, PortError> {
        let guard = self.blobs.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;
        guard.get(&id).cloned().ok_or(PortError::NotFound)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryFullTextIndex {
    chunks: Arc<Mutex<Vec<IndexedChunk>>>,
}

impl InMemoryFullTextIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl FullTextIndex for InMemoryFullTextIndex {
    fn index_chunks(&self, chunks: Vec<IndexedChunk>) -> Result<(), PortError> {
        let mut guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        guard.extend(chunks);
        Ok(())
    }

    fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>, PortError> {
        let guard = self.chunks.lock().map_err(|_| PortError::Internal {
            message: "index lock poisoned".to_string(),
        })?;
        let needle = query.q.to_lowercase();
        let mut hits = guard
            .iter()
            .filter(|chunk| chunk.text.to_lowercase().contains(&needle))
            .map(|chunk| SearchHit {
                chunk: chunk.clone(),
                score: (chunk.text.len().min(u32::MAX as usize)) as u32,
            })
            .collect::<Vec<_>>();

        hits.sort_by_key(|b| std::cmp::Reverse(b.score));
        if hits.len() > query.limit {
            hits.truncate(query.limit);
        }
        Ok(hits)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryVectorIndex {
    embeddings: Arc<Mutex<Vec<VectorEmbedding>>>,
}

impl InMemoryVectorIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn index_embeddings(&self, embeddings: Vec<VectorEmbedding>) -> Result<(), PortError> {
        for embedding in &embeddings {
            validate_vector_values(&embedding.vector, "embedding vector")?;
        }

        let mut guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        for emb in embeddings {
            if let Some(pos) = guard.iter().position(|e| e.chunk_id == emb.chunk_id) {
                guard[pos] = emb;
            } else {
                guard.push(emb);
            }
        }
        Ok(())
    }

    fn search_similar(&self, query: VectorSearchQuery) -> Result<Vec<VectorSearchHit>, PortError> {
        validate_vector_values(&query.vector, "query vector")?;
        if query.limit == 0 {
            return Ok(Vec::new());
        }

        let guard = self.embeddings.lock().map_err(|_| PortError::Internal {
            message: "vector index lock poisoned".to_string(),
        })?;
        let mut hits = Vec::new();

        let q_norm_sq: f64 = query.vector.iter().map(|&v| (v as f64) * (v as f64)).sum();
        if q_norm_sq == 0.0 {
            return Ok(Vec::new()); // No meaningful similarity if query is all zeros.
        }
        let q_norm = q_norm_sq.sqrt();

        for emb in guard.iter() {
            if emb.vector.len() != query.vector.len() {
                continue;
            }

            let mut dot: f64 = 0.0;
            let mut emb_norm_sq: f64 = 0.0;
            for (a, b) in emb.vector.iter().zip(&query.vector) {
                let a64 = *a as f64;
                let b64 = *b as f64;
                dot += a64 * b64;
                emb_norm_sq += a64 * a64;
            }

            let score = if emb_norm_sq == 0.0 {
                0.0
            } else {
                (dot / (emb_norm_sq.sqrt() * q_norm)) as f32
            };

            // Ensure score is finite; though f64 math helps avoid overflow,
            // we explicitly reject/clamp to ensure valid output.
            let score = if score.is_finite() { score } else { 0.0 };

            hits.push(VectorSearchHit {
                chunk_id: emb.chunk_id,
                score,
            });
        }
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.chunk_id.value().cmp(&right.chunk_id.value()))
        });
        hits.truncate(query.limit as usize);
        Ok(hits)
    }
}

fn validate_vector_values(vector: &[f32], label: &str) -> Result<(), PortError> {
    if vector.is_empty() {
        return Err(PortError::InvalidInput {
            message: format!("{label} must not be empty"),
        });
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(PortError::InvalidInput {
            message: format!("{label} must contain only finite values"),
        });
    }
    Ok(())
}

pub trait GraphIndex: Send + Sync {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError>;
    fn get_relations_for(&self, endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError>;
}

#[derive(Clone, Default)]
pub struct InMemoryGraphIndex {
    relations: Arc<Mutex<BTreeMap<RelationId, Relation>>>,
}

impl InMemoryGraphIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl GraphIndex for InMemoryGraphIndex {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError> {
        let mut guard = self.relations.lock().map_err(|_| PortError::Internal {
            message: "graph index lock poisoned".to_string(),
        })?;
        guard.insert(relation.id, relation);
        Ok(())
    }

    fn get_relations_for(&self, endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError> {
        let guard = self.relations.lock().map_err(|_| PortError::Internal {
            message: "graph index lock poisoned".to_string(),
        })?;
        Ok(guard
            .values()
            .filter(|r| r.source == endpoint || r.target == endpoint)
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSnapshotData {
    pub url: String,
    pub html: String,
}

pub trait WebFetcher: Send + Sync {
    fn fetch(&self, url: &str) -> Result<WebSnapshotData, PortError>;
}

#[derive(Clone, Default)]
pub struct InMemoryWebFetcher {
    pages: Arc<std::sync::Mutex<BTreeMap<String, String>>>,
}

impl InMemoryWebFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, url: &str, html: &str) -> Result<(), PortError> {
        let mut guard = self.pages.lock().map_err(|_| PortError::Internal {
            message: "web fetcher lock poisoned".to_string(),
        })?;
        guard.insert(url.to_string(), html.to_string());
        Ok(())
    }
}

impl WebFetcher for InMemoryWebFetcher {
    fn fetch(&self, url: &str) -> Result<WebSnapshotData, PortError> {
        if url.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "url cannot be empty".to_string(),
            });
        }
        let guard = self.pages.lock().map_err(|_| PortError::Internal {
            message: "web fetcher lock poisoned".to_string(),
        })?;
        if let Some(html) = guard.get(url) {
            Ok(WebSnapshotData {
                url: url.to_string(),
                html: html.clone(),
            })
        } else {
            Err(PortError::NotFound)
        }
    }
}

#[derive(Clone)]
pub struct InMemoryParser;

impl Default for InMemoryParser {
    fn default() -> Self {
        Self
    }
}

impl InMemoryParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for InMemoryParser {
    fn id(&self) -> &'static str {
        "in-memory-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        match file.extension.as_deref() {
            Some(ext) => matches!(ext, "md" | "txt" | "rs" | "toml"),
            None => false,
        }
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        if file.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "input file is empty".to_string(),
            });
        }

        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
            message: format!("file bytes are not utf8: {err}"),
        })?;

        let chunk = ParsedChunk {
            chunk_id: ChunkId::new(context.artifact_id.value()),
            artifact_id: context.artifact_id,
            text,
        };
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            chunks: vec![chunk],
            cards: Vec::new(),
        })
    }
}

#[derive(Clone)]
pub struct InMemoryHarnessAdapter {
    capabilities: HarnessCapabilities,
}

impl Default for InMemoryHarnessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryHarnessAdapter {
    pub fn new() -> Self {
        Self {
            capabilities: HarnessCapabilities {
                command_classes: vec![HarnessCommandClass::Shell, HarnessCommandClass::Browser],
                write_enabled: true,
                read_enabled: true,
                web_enabled: false,
            },
        }
    }
}

impl HarnessAdapter for InMemoryHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(self.capabilities.clone())
    }

    fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, PortError> {
        if request.command.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "command must not be empty".to_string(),
            });
        }

        let mut stdout = Vec::new();
        stdout.extend_from_slice(format!("executed {}", request.command).as_bytes());

        Ok(HarnessOutcome {
            run_id: request.run_id,
            command: request.command,
            exit_code: 0,
            scope_checked: true,
            stdout,
            stderr: Vec::new(),
            duration: Duration::from_millis(1),
            artifacts_created: Vec::new(),
            diff_summary: None,
            validation_hints: Vec::new(),
        })
    }
}

#[cfg(any(test, feature = "contract-tests"))]
pub mod contract_tests {
    use super::*;
    use maestria_domain::{
        ClaimId, ContentRange, EventId, EvidenceKind, LogicalTick, RelationKind, SequenceNumber,
        ValidationReportId,
    };

    pub fn sample_artifact(id: u64) -> Artifact {
        Artifact {
            id: ArtifactId::new(id),
            title: format!("artifact-{id}"),
            chunk_ids: Default::default(),
            card_ids: Default::default(),
            claim_ids: Default::default(),
            evidence_ids: Default::default(),
        }
    }

    pub fn assert_artifact_repository_round_trip(repository: &impl ArtifactRepository) {
        let artifact = sample_artifact(1);

        repository.put(artifact.clone()).expect("artifact put");

        assert_eq!(
            repository.get(artifact.id).expect("artifact get"),
            Some(artifact)
        );
        assert_eq!(
            repository
                .get(ArtifactId::new(99))
                .expect("missing artifact get"),
            None
        );
    }

    pub fn assert_chunk_repository_round_trip(repository: &impl ChunkRepository) {
        let first = Chunk {
            id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            order: 2,
            text: "second".to_string(),
        };
        let second = Chunk {
            id: ChunkId::new(11),
            artifact_id: ArtifactId::new(1),
            order: 1,
            text: "first".to_string(),
        };
        let unrelated = Chunk {
            id: ChunkId::new(12),
            artifact_id: ArtifactId::new(2),
            order: 0,
            text: "other".to_string(),
        };

        repository.put(first.clone()).expect("first chunk put");
        repository.put(second.clone()).expect("second chunk put");
        repository.put(unrelated).expect("unrelated chunk put");

        assert_eq!(
            repository.get(first.id).expect("chunk get"),
            Some(first.clone())
        );
        assert_eq!(
            repository
                .list_for_artifact(ArtifactId::new(1))
                .expect("chunk list"),
            vec![second, first]
        );
        assert_eq!(
            repository.get(ChunkId::new(99)).expect("missing chunk get"),
            None
        );
    }

    pub fn assert_card_repository_round_trip(repository: &impl CardRepository) {
        let first = Card {
            id: CardId::new(20),
            artifact_id: ArtifactId::new(1),
            title: "bravo".to_string(),
            body: "body b".to_string(),
            claim_ids: [ClaimId::new(3), ClaimId::new(1)].into(),
        };
        let second = Card {
            id: CardId::new(21),
            artifact_id: ArtifactId::new(1),
            title: "alpha".to_string(),
            body: "body a".to_string(),
            claim_ids: Default::default(),
        };
        let unrelated = Card {
            id: CardId::new(22),
            artifact_id: ArtifactId::new(2),
            title: "other".to_string(),
            body: "body".to_string(),
            claim_ids: Default::default(),
        };

        repository.put(first.clone()).expect("first card put");
        repository.put(second.clone()).expect("second card put");
        repository.put(unrelated).expect("unrelated card put");

        assert_eq!(
            repository.get(first.id).expect("card get"),
            Some(first.clone())
        );
        assert_eq!(
            repository
                .list_for_artifact(ArtifactId::new(1))
                .expect("card list"),
            vec![first, second]
        );
        assert_eq!(
            repository.get(CardId::new(99)).expect("missing card get"),
            None
        );
    }

    pub fn assert_evidence_repository_round_trip(repository: &impl EvidenceRepository) {
        let file = Evidence {
            id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: Some(ClaimId::new(7)),
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range: ContentRange { start: 1, end: 4 },
                content_hash: "sha256:notes".to_string(),
            },
            excerpt: "source excerpt".to_string(),
            observed_at: LogicalTick::new(9),
        };
        let validation = Evidence {
            id: EvidenceId::new(41),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(5),
            },
            excerpt: "validated".to_string(),
            observed_at: LogicalTick::new(10),
        };
        let unrelated = Evidence {
            id: EvidenceId::new(42),
            artifact_id: ArtifactId::new(2),
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(6),
            },
            excerpt: "other".to_string(),
            observed_at: LogicalTick::new(11),
        };

        repository.put(file.clone()).expect("file evidence put");
        repository
            .put(validation.clone())
            .expect("validation evidence put");
        repository.put(unrelated).expect("unrelated evidence put");

        assert_eq!(
            repository.get(file.id).expect("evidence get"),
            Some(file.clone())
        );
        assert_eq!(
            repository
                .list_for_artifact(ArtifactId::new(1))
                .expect("evidence list"),
            vec![file, validation]
        );
        assert_eq!(
            repository
                .get(EvidenceId::new(99))
                .expect("missing evidence get"),
            None
        );
    }

    pub fn assert_event_log_round_trip(log: &impl EventLog) {
        let event = DomainEventEnvelope {
            id: EventId::new(1),
            sequence: SequenceNumber::new(1),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(1),
                title: "notes".to_string(),
            },
        };
        let evidence = DomainEventEnvelope {
            id: EventId::new(2),
            sequence: SequenceNumber::new(2),
            event: DomainEvent::EvidenceRecorded {
                evidence_id: EvidenceId::new(40),
                artifact_id: ArtifactId::new(1),
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: "notes.md".to_string(),
                    range: ContentRange { start: 1, end: 4 },
                    content_hash: "sha256:notes".to_string(),
                },
            },
        };
        let search = DomainEventEnvelope {
            id: EventId::new(3),
            sequence: SequenceNumber::new(3),
            event: DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(1),
                cards_added: 2,
            },
        };
        let unrelated = DomainEventEnvelope {
            id: EventId::new(4),
            sequence: SequenceNumber::new(4),
            event: DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(2),
                title: "other".to_string(),
            },
        };

        log.append(event.clone()).expect("event append");
        log.append(evidence.clone()).expect("evidence event append");
        log.append(search.clone()).expect("search event append");
        log.append(unrelated).expect("unrelated event append");

        let all = log
            .scan(EventFilter { artifact_id: None })
            .expect("full event scan");
        assert_eq!(all.len(), 4);

        let filtered = log
            .scan(EventFilter {
                artifact_id: Some(ArtifactId::new(1)),
            })
            .expect("filtered event scan");
        assert_eq!(filtered, vec![event, evidence, search]);
    }

    pub fn assert_blob_store_round_trip(store: &impl BlobStore) {
        let first = store.put(vec![1, 2, 3]).expect("first blob put");
        let first_duplicate = store.put(vec![1, 2, 3]).expect("duplicate blob put");
        let second = store.put(vec![4, 5]).expect("second blob put");

        assert_eq!(first, first_duplicate);
        assert_ne!(first, second);
        assert_eq!(store.get(first).expect("first blob get"), vec![1, 2, 3]);
        assert_eq!(store.get(second).expect("second blob get"), vec![4, 5]);
        assert!(matches!(
            store.get(BlobId::new(99)),
            Err(PortError::NotFound)
        ));
    }

    pub fn assert_full_text_index_round_trip(index: &impl FullTextIndex) {
        index
            .index_chunks(vec![
                IndexedChunk {
                    artifact_id: ArtifactId::new(1),
                    chunk_id: ChunkId::new(10),
                    text: "hello short".to_string(),
                },
                IndexedChunk {
                    artifact_id: ArtifactId::new(1),
                    chunk_id: ChunkId::new(11),
                    text: "hello search with more ranking text".to_string(),
                },
                IndexedChunk {
                    artifact_id: ArtifactId::new(2),
                    chunk_id: ChunkId::new(20),
                    text: "unrelated".to_string(),
                },
            ])
            .expect("index chunks");

        let hits = index
            .search(SearchQuery {
                q: "hello".to_string(),
                limit: 1,
            })
            .expect("search hits");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(11));
    }

    pub fn assert_vector_index_contract(index: &impl VectorIndex) {
        let prov = || EmbeddingProvenance {
            content_hash: "abcd123".into(),
            model_version: "test-v1".into(),
        };

        index
            .index_embeddings(vec![
                VectorEmbedding {
                    chunk_id: ChunkId::new(2),
                    vector: vec![1.0, 0.0],
                    provenance: prov(),
                },
                VectorEmbedding {
                    chunk_id: ChunkId::new(1),
                    vector: vec![1.0, 0.0],
                    provenance: prov(),
                },
                VectorEmbedding {
                    chunk_id: ChunkId::new(3),
                    vector: vec![0.0, 1.0],
                    provenance: prov(),
                },
                VectorEmbedding {
                    chunk_id: ChunkId::new(4),
                    vector: vec![1.0, 0.0, 0.0],
                    provenance: prov(),
                },
            ])
            .expect("index embeddings");

        let equal_score_hits = index
            .search_similar(VectorSearchQuery {
                vector: vec![1.0, 0.0],
                limit: 4,
            })
            .expect("equal-score search");
        assert_eq!(equal_score_hits[0].chunk_id, ChunkId::new(1));
        assert_eq!(equal_score_hits[1].chunk_id, ChunkId::new(2));
        assert!(
            !equal_score_hits
                .iter()
                .any(|hit| hit.chunk_id == ChunkId::new(4))
        );

        let zero_query_hits = index
            .search_similar(VectorSearchQuery {
                vector: vec![0.0, 0.0],
                limit: 10,
            })
            .expect("all-zero query search");
        assert!(
            zero_query_hits.is_empty(),
            "all-zero query must return no hits"
        );

        index
            .index_embeddings(vec![VectorEmbedding {
                chunk_id: ChunkId::new(7),
                vector: vec![0.0, 1.0],
                provenance: prov(),
            }])
            .expect("initial embedding");
        index
            .index_embeddings(vec![VectorEmbedding {
                chunk_id: ChunkId::new(7),
                vector: vec![1.0, 0.0],
                provenance: prov(),
            }])
            .expect("replacement embedding");
        let replacement_hits = index
            .search_similar(VectorSearchQuery {
                vector: vec![1.0, 0.0],
                limit: 10,
            })
            .expect("replacement search");
        let replaced = replacement_hits
            .iter()
            .filter(|hit| hit.chunk_id == ChunkId::new(7))
            .collect::<Vec<_>>();
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0].score, 1.0);

        assert!(matches!(
            index.index_embeddings(vec![VectorEmbedding {
                chunk_id: ChunkId::new(9),
                vector: Vec::new(),
                provenance: prov(),
            }]),
            Err(PortError::InvalidInput { .. })
        ));
        assert!(matches!(
            index.search_similar(VectorSearchQuery {
                vector: vec![f32::NAN],
                limit: 1,
            }),
            Err(PortError::InvalidInput { .. })
        ));
        // Validate query vector before honoring limit=0.
        assert!(matches!(
            index.search_similar(VectorSearchQuery {
                vector: vec![f32::NAN],
                limit: 0,
            }),
            Err(PortError::InvalidInput { .. })
        ));
    }
    pub fn assert_parser_round_trip(parser: &impl Parser) {
        assert_eq!(parser.id(), "in-memory-parser");
        assert!(parser.supports(&FileMetadata {
            path: PathBuf::from("notes.md"),
            size: 5,
            extension: Some("md".to_string()),
        }));
        assert!(!parser.supports(&FileMetadata {
            path: PathBuf::from("archive.bin"),
            size: 5,
            extension: Some("bin".to_string()),
        }));

        let parsed = parser
            .parse(
                FileHandle {
                    path: PathBuf::from("notes.md"),
                    bytes: b"alpha".to_vec(),
                },
                ParseContext {
                    artifact_id: ArtifactId::new(7),
                },
            )
            .expect("parse utf8 file");

        assert_eq!(parsed.artifact_id, ArtifactId::new(7));
        assert_eq!(parsed.chunks.len(), 1);
        assert_eq!(parsed.chunks[0].text, "alpha");

        assert!(matches!(
            parser.parse(
                FileHandle {
                    path: PathBuf::from("empty.md"),
                    bytes: Vec::new(),
                },
                ParseContext {
                    artifact_id: ArtifactId::new(8),
                },
            ),
            Err(PortError::InvalidInput { .. })
        ));
    }

    pub fn assert_harness_adapter_round_trip(harness: &impl HarnessAdapter) {
        let capabilities = harness.capabilities().expect("capabilities");
        assert!(capabilities.read_enabled);
        assert!(capabilities.write_enabled);
        assert!(
            capabilities
                .command_classes
                .contains(&HarnessCommandClass::Shell)
        );

        let outcome = harness
            .execute(HarnessRequest {
                run_id: HarnessRunId::new(7),
                command: "echo ok".to_string(),
                working_directory: PathBuf::from("/tmp"),
                duration_budget: Duration::from_secs(1),
                class: HarnessCommandClass::Shell,
            })
            .expect("execute command");

        assert_eq!(outcome.run_id, HarnessRunId::new(7));
        assert_eq!(outcome.command, "echo ok");
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.scope_checked);
        assert_eq!(outcome.stdout, b"executed echo ok".to_vec());

        assert!(matches!(
            harness.execute(HarnessRequest {
                run_id: HarnessRunId::new(8),
                command: " ".to_string(),
                working_directory: PathBuf::from("/tmp"),
                duration_budget: Duration::from_secs(1),
                class: HarnessCommandClass::Shell,
            }),
            Err(PortError::InvalidInput { .. })
        ));
    }
    pub fn assert_graph_index_contract(index: &impl GraphIndex) {
        let artifact_ep = RelationEndpoint::Artifact(ArtifactId::new(1));
        let card_ep = RelationEndpoint::Card(CardId::new(2));
        let claim_ep = RelationEndpoint::Claim(ClaimId::new(3));

        let mut rel3 = Relation {
            id: RelationId::new(3),
            source: artifact_ep,
            target: card_ep,
            kind: RelationKind::Contains,
            evidence_id: None,
            confidence_milli: 800,
        };
        let rel1 = Relation {
            id: RelationId::new(1),
            source: card_ep,
            target: claim_ep,
            kind: RelationKind::Supports,
            evidence_id: Some(EvidenceId::new(4)),
            confidence_milli: 900,
        };
        let rel2 = Relation {
            id: RelationId::new(2),
            source: artifact_ep,
            target: claim_ep,
            kind: RelationKind::Contradicts,
            evidence_id: None,
            confidence_milli: 500,
        };

        // Insert out of order
        index.insert_relation(rel3.clone()).expect("insert 3");
        index.insert_relation(rel1.clone()).expect("insert 1");
        index.insert_relation(rel2.clone()).expect("insert 2");

        // Replace 3
        rel3.confidence_milli = 950;
        index.insert_relation(rel3.clone()).expect("replace 3");

        // Query for artifact_ep, which is in rel3 (source) and rel2 (source)
        // Must be returned in order of RelationId: rel2 then rel3
        let artifact_rels = index
            .get_relations_for(artifact_ep)
            .expect("get relations for artifact");
        assert_eq!(artifact_rels.len(), 2);
        assert_eq!(artifact_rels[0], rel2);
        assert_eq!(artifact_rels[1], rel3);

        // Query for claim_ep, which is in rel1 (target) and rel2 (target)
        // Must be returned in order of RelationId: rel1 then rel2
        let claim_rels = index
            .get_relations_for(claim_ep)
            .expect("get relations for claim");
        assert_eq!(claim_rels.len(), 2);
        assert_eq!(claim_rels[0], rel1);
        assert_eq!(claim_rels[1], rel2);
    }

    pub fn assert_web_fetcher_contract(
        fetcher: &impl super::WebFetcher,
        valid_url: &str,
        valid_html: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let fetch_res = fetcher.fetch(valid_url)?;
        assert_eq!(fetch_res.url, valid_url, "URL must be preserved");
        assert_eq!(fetch_res.html, valid_html, "HTML must match");
        assert!(!fetch_res.html.is_empty(), "HTML should be non-empty");

        let empty_res = fetcher.fetch("");
        assert!(
            matches!(empty_res, Err(super::PortError::InvalidInput { .. })),
            "Empty URLs must map to PortError::InvalidInput, got {:?}",
            empty_res
        );

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::contract_tests::*;
    use super::*;

    #[test]
    fn in_memory_artifact_repository_satisfies_contract() {
        assert_artifact_repository_round_trip(&InMemoryArtifactRepository::new());
    }

    #[test]
    fn in_memory_chunk_repository_satisfies_contract() {
        assert_chunk_repository_round_trip(&InMemoryChunkRepository::new());
    }

    #[test]
    fn in_memory_web_fetcher_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
        let fetcher = InMemoryWebFetcher::new();
        fetcher.seed("https://example.com/test", "<html><body>test</body></html>")?;
        assert_web_fetcher_contract(
            &fetcher,
            "https://example.com/test",
            "<html><body>test</body></html>",
        )?;

        let missing_res = fetcher.fetch("https://example.com/not-found-anywhere");
        assert!(
            matches!(missing_res, Err(PortError::NotFound)),
            "Missing URLs must map to PortError::NotFound, got {:?}",
            missing_res
        );

        Ok(())
    }

    #[test]
    fn in_memory_card_repository_satisfies_contract() {
        assert_card_repository_round_trip(&InMemoryCardRepository::new());
    }

    #[test]
    fn in_memory_evidence_repository_satisfies_contract() {
        assert_evidence_repository_round_trip(&InMemoryEvidenceRepository::new());
    }

    #[test]
    fn in_memory_event_log_satisfies_contract() {
        assert_event_log_round_trip(&InMemoryEventLog::new());
    }

    #[test]
    fn in_memory_blob_store_satisfies_contract() {
        assert_blob_store_round_trip(&InMemoryBlobStore::new());
    }

    #[test]
    fn in_memory_full_text_index_satisfies_contract() {
        assert_full_text_index_round_trip(&InMemoryFullTextIndex::new());
    }

    #[test]
    fn in_memory_vector_index_satisfies_contract() {
        assert_vector_index_contract(&InMemoryVectorIndex::new());
    }

    #[test]
    fn in_memory_parser_satisfies_contract() {
        assert_parser_round_trip(&InMemoryParser::new());
    }

    #[test]
    fn in_memory_harness_adapter_satisfies_contract() {
        assert_harness_adapter_round_trip(&InMemoryHarnessAdapter::new());
    }

    #[test]
    fn in_memory_graph_index_satisfies_contract() {
        assert_graph_index_contract(&InMemoryGraphIndex::new());
    }
}
