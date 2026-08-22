use crate::types::*;
use std::sync::Arc;
const MAX_NOTEBOOK_DRAFT_BYTES: usize = 32 * 1024;
impl KernelState {
    pub(super) fn apply_notebook_draft_saved(&mut self, draft: NotebookDraft) {
        let notebook_id = draft.notebook_id;
        let updated_at = draft.updated_at;
        Arc::make_mut(&mut self.notebook_drafts).insert(draft.id, draft);
        if let Some(notebook) = Arc::make_mut(&mut self.notebooks).get_mut(&notebook_id) {
            notebook.updated_at = updated_at;
        }
    }
    pub(super) fn next_notebook_id(&self) -> Result<NotebookId, DomainError> {
        let previous = self
            .notebooks
            .keys()
            .next_back()
            .map(NotebookId::value)
            .into_iter()
            .chain(
                self.event_log
                    .iter()
                    .filter_map(|envelope| match &envelope.event {
                        DomainEvent::NotebookCreated { notebook_id, .. } => {
                            Some(notebook_id.value())
                        }
                        _ => None,
                    }),
            )
            .max();
        next_id(previous, NotebookIdKind::Notebook).map(NotebookId::new)
    }

    pub(super) fn next_draft_id(&self) -> Result<NotebookDraftId, DomainError> {
        let previous = self
            .notebook_drafts
            .keys()
            .next_back()
            .map(NotebookDraftId::value)
            .into_iter()
            .chain(
                self.event_log
                    .iter()
                    .filter_map(|envelope| match &envelope.event {
                        DomainEvent::NotebookDraftSaved { draft_id, .. } => Some(draft_id.value()),
                        _ => None,
                    }),
            )
            .max();
        next_id(previous, NotebookIdKind::Draft).map(NotebookDraftId::new)
    }

    pub(super) fn current_notebook_tick(&self) -> LogicalTick {
        if let Some(at) = self.current_tick {
            return at;
        }
        // No tick observed yet: fall back to the last event's sequence,
        // preserving the pre-cache semantics.
        match self.event_log.last() {
            Some(envelope) => LogicalTick::new(envelope.id.value()),
            None => LogicalTick::new(0),
        }
    }
}
enum NotebookIdKind {
    Notebook,
    Draft,
}
fn next_id(previous: Option<u64>, kind: NotebookIdKind) -> Result<u64, DomainError> {
    match previous {
        Some(value) => value.checked_add(1).ok_or_else(|| match kind {
            NotebookIdKind::Notebook => DomainError::DuplicateNotebook {
                id: NotebookId::new(value),
            },
            NotebookIdKind::Draft => DomainError::DuplicateNotebookDraft {
                id: NotebookDraftId::new(value),
            },
        }),
        None => Ok(1),
    }
}

pub(super) fn notebook_title(value: String) -> Result<NotebookTitle, DomainError> {
    NotebookTitle::try_from(value).map_err(invalid_draft)
}

pub(super) fn draft_title(value: String) -> Result<NotebookDraftTitle, DomainError> {
    NotebookDraftTitle::try_from(value).map_err(invalid_draft)
}

/// Violation reported by [`validate_draft_deletion`]. Callers map
/// `NotebookMismatch` to their own error: the live handler reports
/// `MissingNotebookDraft` while replay reports `NotebookDraftRevisionConflict`
/// (observable contract).
pub(crate) enum DraftDeletionViolation {
    MissingNotebookDraft,
    NotebookMismatch { actual: NotebookDraftRevision },
    RevisionMismatch { actual: NotebookDraftRevision },
}

/// Shared notebook-draft deletion validation used by both the live handler
/// and the replay applier.
pub(crate) fn validate_draft_deletion(
    state: &KernelState,
    notebook_id: NotebookId,
    draft_id: NotebookDraftId,
    expected_revision: NotebookDraftRevision,
) -> Result<(), DraftDeletionViolation> {
    let draft = state
        .notebook_drafts
        .get(&draft_id)
        .ok_or(DraftDeletionViolation::MissingNotebookDraft)?;
    if draft.notebook_id != notebook_id {
        return Err(DraftDeletionViolation::NotebookMismatch {
            actual: draft.revision,
        });
    }
    if draft.revision != expected_revision {
        return Err(DraftDeletionViolation::RevisionMismatch {
            actual: draft.revision,
        });
    }
    Ok(())
}

pub(super) fn source_key(value: String) -> Result<SourceIdentityKey, DomainError> {
    SourceIdentityKey::try_from(value).map_err(|error| DomainError::InvalidSourceIdentityKey {
        reason: error.to_string(),
    })
}

pub(super) fn validate_draft_body(body: &str) -> Result<(), DomainError> {
    if body.trim().is_empty() {
        return Err(invalid_draft("body must not be empty"));
    }
    if body.len() > MAX_NOTEBOOK_DRAFT_BYTES {
        return Err(invalid_draft("body exceeds 32 KiB"));
    }
    Ok(())
}

pub(super) fn invalid_draft(error: impl ToString) -> DomainError {
    DomainError::InvalidNotebookDraft {
        reason: error.to_string(),
    }
}
