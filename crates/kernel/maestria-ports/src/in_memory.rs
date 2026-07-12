use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::{
    Artifact, ArtifactId, BlobId, Card, CardId, Chunk, ChunkId, DomainEvent, DomainEventEnvelope,
    Evidence, EvidenceId, FileHandle, FileMetadata, GraphIndex, HarnessAdapter,
    HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest, Parser, PortError,
    Relation, RelationEndpoint, RelationId, SearchHit, SearchQuery, VectorIndex, VectorSearchHit,
    VectorSearchQuery, WebFetcher, WebSnapshotData,
};

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

impl super::ChunkRepository for InMemoryChunkRepository {
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

impl super::CardRepository for InMemoryCardRepository {
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

impl super::EvidenceRepository for InMemoryEvidenceRepository {
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
        if let Some(existing) = guard.get(&evidence.id) {
            if existing == &evidence {
                return Ok(());
            }
            return Err(PortError::Conflict {
                message: format!(
                    "evidence {} already exists with different content; evidence is immutable",
                    evidence.id.value()
                ),
            });
        }
        guard.insert(evidence.id, evidence);
        Ok(())
    }

    fn replace(&self, evidence: Evidence) -> Result<(), PortError> {
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

impl super::ArtifactRepository for InMemoryArtifactRepository {
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

impl super::EventLog for InMemoryEventLog {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let mut guard = self.events.lock().map_err(|_| PortError::Internal {
            message: "event log lock poisoned".to_string(),
        })?;
        let expected_sequence = guard.len() as u64 + 1;
        if event.sequence.value() != expected_sequence || event.id.value() != expected_sequence {
            return Err(PortError::Conflict {
                message: format!(
                    "expected sequence/id {}, got seq {}, id {}",
                    expected_sequence,
                    event.sequence.value(),
                    event.id.value()
                ),
            });
        }
        guard.push(event);
        Ok(())
    }

    fn scan(&self, filter: super::EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
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
                } => *current == artifact_id,
                DomainEvent::TaskOpened {
                    artifact_id: Some(current),
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

impl super::BlobStore for InMemoryBlobStore {
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
    chunks: Arc<Mutex<Vec<super::IndexedChunk>>>,
}

impl InMemoryFullTextIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl super::FullTextIndex for InMemoryFullTextIndex {
    fn index_chunks(&self, chunks: Vec<super::IndexedChunk>) -> Result<(), PortError> {
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
    embeddings: Arc<Mutex<Vec<super::VectorEmbedding>>>,
}

impl InMemoryVectorIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VectorIndex for InMemoryVectorIndex {
    fn index_embeddings(&self, embeddings: Vec<super::VectorEmbedding>) -> Result<(), PortError> {
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

#[derive(Clone, Default)]
pub struct InMemoryWebFetcher {
    pages: Arc<Mutex<BTreeMap<String, String>>>,
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

    fn parse(
        &self,
        file: FileHandle,
        context: super::ParseContext,
    ) -> Result<super::ParsedArtifact, PortError> {
        if file.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "input file is empty".to_string(),
            });
        }

        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
            message: format!("file bytes are not utf8: {err}"),
        })?;

        let chunk = super::ParsedChunk {
            chunk_id: ChunkId::new(context.artifact_id.value()),
            artifact_id: context.artifact_id,
            text,
        };
        Ok(super::ParsedArtifact {
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
            duration: std::time::Duration::from_millis(1),
            artifacts_created: Vec::new(),
            diff_summary: None,
            validation_hints: Vec::new(),
        })
    }
}
