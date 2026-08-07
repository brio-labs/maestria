use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use super::evidence_payloads::StoredEvidenceKind;
use super::stored_content::StoredContentHash;
use maestria_domain::{
    ArtifactId, DomainEvent, EvidenceId, FrozenNotebookCitation, LogicalTick,
    NotebookDraftRevision, NotebookDraftTitle, NotebookTitle,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredNotebookCitation {
    pub evidence_id: u64,
    pub artifact_id: u64,
    pub artifact_title: String,
    pub artifact_content_hash: StoredContentHash,
    pub source: StoredEvidenceKind,
    pub excerpt: String,
    pub observed_at: u64,
}

impl StoredNotebookCitation {
    pub(crate) fn from_domain(citation: &FrozenNotebookCitation) -> Self {
        Self {
            evidence_id: citation.evidence_id.value(),
            artifact_id: citation.artifact_id.value(),
            artifact_title: citation.artifact_title.clone(),
            artifact_content_hash: StoredContentHash::from_domain(&citation.artifact_content_hash),
            source: StoredEvidenceKind::from_domain(&citation.source),
            excerpt: citation.excerpt.clone(),
            observed_at: citation.observed_at.value(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<FrozenNotebookCitation, PortError> {
        let citation = FrozenNotebookCitation {
            evidence_id: EvidenceId::new(self.evidence_id),
            artifact_id: ArtifactId::new(self.artifact_id),
            artifact_title: self.artifact_title,
            artifact_content_hash: self.artifact_content_hash.try_into_domain()?,
            source: self.source.try_into_domain()?,
            excerpt: self.excerpt,
            observed_at: LogicalTick::new(self.observed_at),
        };
        citation
            .validate()
            .map_err(|error| PortError::InvalidInput {
                message: error.to_string(),
            })?;
        Ok(citation)
    }
}

impl StoredEventPayload {
    pub(crate) fn try_from_domain_notebook(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::NotebookCreated {
                notebook_id,
                title,
                created_at,
                updated_at,
            } => Some(Self::NotebookCreated {
                notebook_id: notebook_id.value(),
                title: title.to_string(),
                created_at: created_at.value(),
                updated_at: updated_at.value(),
            }),
            DomainEvent::NotebookRenamed {
                notebook_id,
                title,
                updated_at,
            } => Some(Self::NotebookRenamed {
                notebook_id: notebook_id.value(),
                title: title.to_string(),
                updated_at: updated_at.value(),
            }),
            DomainEvent::NotebookDeleted { notebook_id } => Some(Self::NotebookDeleted {
                notebook_id: notebook_id.value(),
            }),
            DomainEvent::NotebookSourceAttached {
                notebook_id,
                source_key,
                updated_at,
            } => Some(Self::NotebookSourceAttached {
                notebook_id: notebook_id.value(),
                source_key: source_key.to_string(),
                updated_at: updated_at.value(),
            }),
            DomainEvent::NotebookSourceDetached {
                notebook_id,
                source_key,
                updated_at,
            } => Some(Self::NotebookSourceDetached {
                notebook_id: notebook_id.value(),
                source_key: source_key.to_string(),
                updated_at: updated_at.value(),
            }),
            DomainEvent::NotebookDraftSaved {
                draft_id,
                notebook_id,
                title,
                body_blob,
                body_hash,
                revision,
                citations,
                created_at,
                updated_at,
            } => Some(Self::NotebookDraftSaved {
                draft_id: draft_id.value(),
                notebook_id: notebook_id.value(),
                title: title.to_string(),
                body_blob: body_blob.value(),
                body_hash: StoredContentHash::from_domain(body_hash),
                revision: revision.value(),
                citations: citations
                    .iter()
                    .map(StoredNotebookCitation::from_domain)
                    .collect(),
                created_at: created_at.value(),
                updated_at: updated_at.value(),
            }),
            DomainEvent::NotebookDraftDeleted {
                notebook_id,
                draft_id,
                revision,
            } => Some(Self::NotebookDraftDeleted {
                notebook_id: notebook_id.value(),
                draft_id: draft_id.value(),
                revision: revision.value(),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_notebook(self) -> Result<DomainEvent, FamilyDecodeError> {
        let invalid =
            |message: String| FamilyDecodeError::Invalid(PortError::InvalidInput { message });
        match self {
            Self::NotebookCreated {
                notebook_id,
                title,
                created_at,
                updated_at,
            } => Ok(DomainEvent::NotebookCreated {
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
                title: NotebookTitle::try_from(title)
                    .map_err(|error| invalid(error.to_string()))?,
                created_at: LogicalTick::new(created_at),
                updated_at: LogicalTick::new(updated_at),
            }),
            Self::NotebookRenamed {
                notebook_id,
                title,
                updated_at,
            } => Ok(DomainEvent::NotebookRenamed {
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
                title: NotebookTitle::try_from(title)
                    .map_err(|error| invalid(error.to_string()))?,
                updated_at: LogicalTick::new(updated_at),
            }),
            Self::NotebookDeleted { notebook_id } => Ok(DomainEvent::NotebookDeleted {
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
            }),
            Self::NotebookSourceAttached {
                notebook_id,
                source_key,
                updated_at,
            } => Ok(DomainEvent::NotebookSourceAttached {
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
                source_key: maestria_domain::SourceIdentityKey::try_from(source_key)
                    .map_err(|error| invalid(error.to_string()))?,
                updated_at: LogicalTick::new(updated_at),
            }),
            Self::NotebookSourceDetached {
                notebook_id,
                source_key,
                updated_at,
            } => Ok(DomainEvent::NotebookSourceDetached {
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
                source_key: maestria_domain::SourceIdentityKey::try_from(source_key)
                    .map_err(|error| invalid(error.to_string()))?,
                updated_at: LogicalTick::new(updated_at),
            }),
            Self::NotebookDraftSaved {
                draft_id,
                notebook_id,
                title,
                body_blob,
                body_hash,
                revision,
                citations,
                created_at,
                updated_at,
            } => Ok(DomainEvent::NotebookDraftSaved {
                draft_id: maestria_domain::NotebookDraftId::new(draft_id),
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
                title: NotebookDraftTitle::try_from(title)
                    .map_err(|error| invalid(error.to_string()))?,
                body_blob: maestria_domain::BlobId::new(body_blob),
                body_hash: body_hash
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
                revision: NotebookDraftRevision::try_from(revision)
                    .map_err(|error| invalid(error.to_string()))?,
                citations: citations
                    .into_iter()
                    .map(StoredNotebookCitation::try_into_domain)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(FamilyDecodeError::Invalid)?,
                created_at: LogicalTick::new(created_at),
                updated_at: LogicalTick::new(updated_at),
            }),
            Self::NotebookDraftDeleted {
                notebook_id,
                draft_id,
                revision,
            } => Ok(DomainEvent::NotebookDraftDeleted {
                notebook_id: maestria_domain::NotebookId::new(notebook_id),
                draft_id: maestria_domain::NotebookDraftId::new(draft_id),
                revision: NotebookDraftRevision::try_from(revision)
                    .map_err(|error| invalid(error.to_string()))?,
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    pub(crate) fn try_kind_notebook(&self) -> Option<&'static str> {
        match self {
            Self::NotebookCreated { .. } => Some("notebook_created"),
            Self::NotebookRenamed { .. } => Some("notebook_renamed"),
            Self::NotebookDeleted { .. } => Some("notebook_deleted"),
            Self::NotebookSourceAttached { .. } => Some("notebook_source_attached"),
            Self::NotebookSourceDetached { .. } => Some("notebook_source_detached"),
            Self::NotebookDraftSaved { .. } => Some("notebook_draft_saved"),
            Self::NotebookDraftDeleted { .. } => Some("notebook_draft_deleted"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_notebook(&self) -> Option<u64> {
        None
    }
}
