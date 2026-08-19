use super::notebook_support::{
    DraftDeletionViolation, draft_title, invalid_draft, notebook_title, source_key,
    validate_draft_body, validate_draft_deletion,
};
use crate::notebook_inputs::*;
use crate::types::*;
use std::collections::BTreeSet;

impl KernelState {
    pub(super) fn process_create_notebook(
        &mut self,
        input: CreateNotebookInput,
    ) -> Result<KernelOutput, DomainError> {
        let title = notebook_title(input.title)?;
        let notebook_id = self.next_notebook_id()?;
        let tick = self.current_notebook_tick();
        let event = self.emit_event(DomainEvent::NotebookCreated {
            notebook_id,
            title: title.clone(),
            created_at: tick,
            updated_at: tick,
        });
        self.notebooks.insert(
            notebook_id,
            Notebook {
                id: notebook_id,
                title,
                source_keys: BTreeSet::new(),
                created_at: tick,
                updated_at: tick,
            },
        );
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_rename_notebook(
        &mut self,
        input: RenameNotebookInput,
    ) -> Result<KernelOutput, DomainError> {
        let title = notebook_title(input.title)?;
        let tick = self.current_notebook_tick();
        let notebook =
            self.notebooks
                .get_mut(&input.notebook_id)
                .ok_or(DomainError::MissingNotebook {
                    id: input.notebook_id,
                })?;
        notebook.title = title.clone();
        notebook.updated_at = tick;
        let event = self.emit_event(DomainEvent::NotebookRenamed {
            notebook_id: input.notebook_id,
            title,
            updated_at: tick,
        });
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_delete_notebook(
        &mut self,
        input: DeleteNotebookInput,
    ) -> Result<KernelOutput, DomainError> {
        if !self.notebooks.contains_key(&input.notebook_id) {
            return Err(DomainError::MissingNotebook {
                id: input.notebook_id,
            });
        }
        let event = self.emit_event(DomainEvent::NotebookDeleted {
            notebook_id: input.notebook_id,
        });
        self.notebooks.remove(&input.notebook_id);
        self.notebook_drafts
            .retain(|_, draft| draft.notebook_id != input.notebook_id);
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_attach_notebook_source(
        &mut self,
        input: AttachNotebookSourceInput,
    ) -> Result<KernelOutput, DomainError> {
        let source_key = source_key(input.source_key)?;
        let already_attached = self
            .notebooks
            .get(&input.notebook_id)
            .ok_or(DomainError::MissingNotebook {
                id: input.notebook_id,
            })?
            .source_keys
            .contains(&source_key);
        if already_attached {
            return Ok(KernelOutput::default());
        }
        let Some(artifact_id) = self.active_sources.get(&source_key).copied() else {
            return Err(DomainError::NotebookSourceUnavailable { key: source_key });
        };
        let indexed = self
            .artifacts
            .get(&artifact_id)
            .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed);
        if !indexed {
            return Err(DomainError::NotebookSourceUnavailable { key: source_key });
        }
        let tick = self.current_notebook_tick();
        let notebook =
            self.notebooks
                .get_mut(&input.notebook_id)
                .ok_or(DomainError::MissingNotebook {
                    id: input.notebook_id,
                })?;
        notebook.source_keys.insert(source_key.clone());
        notebook.updated_at = tick;
        let event = self.emit_event(DomainEvent::NotebookSourceAttached {
            notebook_id: input.notebook_id,
            source_key,
            updated_at: tick,
        });
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_detach_notebook_source(
        &mut self,
        input: DetachNotebookSourceInput,
    ) -> Result<KernelOutput, DomainError> {
        let source_key = source_key(input.source_key)?;
        let already_attached = self
            .notebooks
            .get(&input.notebook_id)
            .ok_or(DomainError::MissingNotebook {
                id: input.notebook_id,
            })?
            .source_keys
            .contains(&source_key);
        if !already_attached {
            return Ok(KernelOutput::default());
        }
        let tick = self.current_notebook_tick();
        let notebook =
            self.notebooks
                .get_mut(&input.notebook_id)
                .ok_or(DomainError::MissingNotebook {
                    id: input.notebook_id,
                })?;
        notebook.source_keys.remove(&source_key);
        notebook.updated_at = tick;
        let event = self.emit_event(DomainEvent::NotebookSourceDetached {
            notebook_id: input.notebook_id,
            source_key,
            updated_at: tick,
        });
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_save_notebook_draft_requested(
        &mut self,
        input: SaveNotebookDraftRequested,
    ) -> Result<KernelOutput, DomainError> {
        let title = draft_title(input.title)?;
        validate_draft_body(&input.body)?;
        validate_frozen_citations(&input.citations).map_err(invalid_draft)?;
        self.validate_draft_revision_request(
            input.notebook_id,
            input.draft_id,
            input.expected_revision,
        )?;
        let request = NotebookDraftBlobRequest {
            notebook_id: input.notebook_id,
            draft_id: input.draft_id,
            expected_revision: input.expected_revision,
            title,
            body: input.body,
            citations: input.citations,
            correlation_id: None,
        };
        let mut output = KernelOutput::default();
        output
            .effects
            .push(MaestriaEffect::PersistNotebookDraftBlob(request));
        Ok(output)
    }

    pub(super) fn process_notebook_draft_blob_stored(
        &mut self,
        input: NotebookDraftBlobStored,
    ) -> Result<KernelOutput, DomainError> {
        let (draft_id, revision, created_at) = self.prepare_stored_draft(&input)?;
        let tick = self.current_notebook_tick();
        let event = self.emit_event(DomainEvent::NotebookDraftSaved {
            draft_id,
            notebook_id: input.notebook_id,
            title: input.title.clone(),
            body_blob: input.blob_id,
            body_hash: input.content_hash.clone(),
            revision,
            citations: input.citations.clone(),
            created_at,
            updated_at: tick,
        });
        self.apply_notebook_draft_saved(NotebookDraft {
            id: draft_id,
            notebook_id: input.notebook_id,
            title: input.title,
            body_blob: input.blob_id,
            body_hash: input.content_hash,
            revision,
            citations: input.citations,
            created_at,
            updated_at: tick,
        });
        Ok(Self::output_for_event(event))
    }

    pub(super) fn process_delete_notebook_draft(
        &mut self,
        input: DeleteNotebookDraftInput,
    ) -> Result<KernelOutput, DomainError> {
        match validate_draft_deletion(
            self,
            input.notebook_id,
            input.draft_id,
            input.expected_revision,
        ) {
            Ok(()) => {}
            Err(DraftDeletionViolation::MissingNotebookDraft)
            | Err(DraftDeletionViolation::NotebookMismatch { .. }) => {
                return Err(DomainError::MissingNotebookDraft { id: input.draft_id });
            }
            Err(DraftDeletionViolation::RevisionMismatch { actual }) => {
                return Err(DomainError::NotebookDraftRevisionConflict {
                    notebook_id: input.notebook_id,
                    draft_id: Some(input.draft_id),
                    expected: Some(input.expected_revision.value()),
                    actual: Some(actual.value()),
                });
            }
        }
        let event = self.emit_event(DomainEvent::NotebookDraftDeleted {
            notebook_id: input.notebook_id,
            draft_id: input.draft_id,
            revision: input.expected_revision,
        });
        self.notebook_drafts.remove(&input.draft_id);
        Ok(Self::output_for_event(event))
    }

    fn prepare_stored_draft(
        &self,
        input: &NotebookDraftBlobStored,
    ) -> Result<(NotebookDraftId, NotebookDraftRevision, LogicalTick), DomainError> {
        self.notebooks
            .get(&input.notebook_id)
            .ok_or(DomainError::MissingNotebook {
                id: input.notebook_id,
            })?;
        validate_frozen_citations(&input.citations).map_err(invalid_draft)?;
        self.validate_citation_membership(input.notebook_id, &input.citations)?;
        match (input.draft_id, input.expected_revision) {
            (None, None) => Ok((
                self.next_draft_id()?,
                NotebookDraftRevision::initial(),
                self.current_notebook_tick(),
            )),
            (Some(draft_id), Some(expected)) => {
                let draft = self
                    .notebook_drafts
                    .get(&draft_id)
                    .ok_or(DomainError::MissingNotebookDraft { id: draft_id })?;
                if draft.notebook_id != input.notebook_id {
                    return Err(DomainError::MissingNotebookDraft { id: draft_id });
                }
                if draft.revision != expected {
                    return Err(DomainError::NotebookDraftRevisionConflict {
                        notebook_id: input.notebook_id,
                        draft_id: Some(draft_id),
                        expected: Some(expected.value()),
                        actual: Some(draft.revision.value()),
                    });
                }
                let revision = expected.increment().map_err(invalid_draft)?;
                Ok((draft_id, revision, draft.created_at))
            }
            _ => Err(DomainError::NotebookDraftRevisionConflict {
                notebook_id: input.notebook_id,
                draft_id: input.draft_id,
                expected: input.expected_revision.map(NotebookDraftRevision::value),
                actual: None,
            }),
        }
    }

    fn validate_draft_revision_request(
        &self,
        notebook_id: NotebookId,
        draft_id: Option<NotebookDraftId>,
        expected_revision: Option<NotebookDraftRevision>,
    ) -> Result<(), DomainError> {
        self.notebooks
            .get(&notebook_id)
            .ok_or(DomainError::MissingNotebook { id: notebook_id })?;
        match (draft_id, expected_revision) {
            (None, None) => Ok(()),
            (Some(draft_id), Some(expected)) => {
                let draft = self
                    .notebook_drafts
                    .get(&draft_id)
                    .ok_or(DomainError::MissingNotebookDraft { id: draft_id })?;
                if draft.notebook_id != notebook_id {
                    return Err(DomainError::MissingNotebookDraft { id: draft_id });
                }
                if draft.revision != expected {
                    return Err(DomainError::NotebookDraftRevisionConflict {
                        notebook_id,
                        draft_id: Some(draft_id),
                        expected: Some(expected.value()),
                        actual: Some(draft.revision.value()),
                    });
                }
                Ok(())
            }
            _ => Err(DomainError::NotebookDraftRevisionConflict {
                notebook_id,
                draft_id,
                expected: expected_revision.map(NotebookDraftRevision::value),
                actual: None,
            }),
        }
    }

    fn validate_citation_membership(
        &self,
        notebook_id: NotebookId,
        citations: &[FrozenNotebookCitation],
    ) -> Result<(), DomainError> {
        let notebook = self
            .notebooks
            .get(&notebook_id)
            .ok_or(DomainError::MissingNotebook { id: notebook_id })?;
        for citation in citations {
            let selected = notebook.source_keys.iter().any(|key| {
                self.active_sources.get(key) == Some(&citation.artifact_id)
                    && self
                        .artifacts
                        .get(&citation.artifact_id)
                        .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
            });
            if !selected {
                return Err(DomainError::NotebookSourceArtifactUnavailable {
                    artifact_id: citation.artifact_id,
                });
            }
        }
        Ok(())
    }
}
