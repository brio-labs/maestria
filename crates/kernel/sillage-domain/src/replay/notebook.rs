use crate::input::notebook_support::DraftDeletionViolation;
use crate::types::*;
use std::collections::BTreeSet;
use std::sync::Arc;

impl KernelState {
    pub(super) fn replay_notebook_event(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        match event {
            DomainEvent::NotebookCreated {
                notebook_id,
                title,
                created_at,
                updated_at,
            } => self.replay_notebook_created(notebook_id, title, *created_at, *updated_at),
            DomainEvent::NotebookRenamed {
                notebook_id,
                title,
                updated_at,
            } => self.replay_notebook_renamed(notebook_id, title, *updated_at),
            DomainEvent::NotebookDeleted { notebook_id } => {
                self.replay_notebook_deleted(notebook_id)
            }
            DomainEvent::NotebookSourceAttached {
                notebook_id,
                source_key,
                updated_at,
            } => self.replay_notebook_source_attached(notebook_id, source_key, *updated_at),
            DomainEvent::NotebookSourceDetached {
                notebook_id,
                source_key,
                updated_at,
            } => self.replay_notebook_source_detached(notebook_id, source_key, *updated_at),
            DomainEvent::NotebookDraftSaved { .. } => self.replay_notebook_draft_saved(event),
            DomainEvent::NotebookDraftDeleted {
                notebook_id,
                draft_id,
                revision,
            } => self.replay_notebook_draft_deleted(notebook_id, draft_id, *revision),
            _ => Err(DomainError::InternalInvariantViolation {
                detail: "replay_notebook_event: unexpected event variant",
            }),
        }
    }

    fn replay_notebook_created(
        &mut self,
        notebook_id: &NotebookId,
        title: &NotebookTitle,
        created_at: LogicalTick,
        updated_at: LogicalTick,
    ) -> Result<(), DomainError> {
        if self.notebooks.contains_key(notebook_id) {
            return Err(DomainError::DuplicateNotebook { id: *notebook_id });
        }
        Arc::make_mut(&mut self.notebooks).insert(
            *notebook_id,
            Notebook {
                id: *notebook_id,
                title: title.clone(),
                source_keys: BTreeSet::new(),
                created_at,
                updated_at,
            },
        );
        Ok(())
    }

    fn replay_notebook_renamed(
        &mut self,
        notebook_id: &NotebookId,
        title: &NotebookTitle,
        updated_at: LogicalTick,
    ) -> Result<(), DomainError> {
        let notebook = Arc::make_mut(&mut self.notebooks)
            .get_mut(notebook_id)
            .ok_or(DomainError::MissingNotebook { id: *notebook_id })?;
        notebook.title = title.clone();
        notebook.updated_at = updated_at;
        Ok(())
    }

    fn replay_notebook_deleted(&mut self, notebook_id: &NotebookId) -> Result<(), DomainError> {
        if Arc::make_mut(&mut self.notebooks)
            .remove(notebook_id)
            .is_none()
        {
            return Err(DomainError::MissingNotebook { id: *notebook_id });
        }
        Arc::make_mut(&mut self.notebook_drafts)
            .retain(|_, draft| draft.notebook_id != *notebook_id);
        Ok(())
    }

    fn replay_notebook_source_attached(
        &mut self,
        notebook_id: &NotebookId,
        source_key: &SourceIdentityKey,
        updated_at: LogicalTick,
    ) -> Result<(), DomainError> {
        let notebook = Arc::make_mut(&mut self.notebooks)
            .get_mut(notebook_id)
            .ok_or(DomainError::MissingNotebook { id: *notebook_id })?;
        notebook.source_keys.insert(source_key.clone());
        notebook.updated_at = updated_at;
        Ok(())
    }

    fn replay_notebook_source_detached(
        &mut self,
        notebook_id: &NotebookId,
        source_key: &SourceIdentityKey,
        updated_at: LogicalTick,
    ) -> Result<(), DomainError> {
        let notebook = Arc::make_mut(&mut self.notebooks)
            .get_mut(notebook_id)
            .ok_or(DomainError::MissingNotebook { id: *notebook_id })?;
        notebook.source_keys.remove(source_key);
        notebook.updated_at = updated_at;
        Ok(())
    }

    fn replay_notebook_draft_saved(&mut self, event: &DomainEvent) -> Result<(), DomainError> {
        let DomainEvent::NotebookDraftSaved {
            draft_id,
            notebook_id,
            title,
            body_blob,
            body_hash,
            revision,
            citations,
            created_at,
            updated_at,
        } = event
        else {
            return Err(DomainError::InternalInvariantViolation {
                detail: "replay_notebook_draft_saved: unexpected event variant",
            });
        };
        if !self.notebooks.contains_key(notebook_id) {
            return Err(DomainError::MissingNotebook { id: *notebook_id });
        }
        validate_frozen_citations(citations).map_err(|error| {
            DomainError::InvalidNotebookDraft {
                reason: error.to_string(),
            }
        })?;
        if let Some(existing) = self.notebook_drafts.get(draft_id)
            && existing.notebook_id != *notebook_id
        {
            return Err(DomainError::MissingNotebookDraft { id: *draft_id });
        }
        Arc::make_mut(&mut self.notebook_drafts).insert(
            *draft_id,
            NotebookDraft {
                id: *draft_id,
                notebook_id: *notebook_id,
                title: title.clone(),
                body_blob: *body_blob,
                body_hash: body_hash.clone(),
                revision: *revision,
                citations: citations.to_vec(),
                created_at: *created_at,
                updated_at: *updated_at,
            },
        );
        if let Some(notebook) = Arc::make_mut(&mut self.notebooks).get_mut(notebook_id) {
            notebook.updated_at = *updated_at;
        }
        Ok(())
    }

    fn replay_notebook_draft_deleted(
        &mut self,
        notebook_id: &NotebookId,
        draft_id: &NotebookDraftId,
        revision: NotebookDraftRevision,
    ) -> Result<(), DomainError> {
        match crate::input::notebook_support::validate_draft_deletion(
            self,
            *notebook_id,
            *draft_id,
            revision,
        ) {
            Ok(()) => {}
            Err(DraftDeletionViolation::MissingNotebookDraft) => {
                return Err(DomainError::MissingNotebookDraft { id: *draft_id });
            }
            Err(
                DraftDeletionViolation::NotebookMismatch { actual }
                | DraftDeletionViolation::RevisionMismatch { actual },
            ) => {
                return Err(DomainError::NotebookDraftRevisionConflict {
                    notebook_id: *notebook_id,
                    draft_id: Some(*draft_id),
                    expected: Some(revision.value()),
                    actual: Some(actual.value()),
                });
            }
        }
        Arc::make_mut(&mut self.notebook_drafts).remove(draft_id);
        Ok(())
    }
}
