use crate::evidence_source::EvidenceKind;
use crate::ids::{
    ArtifactId, CardId, ChunkId, ClaimId, EvidenceId, LogicalTick, MemoryCandidateId, MemoryId,
    RelationId, TaskId, ValidationReportId,
};
use crate::security::SecurityMetadata;
use crate::task_status::TaskStatus;
use std::collections::BTreeSet;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(try_from = "ContentRangeDto")]
pub struct ContentRange {
    start: usize,
    end: usize,
}

impl ContentRange {
    /// Builds a content-relative range whose start does not exceed its end.
    pub fn new(start: usize, end: usize) -> Result<Self, ContentRangeError> {
        if start > end {
            return Err(ContentRangeError::StartAfterEnd { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn start(&self) -> usize {
        self.start
    }

    pub const fn end(&self) -> usize {
        self.end
    }
}

/// Failure while building a validated [`ContentRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRangeError {
    StartAfterEnd { start: usize, end: usize },
}

impl std::fmt::Display for ContentRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartAfterEnd { start, end } => {
                write!(f, "content range start {start} must not exceed end {end}")
            }
        }
    }
}

impl std::error::Error for ContentRangeError {}

#[derive(::serde::Deserialize)]
struct ContentRangeDto {
    start: usize,
    end: usize,
}

impl TryFrom<ContentRangeDto> for ContentRange {
    type Error = ContentRangeError;

    fn try_from(dto: ContentRangeDto) -> Result<Self, Self::Error> {
        Self::new(dto.start, dto.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexStatus {
    #[default]
    Unindexed,
    Pending,
    Indexed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub id: ArtifactId,
    pub title: String,
    pub chunk_ids: BTreeSet<ChunkId>,
    pub card_ids: BTreeSet<CardId>,
    pub claim_ids: BTreeSet<ClaimId>,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub index_status: IndexStatus,
    pub content_hash: Option<crate::search::ContentHash>,
    pub parse_status: Option<crate::provenance::ParseStatus>,
    pub security: SecurityMetadata,
}

impl Artifact {
    pub(crate) fn with_title(id: ArtifactId, title: String) -> Self {
        Self {
            id,
            title,
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::default(),
            content_hash: None,
            parse_status: None,
            security: SecurityMetadata::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingArtifact {
    pub artifact_id: ArtifactId,
    pub title: String,
    pub content_hash: crate::search::ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: ChunkId,
    pub artifact_id: ArtifactId,
    pub node_id: crate::ids::StructureNodeId,
    pub source_span: crate::provenance::SourceSpan,
    pub representations: Vec<crate::provenance::ParsedRepresentation>,
    pub order: u32,
    pub text: String,
}

impl Chunk {
    pub(crate) fn new(
        id: ChunkId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        representations: Vec<crate::provenance::ParsedRepresentation>,
        order: u32,
        text: String,
    ) -> Self {
        Self {
            id,
            artifact_id,
            node_id,
            source_span,
            representations,
            order,
            text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    pub artifact_id: ArtifactId,
    pub node_id: crate::ids::StructureNodeId,
    pub source_span: crate::provenance::SourceSpan,
    pub title: String,
    pub body: String,
    pub claim_ids: BTreeSet<ClaimId>,
    pub security: SecurityMetadata,
}

impl Card {
    pub(crate) fn new(
        id: CardId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        title: String,
        body: String,
        security: SecurityMetadata,
    ) -> Self {
        Self {
            id,
            artifact_id,
            node_id,
            source_span,
            title,
            body,
            claim_ids: BTreeSet::new(),
            security,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
    Combined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Passed,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub id: EvidenceId,
    pub artifact_id: ArtifactId,
    pub claim_id: Option<ClaimId>,
    pub kind: EvidenceKind,
    pub excerpt: String,
    pub observed_at: LogicalTick,
    pub security: SecurityMetadata,
}

impl Evidence {
    pub(crate) fn new(
        id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
        excerpt: String,
        observed_at: LogicalTick,
        security: SecurityMetadata,
    ) -> Self {
        Self {
            id,
            artifact_id,
            claim_id,
            kind,
            excerpt,
            observed_at,
            security,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStatus {
    Draft,
    Proposed,
    Verified,
    Disputed,
    Archived,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub id: ClaimId,
    pub artifact_id: ArtifactId,
    pub text: String,
    pub status: ClaimStatus,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub security: SecurityMetadata,
}

impl Claim {
    pub(crate) fn new(
        id: ClaimId,
        artifact_id: ArtifactId,
        text: String,
        security: SecurityMetadata,
    ) -> Self {
        Self {
            id,
            artifact_id,
            text,
            status: ClaimStatus::Draft,
            evidence_ids: BTreeSet::new(),
            security,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub id: RelationId,
    pub source: RelationEndpoint,
    pub kind: RelationKind,
    pub target: RelationEndpoint,
    pub evidence_id: Option<EvidenceId>,
    pub confidence_milli: u16,
    pub security: SecurityMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelationEndpoint {
    Artifact(ArtifactId),
    Claim(ClaimId),
    Task(TaskId),
    Memory(MemoryId),
    Card(CardId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    Contains,
    Defines,
    Supports,
    Contradicts,
    UsedEvidence,
    BasedOn,
    DerivedFrom,
    AppliesTo,
    RelatedTo,
}

/// Minimum candidate confidence (milli) required for memory promotion,
/// owned by the domain and reused by every promotion gate (R28).
pub const MIN_PROMOTION_CONFIDENCE_MILLI: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReportRecord {
    pub task_id: Option<TaskId>,
    pub passed: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCandidate {
    pub id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub confidence_milli: u16,
    pub security: SecurityMetadata,
}

impl MemoryCandidate {
    pub fn has_evidence(&self) -> bool {
        !self.evidence_ids.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryStatus {
    Active,
    Deprecated,
    Contradicted,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    pub id: MemoryId,
    pub candidate_id: MemoryCandidateId,
    pub claim_id: ClaimId,
    pub evidence_ids: BTreeSet<EvidenceId>,
    pub status: MemoryStatus,
    pub security: SecurityMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub priority: TaskPriority,
    pub status: TaskStatus,
    pub validation_report_id: Option<ValidationReportId>,
    pub artifact_ids: BTreeSet<ArtifactId>,
    pub evidence_ids: BTreeSet<EvidenceId>,
}

impl Task {
    pub(crate) fn new(id: TaskId, title: String, priority: TaskPriority) -> Self {
        Self {
            id,
            title,
            priority,
            status: TaskStatus::Draft,
            validation_report_id: None,
            artifact_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
        }
    }
}
