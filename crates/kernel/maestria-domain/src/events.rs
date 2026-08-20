use crate::entities::{ClaimStatus, RelationEndpoint, RelationKind, TaskPriority};
use crate::evidence_source::EvidenceKind;
use crate::ids::StructureNodeId;
use crate::ids::{
    ApprovalId, ArtifactId, ArtifactVersionId, BlobId, CardId, ChunkId, ClaimId, EventId,
    EvidenceId, IndexGenerationId, LogicalTick, MemoryCandidateId, MemoryId, RelationId, TaskId,
    ValidationReportId,
};
use crate::search::{ContentHash, StructureNode};
use crate::security::SecurityMetadata;
use crate::task_status::TaskStatus;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainEventEnvelope {
    pub id: EventId,
    pub event: DomainEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    ArtifactRegistered {
        artifact_id: ArtifactId,
        title: String,
        security: SecurityMetadata,
    },
    ChunkRegistered {
        chunk_id: ChunkId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        representations: Vec<crate::provenance::ParsedRepresentation>,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: CardId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        title: String,
        body: String,
        security: SecurityMetadata,
    },
    ClaimCreated {
        claim_id: ClaimId,
        artifact_id: ArtifactId,
        text: String,
        evidence_ids: Vec<EvidenceId>,
        security: SecurityMetadata,
    },
    EvidenceRecorded {
        evidence_id: EvidenceId,
        artifact_id: ArtifactId,
        claim_id: Option<ClaimId>,
        kind: EvidenceKind,
        excerpt: String,
        observed_at: LogicalTick,
        security: SecurityMetadata,
    },
    TaskOpened {
        task_id: TaskId,
        title: String,
        priority: TaskPriority,
        artifact_id: Option<ArtifactId>,
    },
    TaskStatusChanged {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    },
    /// The completed status carries its validation report (R56).
    TaskCompletionRecorded {
        task_id: TaskId,
        status: TaskStatus,
    },
    TaskEvidenceLinked {
        task_id: TaskId,
        evidence_id: EvidenceId,
    },
    ClaimValidationUpdated {
        claim_id: ClaimId,
        status: ClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: ClaimId,
        evidence_id: EvidenceId,
    },
    RelationCreated {
        relation_id: RelationId,
        source: RelationEndpoint,
        kind: RelationKind,
        target: RelationEndpoint,
        evidence_id: Option<EvidenceId>,
        confidence_milli: u16,
        security: SecurityMetadata,
    },
    MemoryCandidateCreated {
        candidate_id: MemoryCandidateId,
        claim_id: ClaimId,
        evidence_ids: BTreeSet<EvidenceId>,
        confidence_milli: u16,
        security: SecurityMetadata,
    },
    ArtifactParsed {
        artifact_id: ArtifactId,
        status: crate::provenance::ParseStatus,
    },
    DocumentTreeCaptured {
        artifact_id: ArtifactId,
        artifact_version_id: ArtifactVersionId,
        content_hash: ContentHash,
        root_id: StructureNodeId,
        nodes: Vec<StructureNode>,
    },
    PendingIndex {
        artifact_id: ArtifactId,
        content_hash: ContentHash,
    },
    FullTextIndexed {
        artifact_id: ArtifactId,
        chunk_id: ChunkId,
    },
    ArtifactIndexed {
        artifact_id: ArtifactId,
    },
    SearchCompleted {
        artifact_id: ArtifactId,
    },
    HarnessRunCompleted {
        task_id: Option<TaskId>,
        command: String,
        exit_code: i32,
    },
    ModelAgentProposalRequested {
        request: crate::model_agent::ModelAgentProposalRequest,
    },
    ModelAgentProposalCompleted {
        result: crate::model_agent::ModelAgentProposalResult,
    },
    ApprovalRecorded {
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    },
    MemoryPromoted {
        memory_id: MemoryId,
        candidate_id: MemoryCandidateId,
        security: SecurityMetadata,
    },
    MemoryContradicted {
        memory_id: MemoryId,
        contradicting_candidate_id: MemoryCandidateId,
    },
    MemoryDeprecated {
        memory_id: MemoryId,
    },
    MemorySuperseded {
        memory_id: MemoryId,
        by_memory_id: MemoryId,
    },
    ValidationReportCreated {
        report_id: ValidationReportId,
        task_id: Option<TaskId>,
        passed: bool,
        warnings: Vec<String>,
    },
    TickObserved {
        at: LogicalTick,
    },
    SearchExecuted {
        query: String,
        limit: usize,
        evidence_ids: Vec<EvidenceId>,
        pack_metadata: Option<Box<crate::evidence_pack::EvidencePackMetadataRecord>>,
        at: LogicalTick,
    },
    ParserStarted {
        artifact_id: ArtifactId,
        title: String,
        source_path: String,
        content_hash: ContentHash,
        blob_id: BlobId,
    },

    OcrRequested {
        intent: crate::ocr::OcrIntent,
    },
    OcrCompleted {
        artifact_id: ArtifactId,
        completion: crate::ocr::OcrCompletion,
    },
    OcrFailed {
        artifact_id: ArtifactId,
        request_id: crate::ocr::OcrRequestId,
        reason: String,
    },
    SearchKnowledgeCompleted {
        task_id: Option<TaskId>,
        plan: Option<Box<crate::search::SearchPlan>>,
        outcome: crate::search::SearchOutcome,
    },
    IndexGenerationStarted {
        id: IndexGenerationId,
        name: crate::generations::RepresentationName,
        corpus_snapshot: crate::ids::CorpusSnapshotId,
        fingerprint: crate::generations::IndexFingerprint,
        /// Learned-sparse namespace bound to the generation, when the
        /// representation is the sparse projection.
        sparse_namespace: Option<crate::SparseNamespace>,
    },
    IndexGenerationTransitioned {
        id: IndexGenerationId,
        from: crate::generations::IndexLifecycle,
        to: crate::generations::IndexLifecycle,
        replaced_active_id: Option<IndexGenerationId>,
    },
    SourceBecameStale {
        artifact_id: ArtifactId,
        source_path: String,
        content_hash: ContentHash,
    },
    NotebookCreated {
        notebook_id: crate::ids::NotebookId,
        title: crate::notebook::NotebookTitle,
        created_at: LogicalTick,
        updated_at: LogicalTick,
    },
    NotebookRenamed {
        notebook_id: crate::ids::NotebookId,
        title: crate::notebook::NotebookTitle,
        updated_at: LogicalTick,
    },
    NotebookDeleted {
        notebook_id: crate::ids::NotebookId,
    },
    NotebookSourceAttached {
        notebook_id: crate::ids::NotebookId,
        source_key: crate::notebook::SourceIdentityKey,
        updated_at: LogicalTick,
    },
    NotebookSourceDetached {
        notebook_id: crate::ids::NotebookId,
        source_key: crate::notebook::SourceIdentityKey,
        updated_at: LogicalTick,
    },
    NotebookDraftSaved {
        draft_id: crate::ids::NotebookDraftId,
        notebook_id: crate::ids::NotebookId,
        title: crate::notebook::NotebookDraftTitle,
        body_blob: BlobId,
        body_hash: ContentHash,
        revision: crate::notebook::NotebookDraftRevision,
        citations: Vec<crate::notebook::FrozenNotebookCitation>,
        created_at: LogicalTick,
        updated_at: LogicalTick,
    },
    NotebookDraftDeleted {
        notebook_id: crate::ids::NotebookId,
        draft_id: crate::ids::NotebookDraftId,
        revision: crate::notebook::NotebookDraftRevision,
    },
    RealmReadGrantIssued {
        grant: crate::entities::RealmReadGrant,
    },
    RealmReadGrantRevoked {
        token_digest: crate::GrantTokenDigest,
    },
    FederatedReadAccessRecorded {
        token_digest: crate::GrantTokenDigest,
        provider_realm: crate::RealmId,
        consumer_realm: crate::RealmId,
        record: crate::entities::FederatedAccessRecord,
    },
}

