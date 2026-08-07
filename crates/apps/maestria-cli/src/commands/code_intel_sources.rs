//! Canonical source artifact registration for repository code indexes.

use anyhow::{Context, Result};
use maestria_code_intel::RepositoryCodeIndex;
use maestria_core::{InstanceLayout, artifact_id_for, content_hash};
use maestria_domain::{ArtifactDetected, ContentHash, DomainInput, IndexStatus};
use maestria_governance::scan_secrets;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// Register every file with indexed symbols as a canonical source artifact
/// through the kernel pipeline (the same `ArtifactDetected` flow the generic
/// indexer uses), then wait until all are durably indexed. Code queries
/// authorize symbols against these artifacts and their evidence, so a code
/// index without registered sources cannot be searched.
///
/// Returns the relative paths whose on-disk content no longer matches the
/// indexed content hash (the caller should rebuild and re-register those).
pub(super) async fn register_repository_sources(
    layout: &InstanceLayout,
    session: &maestria_daemon::MutationSession,
    index: &RepositoryCodeIndex,
    repository: &Path,
) -> Result<std::collections::BTreeSet<String>> {
    let mut expected: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for symbol in &index.symbols {
        expected
            .entry(symbol.provenance.file_path.clone())
            .or_insert_with(|| symbol.provenance.content_hash.clone());
    }
    if expected.is_empty() {
        return Ok(std::collections::BTreeSet::new());
    }
    let mut mismatched = std::collections::BTreeSet::new();
    let mut skipped = 0_usize;
    // Submit one artifact at a time and wait for it to reach the terminal
    // indexed state before the next. The kernel processes inputs serially
    // and index effects run under a bounded semaphore; flooding the input
    // channel with every source at once stalls the pipeline, so mirror the
    // generic indexer's submit-and-wait cadence.
    for (relative_path, indexed_hash) in &expected {
        let path = repository.join(relative_path);
        let bytes = fs::read(&path).with_context(|| {
            format!(
                "read repository source for artifact registration: {}",
                path.display()
            )
        })?;
        let content_hash = content_hash(&bytes);
        if content_hash != *indexed_hash {
            mismatched.insert(relative_path.clone());
            continue;
        }
        // The kernel refuses to index secret-bearing chunks (its full-text
        // effect fails and the runtime shuts down), so files the same scanner
        // flags are left unbound up front; their symbols are then skipped by
        // the query authorization instead of erroring.
        if !scan_secrets(&String::from_utf8_lossy(&bytes)).is_clean() {
            skipped += 1;
            continue;
        }
        let artifact_id = artifact_id_for(&path, &bytes);
        let title = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name.to_string(),
            None => "artifact".to_string(),
        };
        session
            .submit(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title,
                source_path: path.display().to_string(),
                source_bytes: bytes,
                content_hash: ContentHash::new(content_hash)?,
            }))
            .await
            .with_context(|| format!("submit repository source artifact for {}", path.display()))?;
        crate::helpers::wait_for_kernel_state(
            layout,
            Duration::from_secs(60),
            format!("waiting for repository source indexing: {}", path.display()),
            |state| {
                state
                    .artifacts
                    .get(&artifact_id)
                    .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
            },
        )
        .await?;
    }
    if skipped > 0 {
        eprintln!(
            "skipped {} repository source(s) containing secret-like content (not searchable)",
            skipped
        );
    }
    Ok(mismatched)
}
