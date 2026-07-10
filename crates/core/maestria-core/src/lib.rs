#![forbid(unsafe_code)]

//! Local-first orchestration services for Maestria.
//!
//! This crate composes port traits and domain-shaped values. It deliberately
//! avoids concrete SQL, filesystem, search-engine, and parser implementations.

use std::{
    collections::BTreeSet,
    fmt,
    path::{Path, PathBuf},
};

use maestria_domain::{
    Artifact, ArtifactId, Card, Chunk, ChunkId, ContentRange, DomainEvent, DomainEventEnvelope,
    EventId, Evidence, EvidenceId, EvidenceKind, LogicalTick, SequenceNumber,
};
use maestria_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EventFilter, EventLog,
    FileHandle, FileMetadata, FullTextIndex, IndexedChunk, ParsedArtifact, Parser, PortError,
    SearchQuery,
};

pub const CORE_VERSION: &str = "0.1.0";

#[derive(Debug)]
pub enum CoreError {
    InvalidInput { message: String },
    NotFound { message: String },
    Port(PortError),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::NotFound { message } => write!(f, "not found: {message}"),
            Self::Port(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<PortError> for CoreError {
    fn from(value: PortError) -> Self {
        Self::Port(value)
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLayout {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub database_path: PathBuf,
    pub blobs_dir: PathBuf,
    pub full_text_index_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub active_tasks_dir: PathBuf,
    pub system_dir: PathBuf,
    pub event_log_dir: PathBuf,
}

impl InstanceLayout {
    pub fn for_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let workspace_dir = root.join("workspace");
        let system_dir = root.join("system");
        Self {
            manifest_path: root.join("manifest.txt"),
            database_path: root.join("system").join("maestria.db"),
            blobs_dir: root.join("blobs").join("sha256"),
            full_text_index_dir: root.join("indexes").join("full-text"),
            active_tasks_dir: workspace_dir.join("active_tasks"),
            event_log_dir: system_dir.join("event_log"),
            workspace_dir,
            system_dir,
            root,
        }
    }

    pub fn required_directories(&self) -> Vec<PathBuf> {
        vec![
            self.root.clone(),
            self.blobs_dir.clone(),
            self.full_text_index_dir.clone(),
            self.workspace_dir.clone(),
            self.active_tasks_dir.clone(),
            self.system_dir.join("config"),
            self.system_dir.join("policies"),
            self.system_dir.join("logs"),
            self.system_dir.join("evidence_registry"),
            self.event_log_dir.clone(),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitInstanceInput {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitInstancePlan {
    pub layout: InstanceLayout,
    pub directories: Vec<PathBuf>,
    pub manifest_path: PathBuf,
    pub manifest_contents: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InstanceService;

impl InstanceService {
    pub fn init_instance(input: InitInstanceInput) -> CoreResult<InitInstancePlan> {
        if input.root.as_os_str().is_empty() {
            return Err(CoreError::InvalidInput {
                message: "instance root must not be empty".to_string(),
            });
        }
        let layout = InstanceLayout::for_root(input.root);
        let manifest_contents = format!(
            "schema_version=1\nroot={}\nblobs={}\nindex={}\ndatabase={}\n",
            layout.root.display(),
            layout.blobs_dir.display(),
            layout.full_text_index_dir.display(),
            layout.database_path.display()
        );
        Ok(InitInstancePlan {
            directories: layout.required_directories(),
            manifest_path: layout.manifest_path.clone(),
            manifest_contents,
            layout,
        })
    }
}

pub struct CorePorts<'a> {
    pub artifacts: &'a dyn ArtifactRepository,
    pub chunks: &'a dyn ChunkRepository,
    pub cards: &'a dyn CardRepository,
    pub evidence: &'a dyn maestria_ports::EvidenceRepository,
    pub events: &'a dyn EventLog,
    pub parser: &'a dyn Parser,
    pub search_index: &'a dyn FullTextIndex,
    pub blobs: &'a dyn BlobStore,
}

pub struct CoreServices<'a> {
    ports: CorePorts<'a>,
}

impl<'a> CoreServices<'a> {
    pub fn new(ports: CorePorts<'a>) -> Self {
        Self { ports }
    }

    pub fn ingest_file_from_bytes(&self, input: IngestFileInput) -> CoreResult<IngestFileOutput> {
        if input.bytes.is_empty() {
            return Err(CoreError::InvalidInput {
                message: "ingested file bytes must not be empty".to_string(),
            });
        }

        let metadata = file_metadata(&input.path, input.bytes.len());
        if !self.ports.parser.supports(&metadata) {
            return Err(CoreError::InvalidInput {
                message: format!("no parser supports {}", input.path.display()),
            });
        }

        let artifact_id = match input.artifact_id {
            Some(artifact_id) => artifact_id,
            None => artifact_id_for(&input.path, &input.bytes),
        };
        let content_hash = content_hash(&input.bytes);
        let parsed = self.ports.parser.parse(
            FileHandle {
                path: input.path.clone(),
                bytes: input.bytes.clone(),
            },
            maestria_ports::ParseContext { artifact_id },
        )?;
        if parsed.chunks.is_empty() {
            return Err(CoreError::InvalidInput {
                message: "parser returned no chunks for non-empty input".to_string(),
            });
        }
        let blob_id = self.ports.blobs.put(input.bytes.clone())?;
        if let Some(existing_artifact) = self.ports.artifacts.get(artifact_id)? {
            let existing_chunks = self.ports.chunks.list_for_artifact(artifact_id)?;
            let existing_cards = self.ports.cards.list_for_artifact(artifact_id)?;
            let existing_evidence = self.ports.evidence.list_for_artifact(artifact_id)?;
            let events = self.ports.events.scan(EventFilter { artifact_id: None })?;
            if Self::ingestion_is_complete(
                IngestionCheck {
                    artifact: &existing_artifact,
                    chunks: &existing_chunks,
                    cards: &existing_cards,
                    evidence: &existing_evidence,
                    events: &events,
                },
                &parsed,
                &input.path,
                &content_hash,
            ) {
                return Ok(IngestFileOutput {
                    artifact: existing_artifact,
                    chunks: existing_chunks,
                    evidence: existing_evidence,
                    blob_id,
                    content_hash,
                    unchanged: true,
                });
            }
            let has_different_content = existing_evidence.iter().any(|evidence| {
                matches!(
                    &evidence.kind,
                    EvidenceKind::FileSpan {
                        content_hash: existing_hash,
                        ..
                    } if existing_hash != &content_hash
                )
            });
            let has_completed_event = events.iter().any(|envelope| {
                matches!(
                    envelope.event,
                    DomainEvent::ArtifactParsed {
                        artifact_id: parsed_artifact_id,
                        ..
                    } if parsed_artifact_id == artifact_id
                )
            });
            if input.artifact_id.is_some()
                && (has_different_content || (has_completed_event && existing_evidence.is_empty()))
            {
                return Err(CoreError::InvalidInput {
                    message: format!(
                        "artifact {} already contains a different or untraceable content version",
                        artifact_id
                    ),
                });
            }
        }

        let title = title_for_path(&input.path);
        let mut artifact = Artifact {
            id: artifact_id,
            title: title.clone(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        };
        self.ports.artifacts.put(artifact.clone())?;
        self.append_event(DomainEvent::ArtifactRegistered { artifact_id, title })?;

        let source_text = decode_utf8_lossy(&input.bytes);
        let mut indexed_chunks = Vec::with_capacity(parsed.chunks.len());
        let mut persisted_chunks = Vec::with_capacity(parsed.chunks.len());
        let mut persisted_evidence = Vec::with_capacity(parsed.chunks.len());
        let mut search_start = 0usize;

        for (order, parsed_chunk) in parsed.chunks.into_iter().enumerate() {
            let order = u32::try_from(order).map_err(|_| CoreError::InvalidInput {
                message: "parsed chunk count exceeds u32 order range".to_string(),
            })?;
            let chunk = Chunk {
                id: parsed_chunk.chunk_id,
                artifact_id: parsed_chunk.artifact_id,
                order,
                text: parsed_chunk.text,
            };
            let range = line_range_for_chunk(&source_text, &chunk.text, &mut search_start);
            let evidence_id = evidence_id_for(artifact_id, order);
            let proposed_evidence = Evidence {
                id: evidence_id,
                artifact_id,
                claim_id: None,
                kind: EvidenceKind::FileSpan {
                    path: input.path.display().to_string(),
                    range,
                    content_hash: content_hash.clone(),
                },
                excerpt: excerpt_for(&chunk.text),
                observed_at: input.observed_at,
            };
            let evidence = match self.ports.evidence.get(evidence_id)? {
                Some(existing)
                    if existing.artifact_id == proposed_evidence.artifact_id
                        && existing.claim_id == proposed_evidence.claim_id
                        && existing.kind == proposed_evidence.kind
                        && existing.excerpt == proposed_evidence.excerpt =>
                {
                    existing
                }
                _ => proposed_evidence,
            };

            self.ports.chunks.put(chunk.clone())?;
            self.append_event(DomainEvent::ChunkRegistered {
                chunk_id: chunk.id,
                artifact_id,
                order,
                text: chunk.text.clone(),
            })?;
            self.ports.evidence.put(evidence.clone())?;
            self.append_event(DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id: None,
                kind: evidence.kind.clone(),
                excerpt: evidence.excerpt.clone(),
                observed_at: evidence.observed_at,
            })?;

            artifact.chunk_ids.insert(chunk.id);
            artifact.evidence_ids.insert(evidence_id);
            indexed_chunks.push(IndexedChunk {
                artifact_id,
                chunk_id: chunk.id,
                text: chunk.text.clone(),
            });
            persisted_chunks.push(chunk);
            persisted_evidence.push(evidence);
        }

        for card_input in parsed.cards {
            let card = Card {
                id: card_input.card_id,
                artifact_id: card_input.artifact_id,
                title: card_input.title,
                body: card_input.body,
                claim_ids: BTreeSet::new(),
            };
            self.ports.cards.put(card.clone())?;
            self.append_event(DomainEvent::CardCreated {
                card_id: card.id,
                artifact_id,
                title: card.title.clone(),
                body: card.body.clone(),
            })?;
            artifact.card_ids.insert(card.id);
        }

        self.ports.search_index.index_chunks(indexed_chunks)?;
        self.append_event(DomainEvent::ArtifactParsed {
            artifact_id,
            chunks_added: persisted_chunks.len().min(u32::MAX as usize) as u32,
        })?;
        self.ports.artifacts.put(artifact.clone())?;

        Ok(IngestFileOutput {
            artifact,
            chunks: persisted_chunks,
            evidence: persisted_evidence,
            blob_id,
            content_hash,
            unchanged: false,
        })
    }

    pub fn search(&self, input: SearchInput) -> CoreResult<SearchOutput> {
        let hits = self.ports.search_index.search(SearchQuery {
            q: input.query,
            limit: input.limit,
        })?;
        let mut results = Vec::with_capacity(hits.len());
        for hit in hits {
            let artifact = self
                .ports
                .artifacts
                .get(hit.chunk.artifact_id)?
                .ok_or_else(|| CoreError::NotFound {
                    message: format!("artifact {} for search hit", hit.chunk.artifact_id),
                })?;
            let chunk =
                self.ports
                    .chunks
                    .get(hit.chunk.chunk_id)?
                    .ok_or_else(|| CoreError::NotFound {
                        message: format!("chunk {} for search hit", hit.chunk.chunk_id),
                    })?;
            let evidence = self
                .ports
                .evidence
                .get(evidence_id_for(chunk.artifact_id, chunk.order))?
                .ok_or_else(|| CoreError::NotFound {
                    message: format!("evidence for search chunk {}", chunk.id),
                })?;
            results.push(SourceGroundedSearchHit {
                artifact,
                chunk,
                evidence,
                score: hit.score,
            });
        }
        Ok(SearchOutput { hits: results })
    }

    pub fn open_evidence(&self, input: OpenEvidenceInput) -> CoreResult<OpenEvidenceOutput> {
        let evidence =
            self.ports
                .evidence
                .get(input.evidence_id)?
                .ok_or_else(|| CoreError::NotFound {
                    message: format!("evidence {}", input.evidence_id),
                })?;
        let artifact = self
            .ports
            .artifacts
            .get(evidence.artifact_id)?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("artifact {} for evidence", evidence.artifact_id),
            })?;
        Ok(OpenEvidenceOutput { artifact, evidence })
    }

    pub fn open_chunk_evidence(
        &self,
        input: OpenChunkEvidenceInput,
    ) -> CoreResult<OpenEvidenceOutput> {
        let chunk = self
            .ports
            .chunks
            .get(input.chunk_id)?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("chunk {}", input.chunk_id),
            })?;
        let evidence = self
            .ports
            .evidence
            .get(evidence_id_for(chunk.artifact_id, chunk.order))?
            .ok_or_else(|| CoreError::NotFound {
                message: format!("evidence for chunk {}", input.chunk_id),
            })?;
        self.open_evidence(OpenEvidenceInput {
            evidence_id: evidence.id,
        })
    }

    fn append_event(&self, event: DomainEvent) -> CoreResult<DomainEventEnvelope> {
        let events = self.ports.events.scan(EventFilter { artifact_id: None })?;
        if let Some(existing) = events
            .iter()
            .find(|envelope| same_event_identity(&envelope.event, &event))
        {
            return Ok(existing.clone());
        }
        let latest_sequence = events.iter().map(|event| event.sequence.value()).max();
        let next = match latest_sequence {
            Some(sequence) => sequence.saturating_add(1),
            None => 1,
        };
        let envelope = DomainEventEnvelope {
            id: EventId::new(next),
            sequence: SequenceNumber::new(next),
            event,
        };
        self.ports.events.append(envelope.clone())?;
        Ok(envelope)
    }
    fn ingestion_is_complete(
        existing: IngestionCheck<'_>,
        parsed: &ParsedArtifact,
        path: &Path,
        content_hash: &str,
    ) -> bool {
        let IngestionCheck {
            artifact,
            chunks,
            cards,
            evidence,
            events,
        } = existing;
        if parsed.chunks.is_empty() {
            return false;
        }
        let source_path = path.display().to_string();
        if artifact.title != title_for_path(path)
            || artifact.chunk_ids.len() != parsed.chunks.len()
            || artifact.card_ids.len() != parsed.cards.len()
            || artifact.evidence_ids.len() != parsed.chunks.len()
            || chunks.len() != parsed.chunks.len()
            || cards.len() != parsed.cards.len()
            || evidence.len() != parsed.chunks.len()
        {
            return false;
        }

        for (order, expected) in parsed.chunks.iter().enumerate() {
            let Some(chunk) = chunks.iter().find(|chunk| chunk.id == expected.chunk_id) else {
                return false;
            };
            if chunk.artifact_id != artifact.id
                || chunk.order != order as u32
                || chunk.text != expected.text
                || !artifact.chunk_ids.contains(&chunk.id)
            {
                return false;
            }

            let expected_evidence_id = evidence_id_for(artifact.id, order as u32);
            let Some(source_evidence) =
                evidence.iter().find(|item| item.id == expected_evidence_id)
            else {
                return false;
            };
            if !artifact.evidence_ids.contains(&source_evidence.id)
                || source_evidence.artifact_id != artifact.id
                || !matches!(
                    &source_evidence.kind,
                    EvidenceKind::FileSpan {
                        path,
                        content_hash: existing_hash,
                        ..
                    } if path == &source_path && existing_hash == content_hash
                )
            {
                return false;
            }
        }

        for expected in &parsed.cards {
            let Some(card) = cards.iter().find(|card| card.id == expected.card_id) else {
                return false;
            };
            if card.artifact_id != artifact.id
                || card.title != expected.title
                || card.body != expected.body
                || !artifact.card_ids.contains(&card.id)
            {
                return false;
            }
        }

        let has_event = |predicate: &dyn Fn(&DomainEvent) -> bool| {
            events.iter().any(|envelope| predicate(&envelope.event))
        };
        if !has_event(&|event| {
            matches!(
                event,
                DomainEvent::ArtifactRegistered {
                    artifact_id,
                    title
                } if *artifact_id == artifact.id && title == &artifact.title
            )
        }) || !has_event(&|event| {
            matches!(
                event,
                DomainEvent::ArtifactParsed {
                    artifact_id,
                    chunks_added
                } if *artifact_id == artifact.id && *chunks_added == parsed.chunks.len() as u32
            )
        }) {
            return false;
        }

        for (order, expected) in parsed.chunks.iter().enumerate() {
            if !has_event(&|event| {
                matches!(
                    event,
                    DomainEvent::ChunkRegistered {
                        chunk_id,
                        artifact_id,
                        order: event_order,
                        text
                    } if *chunk_id == expected.chunk_id
                        && *artifact_id == artifact.id
                        && *event_order == order as u32
                        && text == &expected.text
                )
            }) || !has_event(&|event| {
                matches!(
                    event,
                    DomainEvent::EvidenceRecorded {
                        evidence_id,
                        artifact_id,
                        ..
                    } if *evidence_id == evidence_id_for(artifact.id, order as u32)
                        && *artifact_id == artifact.id
                )
            }) {
                return false;
            }
        }

        parsed.cards.iter().all(|expected| {
            has_event(&|event| {
                matches!(
                    event,
                    DomainEvent::CardCreated {
                        card_id,
                        artifact_id,
                        title,
                        body
                    } if *card_id == expected.card_id
                        && *artifact_id == artifact.id
                        && title == &expected.title
                        && body == &expected.body
                )
            })
        })
    }
}

struct IngestionCheck<'a> {
    artifact: &'a Artifact,
    chunks: &'a [Chunk],
    cards: &'a [Card],
    evidence: &'a [Evidence],
    events: &'a [DomainEventEnvelope],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestFileInput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub observed_at: LogicalTick,
    pub artifact_id: Option<ArtifactId>,
}
fn same_event_identity(left: &DomainEvent, right: &DomainEvent) -> bool {
    match (left, right) {
        (
            DomainEvent::EvidenceRecorded {
                evidence_id: left_id,
                artifact_id: left_artifact_id,
                claim_id: left_claim_id,
                kind: left_kind,
                excerpt: left_excerpt,
                ..
            },
            DomainEvent::EvidenceRecorded {
                evidence_id: right_id,
                artifact_id: right_artifact_id,
                claim_id: right_claim_id,
                kind: right_kind,
                excerpt: right_excerpt,
                ..
            },
        ) => {
            left_id == right_id
                && left_artifact_id == right_artifact_id
                && left_claim_id == right_claim_id
                && left_kind == right_kind
                && left_excerpt == right_excerpt
        }
        _ => left == right,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestFileOutput {
    pub artifact: Artifact,
    pub chunks: Vec<Chunk>,
    pub evidence: Vec<Evidence>,
    pub blob_id: maestria_domain::BlobId,
    pub content_hash: String,
    pub unchanged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchInput {
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutput {
    pub hits: Vec<SourceGroundedSearchHit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGroundedSearchHit {
    pub artifact: Artifact,
    pub chunk: Chunk,
    pub evidence: Evidence,
    pub score: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenEvidenceInput {
    pub evidence_id: EvidenceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenChunkEvidenceInput {
    pub chunk_id: ChunkId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenEvidenceOutput {
    pub artifact: Artifact,
    pub evidence: Evidence,
}

fn file_metadata(path: &Path, size: usize) -> FileMetadata {
    FileMetadata {
        path: path.to_path_buf(),
        size,
        extension: path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase),
    }
}

fn title_for_path(path: &Path) -> String {
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty());
    match title {
        Some(title) => title.to_string(),
        None => "artifact".to_string(),
    }
}

fn artifact_id_for(path: &Path, bytes: &[u8]) -> ArtifactId {
    let mut hash = Fnv64::new();
    hash.update(path.display().to_string().as_bytes());
    hash.update(&[0]);
    hash.update(bytes);
    ArtifactId::new(non_zero_id(hash.finish() % 1_000_000_000))
}

fn evidence_id_for(artifact_id: ArtifactId, order: u32) -> EvidenceId {
    EvidenceId::new(
        artifact_id
            .value()
            .wrapping_mul(1_000_003)
            .wrapping_add(u64::from(order))
            .wrapping_add(500_001),
    )
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = Fnv64::new();
    hash.update(bytes);
    format!("fnv64:{:016x}", hash.finish())
}

fn non_zero_id(value: u64) -> u64 {
    if value == 0 { 1 } else { value }
}

fn decode_utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn line_range_for_chunk(source: &str, chunk: &str, search_start: &mut usize) -> ContentRange {
    let found = source
        .get(*search_start..)
        .and_then(|tail| tail.find(chunk).map(|offset| *search_start + offset))
        .or_else(|| source.find(chunk));
    let (start_byte, end_byte) = match found {
        Some(start) => {
            let end = start.saturating_add(chunk.len());
            *search_start = end;
            (start, end)
        }
        None => {
            let start_line = line_number_at(source, *search_start);
            let line_count = chunk.lines().count().max(1);
            return ContentRange {
                start: start_line,
                end: start_line.saturating_add(line_count).saturating_sub(1),
            };
        }
    };

    ContentRange {
        start: line_number_at(source, start_byte),
        end: line_number_at(source, end_byte.saturating_sub(1))
            .max(line_number_at(source, start_byte)),
    }
}

fn line_number_at(text: &str, byte_index: usize) -> usize {
    let capped = byte_index.min(text.len());
    text[..capped].bytes().filter(|byte| *byte == b'\n').count() + 1
}

fn excerpt_for(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(240).collect()
}

struct Fnv64(u64);

impl Fnv64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    const fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::EvidenceKind;
    use maestria_ports::{
        FileHandle, FileMetadata, InMemoryArtifactRepository, InMemoryBlobStore,
        InMemoryCardRepository, InMemoryChunkRepository, InMemoryEventLog,
        InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryParser, ParseContext,
        ParsedArtifact, ParsedChunk, Parser, PortError,
    };

    #[derive(Clone)]
    struct ParagraphParser;

    impl Parser for ParagraphParser {
        fn id(&self) -> &'static str {
            "paragraph-parser"
        }

        fn supports(&self, file: &FileMetadata) -> bool {
            file.extension.as_deref() == Some("md")
        }

        fn parse(
            &self,
            file: FileHandle,
            context: ParseContext,
        ) -> Result<ParsedArtifact, PortError> {
            let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
                message: format!("file bytes are not utf8: {err}"),
            })?;
            let mut chunks = Vec::new();
            for paragraph in text.split("\n\n").filter(|paragraph| !paragraph.is_empty()) {
                let chunk_index = chunks.len() as u64;
                chunks.push(ParsedChunk {
                    chunk_id: ChunkId::new(
                        context
                            .artifact_id
                            .value()
                            .saturating_mul(100)
                            .saturating_add(chunk_index)
                            .saturating_add(1),
                    ),
                    artifact_id: context.artifact_id,
                    text: paragraph.to_string(),
                });
            }

            Ok(ParsedArtifact {
                artifact_id: context.artifact_id,
                chunks,
                cards: Vec::new(),
            })
        }
    }

    #[test]
    fn ingest_markdown_search_and_open_evidence_with_in_memory_ports()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = InMemoryArtifactRepository::new();
        let chunks = InMemoryChunkRepository::new();
        let cards = InMemoryCardRepository::new();
        let evidence = InMemoryEvidenceRepository::new();
        let events = InMemoryEventLog::new();
        let parser = InMemoryParser::new();
        let search_index = InMemoryFullTextIndex::new();
        let blobs = InMemoryBlobStore::new();
        let core = CoreServices::new(CorePorts {
            artifacts: &artifacts,
            chunks: &chunks,
            cards: &cards,
            evidence: &evidence,
            events: &events,
            parser: &parser,
            search_index: &search_index,
            blobs: &blobs,
        });

        let path = PathBuf::from("notes/project.md");
        let ingested = core.ingest_file_from_bytes(IngestFileInput {
            path: path.clone(),
            bytes: b"# Project\n\nLocal brain ingestion should find retrieval evidence.".to_vec(),
            observed_at: LogicalTick::new(7),
            artifact_id: Some(ArtifactId::new(42)),
        })?;

        assert_eq!(ingested.artifact.id, ArtifactId::new(42));
        assert_eq!(ingested.chunks.len(), 1);
        assert_eq!(ingested.evidence.len(), 1);
        assert_eq!(ingested.chunks[0].artifact_id, ingested.artifact.id);
        assert_eq!(ingested.evidence[0].artifact_id, ingested.artifact.id);
        let events_before = maestria_ports::EventLog::scan(
            &events,
            maestria_ports::EventFilter { artifact_id: None },
        )?
        .len();
        let repeated = core.ingest_file_from_bytes(IngestFileInput {
            path: path.clone(),
            bytes: b"# Project\n\nLocal brain ingestion should find retrieval evidence.".to_vec(),
            observed_at: LogicalTick::new(8),
            artifact_id: Some(ArtifactId::new(42)),
        })?;

        let changed = core.ingest_file_from_bytes(IngestFileInput {
            path,
            bytes: b"# Project\n\nA changed source body.".to_vec(),
            observed_at: LogicalTick::new(9),
            artifact_id: Some(ArtifactId::new(42)),
        });
        assert!(matches!(
            changed,
            Err(CoreError::InvalidInput { ref message })
                if message.contains("different or untraceable")
        ));
        assert!(repeated.unchanged);
        assert_eq!(repeated.artifact, ingested.artifact);
        assert_eq!(repeated.chunks, ingested.chunks);
        assert_eq!(repeated.evidence, ingested.evidence);
        assert_eq!(
            maestria_ports::EventLog::scan(
                &events,
                maestria_ports::EventFilter { artifact_id: None }
            )?
            .len(),
            events_before
        );

        let search = core.search(SearchInput {
            query: "retrieval".to_string(),
            limit: 5,
        })?;
        assert_eq!(search.hits.len(), 1);
        assert_eq!(search.hits[0].artifact.id, ingested.artifact.id);
        assert_eq!(search.hits[0].chunk.id, ingested.chunks[0].id);
        let hit_evidence = &search.hits[0].evidence;
        assert_eq!(hit_evidence.id, ingested.evidence[0].id);

        let opened = core.open_evidence(OpenEvidenceInput {
            evidence_id: hit_evidence.id,
        })?;
        assert_eq!(opened.artifact.id, ingested.artifact.id);
        assert_eq!(opened.evidence.id, hit_evidence.id);
        match opened.evidence.kind {
            EvidenceKind::FileSpan {
                path,
                range,
                content_hash,
            } => {
                assert_eq!(path, "notes/project.md");
                assert_eq!(range.start, 1);
                assert!(range.end >= range.start);
                assert_eq!(content_hash, ingested.content_hash);
            }
            other => panic!("expected file evidence, got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn chunk_evidence_lookup_uses_the_matching_chunk_order_and_source_span()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = InMemoryArtifactRepository::new();
        let chunks = InMemoryChunkRepository::new();
        let cards = InMemoryCardRepository::new();
        let evidence = InMemoryEvidenceRepository::new();
        let events = InMemoryEventLog::new();
        let parser = ParagraphParser;
        let search_index = InMemoryFullTextIndex::new();
        let blobs = InMemoryBlobStore::new();
        let core = CoreServices::new(CorePorts {
            artifacts: &artifacts,
            chunks: &chunks,
            cards: &cards,
            evidence: &evidence,
            events: &events,
            parser: &parser,
            search_index: &search_index,
            blobs: &blobs,
        });

        let ingested = core.ingest_file_from_bytes(IngestFileInput {
            path: PathBuf::from("notes/multi-source.md"),
            bytes: concat!(
                "Alpha source span anchors first evidence.\n",
                "\n",
                "Beta source span carries beta-token evidence.\n",
                "\n",
                "Gamma source span carries gamma-token evidence.\n",
            )
            .as_bytes()
            .to_vec(),
            observed_at: LogicalTick::new(11),
            artifact_id: Some(ArtifactId::new(7)),
        })?;

        let expected = [
            ("Alpha source span anchors first evidence.", 1usize),
            ("Beta source span carries beta-token evidence.", 3usize),
            ("Gamma source span carries gamma-token evidence.", 5usize),
        ];
        assert_eq!(ingested.chunks.len(), expected.len());
        assert_eq!(ingested.evidence.len(), expected.len());

        for (order, ((chunk, evidence), (excerpt, line))) in ingested
            .chunks
            .iter()
            .zip(ingested.evidence.iter())
            .zip(expected.iter())
            .enumerate()
        {
            assert_eq!(chunk.order, order as u32);
            assert_eq!(evidence.excerpt, *excerpt);

            let opened = core.open_chunk_evidence(OpenChunkEvidenceInput { chunk_id: chunk.id })?;
            assert_eq!(opened.artifact.id, ingested.artifact.id);
            assert_eq!(opened.evidence.id, evidence.id);
            assert_eq!(opened.evidence.excerpt, *excerpt);
            match opened.evidence.kind {
                EvidenceKind::FileSpan {
                    path,
                    range,
                    content_hash,
                } => {
                    assert_eq!(path, "notes/multi-source.md");
                    assert_eq!(range.start, *line);
                    assert_eq!(range.end, *line);
                    assert_eq!(content_hash, ingested.content_hash);
                }
                other => panic!("expected file evidence, got {other:?}"),
            }
        }

        let search = core.search(SearchInput {
            query: "gamma-token".to_string(),
            limit: 5,
        })?;
        assert_eq!(search.hits.len(), 1);
        let hit = &search.hits[0];
        assert_eq!(hit.artifact.id, ingested.artifact.id);
        assert_eq!(hit.chunk.id, ingested.chunks[2].id);
        let hit_evidence = &hit.evidence;
        assert_eq!(hit_evidence.id, ingested.evidence[2].id);
        assert_eq!(hit_evidence.excerpt, expected[2].0);
        match &hit_evidence.kind {
            EvidenceKind::FileSpan {
                path,
                range,
                content_hash,
            } => {
                assert_eq!(path, "notes/multi-source.md");
                assert_eq!(range.start, expected[2].1);
                assert_eq!(range.end, expected[2].1);
                assert_eq!(content_hash, &ingested.content_hash);
            }
            other => panic!("expected file evidence, got {other:?}"),
        }

        Ok(())
    }

    #[test]
    fn init_instance_returns_isolated_local_layout() -> Result<(), Box<dyn std::error::Error>> {
        let plan = InstanceService::init_instance(InitInstanceInput {
            root: PathBuf::from("/tmp/maestria/personal"),
        })?;

        assert_eq!(
            plan.layout.blobs_dir,
            PathBuf::from("/tmp/maestria/personal/blobs/sha256")
        );
        assert_eq!(
            plan.layout.full_text_index_dir,
            PathBuf::from("/tmp/maestria/personal/indexes/full-text")
        );
        assert!(plan.directories.contains(&plan.layout.active_tasks_dir));
        assert!(plan.manifest_contents.contains("schema_version=1"));
        Ok(())
    }
}
