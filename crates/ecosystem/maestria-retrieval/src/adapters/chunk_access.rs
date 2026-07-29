use maestria_domain::{Artifact, Chunk, ChunkId, IndexStatus};
use maestria_governance::{RetrievalAuthorizationContext, RetrievalDecision, scan_secrets};
use maestria_ports::{ArtifactRepository, ChunkRepository, PortError};

/// Loads a chunk only after its owning artifact has passed retrieval checks.
///
/// The metadata-only owner lookup is intentionally the first repository access:
/// a denied or otherwise invalid owner must never cause a content-bearing chunk
/// read. The returned artifact and chunk are paired only after their ownership
/// is checked again against the full chunk row.
pub(super) fn load_authorized_chunk(
    chunks: &dyn ChunkRepository,
    artifacts: &dyn ArtifactRepository,
    chunk_id: ChunkId,
    authorization: &RetrievalAuthorizationContext,
) -> Result<Option<(Artifact, Chunk)>, PortError> {
    let Some(owner_id) = chunks.find_artifact_id(chunk_id)? else {
        return Ok(None);
    };
    let Some(artifact) = artifacts.get(owner_id)? else {
        return Ok(None);
    };
    if artifact.index_status != IndexStatus::Indexed
        || authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
    {
        return Ok(None);
    }
    let Some(chunk) = chunks.get(chunk_id)? else {
        return Ok(None);
    };
    if chunk.artifact_id != owner_id {
        return Err(PortError::Conflict {
            message: format!(
                "chunk {chunk_id} owner mismatch: metadata points to artifact {owner_id}, full row points to {}",
                chunk.artifact_id
            ),
        });
    }
    if !scan_secrets(&chunk.text).is_clean() {
        return Ok(None);
    }
    Ok(Some((artifact, chunk)))
}