impl DomainEvent {
    /// Returns the artifact the event references, when the event is
    /// artifact-scoped. `TaskOpened` reports its optional artifact binding;
    /// `OcrRequested` reports the intent's artifact.
    #[must_use]
    pub fn artifact_id(&self) -> Option<ArtifactId> {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ChunkRegistered { artifact_id, .. }
            | Self::CardCreated { artifact_id, .. }
            | Self::ClaimCreated { artifact_id, .. }
            | Self::EvidenceRecorded { artifact_id, .. }
            | Self::ArtifactParsed { artifact_id, .. }
            | Self::DocumentTreeCaptured { artifact_id, .. }
            | Self::SearchCompleted { artifact_id, .. }
            | Self::PendingIndex { artifact_id, .. }
            | Self::FullTextIndexed { artifact_id, .. }
            | Self::ArtifactIndexed { artifact_id }
            | Self::ParserStarted { artifact_id, .. }
            | Self::SourceBecameStale { artifact_id, .. }
            | Self::OcrCompleted { artifact_id, .. }
            | Self::OcrFailed { artifact_id, .. } => Some(*artifact_id),
            Self::TaskOpened { artifact_id, .. } => *artifact_id,
            Self::OcrRequested { intent } => Some(intent.artifact_id()),
            _ => None,
        }
    }

    /// Returns the approval decision recorded by the event, if any.
    #[must_use]
    pub fn approval_record(&self) -> Option<(ApprovalId, ApprovalOutcome)> {
        match self {
            Self::ApprovalRecorded {
                approval_id,
                outcome,
            } => Some((*approval_id, *outcome)),
            _ => None,
        }
    }

    /// Returns the validation report identity recorded by the event, if any.
    #[must_use]
    pub fn validation_report(&self) -> Option<(ValidationReportId, Option<TaskId>, bool)> {
        match self {
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                ..
            } => Some((*report_id, *task_id, *passed)),
            _ => None,
        }
    }
}

