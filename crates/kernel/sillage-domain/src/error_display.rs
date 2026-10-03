use super::DomainError;
use crate::ids::{TaskId, ValidationReportId};
use std::fmt;

impl DomainError {
    fn fmt_missing(f: &mut fmt::Formatter, kind: &str, id: impl fmt::Display) -> fmt::Result {
        write!(f, "missing {kind} {id}")
    }

    fn fmt_duplicate(f: &mut fmt::Formatter, kind: &str, id: impl fmt::Display) -> fmt::Result {
        write!(f, "duplicate {kind} id: {id}")
    }

    fn fmt_validation_report_task_mismatch(
        f: &mut fmt::Formatter,
        report_id: ValidationReportId,
        report_task_id: Option<TaskId>,
        task_id: TaskId,
    ) -> fmt::Result {
        match report_task_id {
            Some(report_task_id) => write!(
                f,
                "validation report {report_id} is for task {report_task_id}, not {task_id}"
            ),
            None => write!(
                f,
                "validation report {report_id} is not associated with task {task_id}"
            ),
        }
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

    fn fmt_validation_required(f: &mut fmt::Formatter, task_id: TaskId) -> fmt::Result {
        write!(f, "task {task_id} requires validation before completion")
    }
    fn fmt_model_agent(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (message, run_id) = match self {
            Self::DuplicateModelAgentProposalRunId { run_id } => {
                ("duplicate model-agent proposal run id", run_id)
            }
            Self::ModelAgentProposalRequestNotFresh { run_id } => {
                ("model-agent proposal request must be fresh", run_id)
            }
            Self::ModelAgentProposalResumeMismatch { run_id } => (
                "model-agent proposal resume does not match its canonical request",
                run_id,
            ),
            Self::ModelAgentProposalNotResumable { run_id } => {
                ("model-agent proposal is missing or terminal", run_id)
            }
            _ => return Err(fmt::Error),
        };
        write!(f, "{message}: {run_id}")
    }

    fn fmt_memory_candidate(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Self::MemoryCandidateIneligibleForPromotion {
            candidate_id,
            confidence_milli,
            minimum_confidence_milli,
            reason,
        } = self
        else {
            return Err(fmt::Error);
        };
        write!(
            f,
            "memory candidate {candidate_id} cannot be promoted ({reason}): {confidence_milli} < {minimum_confidence_milli}"
        )
    }

    fn fmt_realm_read_grant(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::DuplicateRealmReadGrantDigest { digest } => {
                write!(f, "duplicate realm read grant digest: {digest}")
            }
            Self::DuplicateActiveRealmReadGrant { consumer_realm } => {
                write!(
                    f,
                    "consumer realm already has an active read grant: {consumer_realm}"
                )
            }
            Self::MissingRealmReadGrant { digest } => {
                write!(f, "missing realm read grant: {digest}")
            }
            Self::RealmReadGrantAlreadyRevoked { digest } => {
                write!(f, "realm read grant is already revoked: {digest}")
            }
            Self::RealmReadGrantRevoked { digest } => {
                write!(f, "realm read grant is revoked: {digest}")
            }
            Self::RealmReadGrantUnsupportedAccess { digest } => {
                write!(f, "realm read grant does not allow this access: {digest}")
            }
            Self::RealmReadGrantProviderMismatch { expected, actual } => {
                write!(
                    f,
                    "realm read grant provider mismatch: expected {expected}, got {actual}"
                )
            }
            Self::RealmReadGrantConsumerMismatch { expected, actual } => {
                write!(
                    f,
                    "realm read grant consumer mismatch: expected {expected}, got {actual}"
                )
            }
            _ => Err(fmt::Error),
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateArtifact { id } => Self::fmt_duplicate(f, "artifact", id),
            Self::DuplicateChunk { id } => Self::fmt_duplicate(f, "chunk", id),
            Self::DuplicateChunkOrder { id } => Self::fmt_duplicate(f, "chunk_order", id),
            Self::DuplicateCard { id } => Self::fmt_duplicate(f, "card", id),
            Self::DuplicateClaim { id } => Self::fmt_duplicate(f, "claim", id),
            Self::DuplicateEvidenceInClaim { id } => {
                Self::fmt_duplicate(f, "evidence_in_claim", id)
            }
            Self::DuplicateEvidenceClaim { id } => Self::fmt_duplicate(f, "evidence_claim", id),
            Self::DuplicateEvidence { id } => Self::fmt_duplicate(f, "evidence", id),
            Self::DuplicateMemoryCandidate { id } => Self::fmt_duplicate(f, "memory_candidate", id),
            Self::DuplicateMemory { id } => Self::fmt_duplicate(f, "memory", id),
            Self::DuplicateRelation { id } => Self::fmt_duplicate(f, "relation", id),
            Self::DuplicateTask { id } => Self::fmt_duplicate(f, "task", id),
            Self::DuplicateValidationReport { id } => {
                Self::fmt_duplicate(f, "validation_report", id)
            }
            Self::DuplicateNotebook { id } => Self::fmt_duplicate(f, "notebook", id),
            Self::DuplicateNotebookDraft { id } => Self::fmt_duplicate(f, "notebook draft", id),
            Self::DuplicateIndexGeneration { id } => Self::fmt_duplicate(f, "IndexGeneration", id),
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
            } => {
                Self::fmt_validation_report_task_mismatch(f, *report_id, *report_task_id, *task_id)
            }
            Self::InvalidTaskTransition { task_id, from, to } => {
                Self::fmt_transition(f, "invalid task transition", task_id, from, to)
            }
            Self::InvalidGenerationTransition { id, from, to } => {
                Self::fmt_transition(f, "invalid index generation transition for", id, from, to)
            }
            Self::ValidationRequired { task_id } => Self::fmt_validation_required(f, *task_id),

            error @ (Self::DuplicateModelAgentProposalRunId { .. }
            | Self::ModelAgentProposalRequestNotFresh { .. }
            | Self::ModelAgentProposalResumeMismatch { .. }
            | Self::ModelAgentProposalNotResumable { .. }) => error.fmt_model_agent(f),
            error @ Self::MemoryCandidateIneligibleForPromotion { .. } => {
                error.fmt_memory_candidate(f)
            }
            error @ (Self::ValidationWarningsRequired { .. }
            | Self::ValidationWarningsForbidden { .. }
            | Self::PendingChunksExist { .. }) => error.fmt_validation(f),
            error @ (Self::DuplicateRealmReadGrantDigest { .. }
            | Self::DuplicateActiveRealmReadGrant { .. }
            | Self::MissingRealmReadGrant { .. }
            | Self::RealmReadGrantAlreadyRevoked { .. }
            | Self::RealmReadGrantRevoked { .. }
            | Self::RealmReadGrantUnsupportedAccess { .. }
            | Self::RealmReadGrantProviderMismatch { .. }
            | Self::RealmReadGrantConsumerMismatch { .. }) => error.fmt_realm_read_grant(f),
            error @ (Self::MissingNotebook { .. }
            | Self::MissingNotebookDraft { .. }
            | Self::NotebookSourceUnavailable { .. }
            | Self::NotebookSourceArtifactUnavailable { .. }
            | Self::InvalidSourceIdentityKey { .. }
            | Self::NotebookDraftRevisionConflict { .. }
            | Self::InvalidNotebookDraft { .. }
            | Self::InternalInvariantViolation { .. }) => error.fmt_notebook(f),

            error @ (Self::MemorySupersedesItself { .. }
            | Self::EmptyClaimText
            | Self::EmptyIntent
            | Self::EmptyRetirementReason
            | Self::MemoryCandidateRequiresEvidence { .. }
            | Self::ArtifactIndexedRequiresEvidence { .. }
            | Self::InvalidEventId { .. }
            | Self::InvalidConfidence { .. }
            | Self::ArtifactMismatch { .. }
            | Self::ValidationFailed { .. }
            | Self::MalformedDeterministicEvidence { .. }
            | Self::SearchIncompatible { .. }) => error.fmt_specialized(f),
        }
    }
}

impl DomainError {
    fn fmt_specialized(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemorySupersedesItself { memory_id } => {
                write!(f, "memory {memory_id} cannot supersede itself")
            }
            Self::EmptyClaimText => write!(f, "claim text must not be empty"),
            Self::EmptyIntent => write!(f, "user intent must not be empty"),
            Self::EmptyRetirementReason => {
                write!(f, "retirement reason must not be empty")
            }
            Self::MemoryCandidateRequiresEvidence { id } => {
                write!(f, "memory_candidate {id} requires at least one evidence id")
            }
            Self::ArtifactIndexedRequiresEvidence { id } => {
                write!(f, "artifact_indexed {id} requires at least one evidence id")
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
            Self::MalformedDeterministicEvidence {
                evidence_id,
                reason,
            } => write!(
                f,
                "malformed deterministic evidence {evidence_id}: {reason}"
            ),
            Self::SearchIncompatible { error } => {
                write!(f, "search contract violation: {error}")
            }

            // Unreachable: [`Self::fmt`] routes every variant through one of
            // the one-liner, multi-arm, family, or inline-write arms above.
            _ => Err(fmt::Error),
        }
    }
}

impl std::error::Error for DomainError {}
