use crate::ids::{
    ArtifactId, BlobId, CardId, ChunkId, ClaimId, EvidenceId, HarnessRunId, LogicalTick,
    MemoryCandidateId, MemoryId, RelationId, TaskId, ValidationReportId,
};
use crate::security::SecurityMetadata;
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
pub struct ContentRange {
    pub start: usize,
    pub end: usize,
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
    pub content_hash: Option<String>,
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
    pub content_hash: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebEvidenceMetadata {
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub effective_at: Option<String>,
    pub accessed_at: Option<String>,
    pub content_type: Option<String>,
    pub primary_source: bool,
    pub is_dynamic: bool,
    pub is_paywalled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceKind {
    FileSpan {
        path: String,
        range: ContentRange,
        content_hash: String,
        snapshot: Option<BlobId>,
    },
    PdfSpan {
        blob: BlobId,
        page_start: u32,
        page_end: u32,
    },
    PdfRegion {
        blob: BlobId,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    WebSnapshot {
        url: String,
        snapshot: BlobId,
        fetched_at: LogicalTick,
        content_hash: String,
        metadata: WebEvidenceMetadata,
    },
    CommandOutput {
        harness_run: HarnessRunId,
        stream: OutputStream,
        blob: BlobId,
    },
    TestResult {
        harness_run: HarnessRunId,
        status: TestStatus,
        log: BlobId,
    },
    Diff {
        harness_run: HarnessRunId,
        patch_blob: BlobId,
    },
    Validation {
        report_id: ValidationReportId,
    },
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

pub(crate) const MIN_PROMOTION_CONFIDENCE_MILLI: u16 = 500;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Draft,
    Open,
    Active,
    Validating,
    Blocked,
    CompletedVerified,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Draft => matches!(next, Self::Open | Self::Cancelled),
            Self::Open => matches!(next, Self::Active | Self::Cancelled),
            Self::Active => matches!(
                next,
                Self::Validating | Self::Blocked | Self::Failed | Self::Cancelled
            ),
            Self::Validating => matches!(
                next,
                Self::CompletedVerified | Self::CompletedWithWarnings | Self::Failed | Self::Active
            ),
            Self::Blocked => matches!(next, Self::Active | Self::Failed | Self::Cancelled),
            Self::CompletedVerified
            | Self::CompletedWithWarnings
            | Self::Failed
            | Self::Cancelled => false,
        }
    }

    pub fn is_completion(self) -> bool {
        matches!(self, Self::CompletedVerified | Self::CompletedWithWarnings)
    }
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
