use anyhow::{Context, Result, bail};
use maestria_blob_fs::FsBlobStore;
use maestria_core::InstanceLayout;
use maestria_domain::{DomainInput, KernelState, content_hash};
use maestria_ports::BlobStore;
/// Collects pending parser metadata from the replayed kernel state and builds
/// `ResumeParser` inputs so the runtime can resume parsing after restart
/// without re-ingesting source bytes. Each entry in `pending_parsers`
/// represents a `ParserStarted` event whose `ParserCompleted` counterpart
/// never arrived before the previous shutdown.
pub fn pending_resume_parsers(state: &KernelState) -> Vec<DomainInput> {
    state
        .pending_parsers
        .values()
        .cloned()
        .map(DomainInput::ResumeParser)
        .collect()
}

/// Verify that every pending parser's blob bytes are intact in the blob store
/// by reading the full object through `BlobStore::get` and comparing its
/// content hash against the `ParserStarted::content_hash` recorded at ingestion.
///
/// Returns an error on the first missing, corrupt, or tampered blob so the
/// operator knows which artifact is stranded rather than silently dropping work.
pub fn verify_pending_blobs(layout: &InstanceLayout, pending: &[DomainInput]) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    let blob_store = FsBlobStore::open(&layout.blobs_dir).with_context(|| {
        format!(
            "open blob store for resume verification: {}",
            layout.blobs_dir.display()
        )
    })?;
    for input in pending {
        if let DomainInput::ResumeParser(record) = input {
            let bytes = blob_store.get(record.blob_id).with_context(|| {
                format!(
                    "blob {} missing or corrupt for pending parser of artifact {} ({})",
                    record.blob_id, record.artifact_id, record.title
                )
            })?;
            let actual_hash = content_hash(&bytes);
            if actual_hash != record.content_hash {
                bail!(
                    "blob {} content hash mismatch for pending parser of artifact {} ({}): expected {}, got {}",
                    record.blob_id,
                    record.artifact_id,
                    record.title,
                    record.content_hash,
                    actual_hash,
                );
            }
        }
    }
    Ok(())
}
