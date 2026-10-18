use anyhow::{Context, Result};
use sillage_blob_fs::FsBlobStore;
use sillage_core::open_notebook_draft_body;
use sillage_domain::NotebookDraft;

pub(crate) fn open(
    context: &crate::api::server::ApiContext,
    draft: &NotebookDraft,
) -> Result<String> {
    let blobs = FsBlobStore::open(&context.layout.blobs_dir).with_context(|| {
        format!(
            "open instance blob store {}",
            context.layout.blobs_dir.display()
        )
    })?;
    open_notebook_draft_body(&blobs, draft).map_err(|error| anyhow::anyhow!(error.to_string()))
}
