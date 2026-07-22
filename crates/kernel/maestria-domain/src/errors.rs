use crate::entities::TaskStatus;
use crate::ids::{
    ArtifactId, CardId, ChunkId, ClaimId, EvidenceId, IndexGenerationId, MemoryCandidateId,
    MemoryId, RelationId, TaskId, ValidationReportId,
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    DuplicateId {
        kind: &'static str,
        id: u64,
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
    EvidenceRequired {
        kind: &'static str,
        id: u64,
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
    InvalidSequence {
        expected: u64,
        actual: u64,
    },
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
    InternalInvariantViolation {
        detail: &'static str,
    },
}

impl DomainError {
    fn fmt_missing(f: &mut fmt::Formatter, kind: &str, id: impl fmt::Display) -> fmt::Result {
        write!(f, "missing {kind} {id}")
    }

    fn fmt_transition(
        f: &mut fmt::Formatter,
        prefix: impl fmt::Display,
        id: impl fmt::Display,
        from: impl fmt::Debug,
        to: impl fmt::Debug,
    ) -> fmt::Result {
        write!(f, "{prefix} {id}: {from:?} -> {to:?}")
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} id: {id}"),
            Self::MissingArtifact { id } => Self::fmt_missing(f, "artifact", id),
            Self::MissingChunk { id } => Self::fmt_missing(f, "chunk", id),
            Self::MissingCard { id } => Self::fmt_missing(f, "card", id),
            Self::MissingEvidence { id } => Self::fmt_missing(f, "evidence", id),
            Self::MissingClaim { id } => Self::fmt_missing(f, "claim", id),
            Self::MissingTask { id } => Self::fmt_missing(f, "task", id),
            Self::MissingRelation { id } => Self::fmt_missing(f, "relation", id),
            Self::MissingMemoryCandidate { id } => Self::fmt_missing(f, "memory candidate", id),
            Self::MissingMemory { id } => Self::fmt_missing(f, "memory", id),
            Self::MissingValidationReport { id } => Self::fmt_missing(f, "validation report", id),
            Self::MissingIndexGeneration { id } => Self::fmt_missing(f, "index generation", id),
            Self::ValidationReportTaskMismatch {
                report_id,
                report_task_id,
                task_id,
            } => match report_task_id {
                Some(report_task_id) => write!(
                    f,
                    "validation report {report_id} is for task {report_task_id}, not {task_id}"
                ),
                None => write!(
                    f,
                    "validation report {report_id} is not associated with task {task_id}"
                ),
            },
            Self::InvalidTaskTransition { task_id, from, to } => {
                Self::fmt_transition(f, "invalid task transition", task_id, from, to)
            }
            Self::InvalidGenerationTransition { id, from, to } => {
                Self::fmt_transition(f, "invalid index generation transition for", id, from, to)
            }
            Self::ValidationRequired { task_id } => {
                write!(f, "task {task_id} requires validation before completion")
            }
            Self::EmptyClaimText => write!(f, "claim text must not be empty"),
            Self::EmptyIntent => write!(f, "user intent must not be empty"),
            Self::EvidenceRequired { kind, id } => {
                write!(f, "{kind} {id} requires at least one evidence id")
            }
            Self::MemoryCandidateIneligibleForPromotion {
                candidate_id,
                confidence_milli,
                minimum_confidence_milli,
                reason,
            } => write!(
                f,
                "memory candidate {candidate_id} cannot be promoted ({reason}): {confidence_milli} < {minimum_confidence_milli}"
            ),
            Self::InvalidSequence { expected, actual } => {
                write!(
                    f,
                    "invalid event sequence: expected {expected}, got {actual}"
                )
            }
            Self::InvalidEventId { expected, actual } => {
                write!(f, "invalid event id: expected {expected}, got {actual}")
            }
            Self::InvalidConfidence { max, actual } => {
                write!(f, "invalid confidence: max {max}, got {actual}")
            }
            Self::ArtifactMismatch { expected, actual } => {
                write!(f, "artifact mismatch: expected {expected}, got {actual}")
            }
            Self::ValidationFailed { task_id } => {
                write!(f, "task {task_id} validation failed")
            }
            Self::ValidationWarningsRequired { task_id } => {
                write!(
                    f,
                    "task {task_id} completed with warnings but validation report has none"
                )
            }
            Self::ValidationWarningsForbidden { task_id } => {
                write!(
                    f,
                    "task {task_id} completed verified but validation report has warnings"
                )
            }
            Self::PendingChunksExist { artifact_id } => {
                write!(
                    f,
                    "artifact {artifact_id} still has pending full-text chunks"
                )
            }
            Self::MalformedDeterministicEvidence {
                evidence_id,
                reason,
            } => write!(
                f,
                "malformed deterministic evidence {evidence_id}: {reason}"
            ),
            Self::InternalInvariantViolation { detail } => {
                write!(f, "internal invariant violation: {detail}")
            }
        }
    }
}

impl std::error::Error for DomainError {}