/// Projects the currently active source versions from the append-only event
/// log, keyed by canonical source path.
///
/// `ParserStarted` records the source path with a placeholder version
/// (`ArtifactVersionId` derived from the artifact id); `DocumentTreeCaptured`
/// carries the real content-addressed version and replaces the placeholder for
/// that path (R27). A later `SourceBecameStale` removes the path when it
/// matches the recorded artifact and hash. Consumers share this single
/// projection so the version namespace never borrows the artifact-id namespace
/// and stale versions never surface in retrieval.
pub fn active_source_versions(
    events: &[DomainEventEnvelope],
) -> BTreeMap<PathBuf, (ArtifactId, ArtifactVersionId, ContentHash)> {
    let mut active = BTreeMap::new();
    let mut path_by_artifact = BTreeMap::new();
    for envelope in events {
        match &envelope.event {
            DomainEvent::ParserStarted {
                artifact_id,
                source_path,
                content_hash,
                ..
            } => {
                path_by_artifact.insert(*artifact_id, source_path.clone());
                active.insert(
                    PathBuf::from(source_path),
                    (
                        *artifact_id,
                        ArtifactVersionId::new(artifact_id.value()),
                        content_hash.clone(),
                    ),
                );
            }
            DomainEvent::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id,
                ..
            } => {
                if let Some(path) = path_by_artifact.get(artifact_id)
                    && let Some(entry) = active.get_mut(Path::new(path))
                {
                    entry.1 = *artifact_version_id;
                }
            }
            DomainEvent::SourceBecameStale {
                artifact_id,
                source_path,
                content_hash,
            } => {
                let path = PathBuf::from(source_path);
                if active
                    .get(&path)
                    .is_some_and(|(active_id, _, active_hash)| {
                        active_id == artifact_id && active_hash == content_hash
                    })
                {
                    active.remove(&path);
                }
                path_by_artifact.remove(artifact_id);
            }
            _ => {}
        }
    }
    active
}

/// Outcome recorded by an `ApprovalRecorded` event.
///
/// `Acknowledged` records an operator decision without a task transition
/// (model-agent approvals); the task linkage is audit metadata only.
/// `TaskTransition` records a decision that transitioned a task; the
/// transition is fully specified, so the old correlated `approved` flag plus
/// `Option` status pair is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Acknowledged {
        task_id: Option<TaskId>,
        approved: bool,
    },
    TaskTransition {
        task_id: TaskId,
        approved: bool,
        from_status: TaskStatus,
        to_status: TaskStatus,
    },
}

impl ApprovalOutcome {
    #[must_use]
    pub const fn approved(self) -> bool {
        match self {
            Self::Acknowledged { approved, .. } | Self::TaskTransition { approved, .. } => approved,
        }
    }

    #[must_use]
    pub const fn task_id(self) -> Option<TaskId> {
        match self {
            Self::Acknowledged { task_id, .. } => task_id,
            Self::TaskTransition { task_id, .. } => Some(task_id),
        }
    }
}
