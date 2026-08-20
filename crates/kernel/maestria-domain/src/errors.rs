#[path = "error_display.rs"]
mod error_display;
#[path = "error_notebook.rs"]
mod error_notebook;

use crate::ids::{
    ArtifactId, CardId, ChunkId, ClaimId, EvidenceId, HarnessRunId, IndexGenerationId,
    MemoryCandidateId, MemoryId, NotebookDraftId, NotebookId, RelationId, TaskId,
    ValidationReportId,
};
use crate::notebook::SourceIdentityKey;
use crate::task_status::TaskStatus;
use crate::{GrantTokenDigest, RealmId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    DuplicateArtifact {
        id: ArtifactId,
    },
    DuplicateChunk {
        id: ChunkId,
    },
    DuplicateChunkOrder {
        id: ChunkId,
    },
    DuplicateCard {
        id: CardId,
    },
    DuplicateClaim {
        id: ClaimId,
    },
    DuplicateEvidenceInClaim {
        id: EvidenceId,
    },
    DuplicateEvidenceClaim {
        id: EvidenceId,
    },
    DuplicateEvidence {
        id: EvidenceId,
    },
    DuplicateMemoryCandidate {
        id: MemoryCandidateId,
    },
    DuplicateMemory {
        id: MemoryId,
    },
    DuplicateRelation {
        id: RelationId,
    },
    DuplicateTask {
        id: TaskId,
    },
    DuplicateValidationReport {
        id: ValidationReportId,
    },
    DuplicateNotebook {
        id: NotebookId,
    },
    DuplicateNotebookDraft {
        id: NotebookDraftId,
    },
    DuplicateIndexGeneration {
        id: IndexGenerationId,
    },
    DuplicateModelAgentProposalRunId {
        run_id: HarnessRunId,
    },
    ModelAgentProposalRequestNotFresh {
        run_id: HarnessRunId,
    },
    ModelAgentProposalResumeMismatch {
        run_id: HarnessRunId,
    },
    ModelAgentProposalNotResumable {
        run_id: HarnessRunId,
    },
    MissingArtifact {
        id: ArtifactId,
    },
    MissingChunk {
        id: ChunkId,
    },
    MissingCard {
        id: CardId,
    },
    MissingEvidence {
        id: EvidenceId,
    },
    MissingClaim {
        id: ClaimId,
    },
    MissingTask {
        id: TaskId,
    },
    MissingRelation {
        id: RelationId,
    },
    MissingMemoryCandidate {
        id: MemoryCandidateId,
    },
    MissingMemory {
        id: MemoryId,
    },
    MemorySupersedesItself {
        memory_id: MemoryId,
    },
    MissingValidationReport {
        id: ValidationReportId,
    },
    MissingIndexGeneration {
        id: IndexGenerationId,
    },
    ValidationReportTaskMismatch {
        report_id: ValidationReportId,
        report_task_id: Option<TaskId>,
        task_id: TaskId,
    },
    InvalidTaskTransition {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    },
    InvalidGenerationTransition {
        id: IndexGenerationId,
        from: crate::generations::IndexLifecycle,
        to: crate::generations::IndexLifecycle,
    },
    ValidationRequired {
        task_id: TaskId,
    },
    MemoryCandidateRequiresEvidence {
        id: MemoryCandidateId,
    },
    ArtifactIndexedRequiresEvidence {
        id: ArtifactId,
    },
    MemoryCandidateIneligibleForPromotion {
        candidate_id: MemoryCandidateId,
        confidence_milli: u16,
        minimum_confidence_milli: u16,
        reason: &'static str,
    },
    InvalidEventId {
        expected: u64,
        actual: u64,
    },
    EmptyIntent,
    EmptyClaimText,
    InvalidConfidence {
        max: u16,
        actual: u16,
    },
    ArtifactMismatch {
        expected: ArtifactId,
        actual: ArtifactId,
    },
    ValidationFailed {
        task_id: TaskId,
    },
    ValidationWarningsRequired {
        task_id: TaskId,
    },
    ValidationWarningsForbidden {
        task_id: TaskId,
    },
    PendingChunksExist {
        artifact_id: ArtifactId,
    },
    MalformedDeterministicEvidence {
        evidence_id: EvidenceId,
        reason: &'static str,
    },
    SearchIncompatible {
        error: crate::search::SearchCompatibilityError,
    },
    DuplicateRealmReadGrantDigest {
        digest: GrantTokenDigest,
    },
    DuplicateActiveRealmReadGrant {
        consumer_realm: RealmId,
    },
    MissingRealmReadGrant {
        digest: GrantTokenDigest,
    },
    RealmReadGrantAlreadyRevoked {
        digest: GrantTokenDigest,
    },
    RealmReadGrantRevoked {
        digest: GrantTokenDigest,
    },
    RealmReadGrantUnsupportedAccess {
        digest: GrantTokenDigest,
    },
    RealmReadGrantProviderMismatch {
        expected: RealmId,
        actual: RealmId,
    },
    RealmReadGrantConsumerMismatch {
        expected: RealmId,
        actual: RealmId,
    },
    MissingNotebook {
        id: NotebookId,
    },
    MissingNotebookDraft {
        id: NotebookDraftId,
    },
    NotebookSourceUnavailable {
        key: SourceIdentityKey,
    },
    NotebookSourceArtifactUnavailable {
        artifact_id: ArtifactId,
    },
    InvalidSourceIdentityKey {
        reason: String,
    },
    NotebookDraftRevisionConflict {
        notebook_id: NotebookId,
        draft_id: Option<NotebookDraftId>,
        expected: Option<u64>,
        actual: Option<u64>,
    },
    InvalidNotebookDraft {
        reason: String,
    },
    InternalInvariantViolation {
        detail: &'static str,
    },
}
