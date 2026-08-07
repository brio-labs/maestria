//! Canonical source artifact registration for repository code indexes.

use anyhow::{Context, Result};
use maestria_code_intel::RepositoryCodeIndex;
use maestria_core::{InstanceLayout, artifact_id_for, content_hash};
use maestria_domain::{ArtifactDetected, ArtifactId, ContentHash, DomainInput, IndexStatus};
use maestria_governance::scan_secrets;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Maximum number of repository source artifacts kept in flight (submitted
/// but not yet awaited) during registration.
///
/// The kernel input loop processes domain inputs serially and index effects
/// run under a bounded semaphore (`max_concurrent_effects`, 16 slots); a
/// burst that submits every source at once previously stalled the pipeline.
/// N=4 keeps the outstanding window well inside that headroom — four
/// artifacts each holding a small effect chain never exhaust the semaphore
/// or starve unrelated background effects — while still overlapping each
/// artifact's parse, full-text, evidence, and tantivy-commit work with the
/// next submissions. Waits stay serialized (oldest first, one poller at a
/// time). Do not raise N without empirical measurements of the runtime
/// pipeline.
const REGISTRATION_IN_FLIGHT: usize = 4;

/// Register every file with indexed symbols as a canonical source artifact
/// through the kernel pipeline (the same `ArtifactDetected` flow the generic
/// indexer uses), then wait until all are durably indexed. Code queries
/// authorize symbols against these artifacts and their evidence, so a code
/// index without registered sources cannot be searched.
///
/// Artifacts are registered in a bounded submit-ahead window
/// (`REGISTRATION_IN_FLIGHT` at most outstanding): each submission is
/// awaited immediately, but the wait for the terminal `Indexed` state
/// happens only when the window is full, always for the oldest submission
/// first. A failed registration aborts with the first error and leaves a
/// consistent index; the next run reconciles it.
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
    let mut in_flight: VecDeque<(ArtifactId, PathBuf)> = VecDeque::new();
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
        // An empty source file parses to zero chunks, so the kernel pipeline
        // never reaches the Indexed state and the wait would time out. Such a
        // file has no indexable content; its records (typically only the
        // module symbol) stay unbound and the query authorization skips them,
        // exactly like secret-skipped files.
        if bytes.is_empty() {
            skipped += 1;
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
        in_flight.push_back((artifact_id, path));
        if in_flight.len() == REGISTRATION_IN_FLIGHT {
            wait_oldest_registered(layout, &mut in_flight).await?;
        }
    }
    while !in_flight.is_empty() {
        wait_oldest_registered(layout, &mut in_flight).await?;
    }
    if skipped > 0 {
        eprintln!(
            "skipped {} repository source(s) with no indexable content or secret-like content (not searchable)",
            skipped
        );
    }
    Ok(mismatched)
}

/// Per-file wait budget for one repository source artifact to reach
/// `Indexed`. Not a hard deadline: the wait extends while the kernel event
/// log keeps growing (pipeline alive); a zero-progress budget fails.
const REGISTRATION_WAIT_BUDGET: Duration = Duration::from_secs(60);

/// Wait until the oldest in-flight submission reaches `Indexed`, then drop
/// it from the window. Waits are serialized (oldest first); a budget expiry
/// extends while the kernel event log keeps advancing, so a slow-but-alive
/// pipeline never fails registration (see [`REGISTRATION_WAIT_BUDGET`]).
async fn wait_oldest_registered(
    layout: &InstanceLayout,
    in_flight: &mut VecDeque<(ArtifactId, PathBuf)>,
) -> Result<()> {
    let (artifact_id, path) = in_flight
        .pop_front()
        .ok_or_else(|| anyhow::anyhow!("repository registration window drained unexpectedly"))?;
    let wait_context = format!("waiting for repository source indexing: {}", path.display());
    let mut last_event_count = crate::helpers::load_kernel_state_with_retry(
        layout,
        "seed repository registration progress",
    )?
    .event_log
    .len();
    loop {
        let wait = crate::helpers::wait_for_kernel_state(
            layout,
            REGISTRATION_WAIT_BUDGET,
            wait_context.clone(),
            |state| {
                state
                    .artifacts
                    .get(&artifact_id)
                    .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
            },
        )
        .await;
        match wait {
            Ok(_) => return Ok(()),
            Err(error) => {
                let state = crate::helpers::load_kernel_state_with_retry(
                    layout,
                    "assess repository registration progress",
                )?;
                let event_count = state.event_log.len();
                let progressed = event_count > last_event_count;
                last_event_count = event_count;
                if !progressed {
                    return Err(error.context(
                        "repository source indexing stalled: kernel event log made no progress",
                    ));
                }
            }
        }
    }
}
