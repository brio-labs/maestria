use crate::error::{CoreError, CoreResult};
use maestria_domain::{ContentHash, NotebookDraft, content_hash};
use maestria_ports::BlobStore;
const MAX_NOTEBOOK_DRAFT_BYTES: usize = 32 * 1024;

pub fn open_notebook_draft_body(
    blobs: &dyn BlobStore,
    draft: &NotebookDraft,
) -> CoreResult<String> {
    let bytes = blobs.get(draft.body_blob).map_err(|error| match error {
        maestria_ports::PortError::NotFound => CoreError::NotFound {
            message: format!("notebook draft blob {}", draft.body_blob),
        },
        other => CoreError::Port(other),
    })?;
    if bytes.len() > MAX_NOTEBOOK_DRAFT_BYTES {
        return Err(CoreError::BlobIntegrity {
            message: "notebook draft body exceeds 32 KiB".to_owned(),
        });
    }
    let actual_hash =
        ContentHash::new(content_hash(&bytes)).map_err(|error| CoreError::BlobIntegrity {
            message: format!("cannot hash notebook draft body: {error}"),
        })?;
    if actual_hash != draft.body_hash {
        return Err(CoreError::BlobIntegrity {
            message: format!("notebook draft body hash mismatch for {}", draft.id),
        });
    }
    String::from_utf8(bytes).map_err(|error| CoreError::BlobIntegrity {
        message: format!(
            "notebook draft body is not valid UTF-8 at byte {}: {}",
            error.utf8_error().valid_up_to(),
            error
        ),
    })
}
