use crate::error::{CoreError, CoreResult};
use crate::provenance::{artifact_id_for, content_hash, title_for_path};
use maestria_domain::{ArtifactDetected, DomainInput};
use std::path::Path;

/// Build a deterministic [`DomainInput::ArtifactDetected`] from a file path and raw bytes.
///
/// Validates that bytes are non-empty. Uses deterministic core helpers for
/// artifact ID, content hash, and title so that the same path and bytes always
/// produce the same input—enabling replay, deduplication, and pure reducer
/// composition.
pub fn build_artifact_detected_input(
    source_path: &Path,
    source_bytes: Vec<u8>,
) -> CoreResult<DomainInput> {
    if source_bytes.is_empty() {
        return Err(CoreError::InvalidInput {
            message: "source bytes must not be empty".to_string(),
        });
    }

    let artifact_id = artifact_id_for(source_path, &source_bytes);
    let content_hash = content_hash(&source_bytes);
    let title = title_for_path(source_path);

    Ok(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title,
        source_path: source_path.display().to_string(),
        source_bytes,
        content_hash,
    }))
}
