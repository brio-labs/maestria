use crate::ids::BlobId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateNotebookInput {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameNotebookInput {
    pub notebook_id: crate::ids::NotebookId,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteNotebookInput {
    pub notebook_id: crate::ids::NotebookId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachNotebookSourceInput {
    pub notebook_id: crate::ids::NotebookId,
    pub source_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachNotebookSourceInput {
    pub notebook_id: crate::ids::NotebookId,
    pub source_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveNotebookDraftRequested {
    pub notebook_id: crate::ids::NotebookId,
    pub draft_id: Option<crate::ids::NotebookDraftId>,
    pub expected_revision: Option<crate::notebook::NotebookDraftRevision>,
    pub title: String,
    pub body: String,
    pub citations: Vec<crate::notebook::FrozenNotebookCitation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookDraftBlobStored {
    pub notebook_id: crate::ids::NotebookId,
    pub draft_id: Option<crate::ids::NotebookDraftId>,
    pub expected_revision: Option<crate::notebook::NotebookDraftRevision>,
    pub title: crate::notebook::NotebookDraftTitle,
    pub blob_id: BlobId,
    pub content_hash: crate::search::ContentHash,
    pub citations: Vec<crate::notebook::FrozenNotebookCitation>,
    pub correlation_id: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotebookDraftBlobStoreFailed {
    pub correlation_id: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteNotebookDraftInput {
    pub notebook_id: crate::ids::NotebookId,
    pub draft_id: crate::ids::NotebookDraftId,
    pub expected_revision: crate::notebook::NotebookDraftRevision,
}
