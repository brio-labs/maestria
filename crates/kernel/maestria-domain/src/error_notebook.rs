use std::fmt;

use super::DomainError;

impl DomainError {
    pub(super) fn fmt_validation(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
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
            _ => Err(fmt::Error),
        }
    }

    pub(super) fn fmt_notebook(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::MissingNotebook { id } => write!(f, "missing notebook {id}"),
            Self::MissingNotebookDraft { id } => write!(f, "missing notebook draft {id}"),
            Self::NotebookSourceUnavailable { key } => {
                write!(f, "notebook source unavailable: {key}")
            }
            Self::NotebookDraftRevisionConflict {
                notebook_id,
                draft_id,
                expected,
                actual,
            } => write!(
                f,
                "notebook {notebook_id} draft {draft_id:?} revision conflict: expected {expected:?}, actual {actual:?}"
            ),
            Self::InvalidNotebookDraft { reason } => {
                write!(f, "invalid notebook draft: {reason}")
            }
            Self::InternalInvariantViolation { detail } => {
                write!(f, "internal invariant violation: {detail}")
            }
            _ => Err(fmt::Error),
        }
    }
}
