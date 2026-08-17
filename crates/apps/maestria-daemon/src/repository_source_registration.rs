//! Canonical source artifact registration for repository code indexes.
//!
//! Shared by the CLI (`index repository` via a mutation session) and the
//! daemon `RepositoryIndexRun` service (via the live runtime handle):
//! every file with indexed symbols is registered as a canonical artifact
//! through the kernel pipeline (the same `ArtifactDetected` flow the
//! generic indexer uses), then waited on until all are durably indexed.
//! Code queries authorize symbols against these artifacts and their
//! evidence, so a code index without registered sources cannot be searched.

use anyhow::{Context, Result, anyhow};
use maestria_code_intel::RepositoryCodeIndex;
use maestria_core::{InstanceLayout, artifact_id_for, content_hash};
use maestria_domain::{ArtifactDetected, ArtifactId, ContentHash, DomainInput, IndexStatus};
use maestria_governance::scan_secrets;
use maestria_runtime::{DomainApplicationResult, RuntimeHandle, RuntimeSubmissionError};
use maestria_storage_sqlite::SqliteStore;
use std::collections::VecDeque;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Live progress of the active repository index run, shared between the run
/// handler (writer) and the status handler (reader) in this process. Cleared
/// when the run finishes or the daemon restarts.
static REPOSITORY_INDEX_PROGRESS: Mutex<Option<crate::api::RepositoryIndexProgress>> =
    Mutex::new(None);

/// Publish the live run progress; `None` clears it.
pub(crate) fn set_repository_index_progress(progress: Option<crate::api::RepositoryIndexProgress>) {
    if let Ok(mut slot) = REPOSITORY_INDEX_PROGRESS.lock() {
        *slot = progress;
    }
}

/// The current live run progress, when a run is active.
pub(crate) fn repository_index_progress() -> Option<crate::api::RepositoryIndexProgress> {
    REPOSITORY_INDEX_PROGRESS
        .lock()
        .ok()
        .and_then(|guard| (*guard).clone())
}

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
/// through a mutation session (the CLI path), then wait until all are
/// durably indexed.
///
/// Artifacts are registered in a bounded submit-ahead window
/// (`REGISTRATION_IN_FLIGHT` at most outstanding): each submission is
/// awaited immediately, but the wait for the terminal `Indexed` state
/// happens only when the window is full, always for the oldest submission
/// first. A failed registration aborts with the first error and leaves a
/// consistent index; the next run reconciles it.
///
/// Files already durably indexed with the same content hash are skipped, so
/// re-runs and interrupted runs resume instead of re-registering from
/// scratch.
///
/// Returns the relative paths whose on-disk content no longer matches the
/// indexed content hash (the caller should rebuild and re-register those),
/// plus the number of sources skipped during registration (already
/// indexed, empty, or secret-like content).
///
/// # Cancellation
/// Cancelling stops submission between files; files already submitted are
/// processed by the runtime as usual and the next run reconciles any gap.
pub async fn register_repository_sources_with_session(
    layout: &InstanceLayout,
    session: &crate::MutationSession,
    index: &RepositoryCodeIndex,
    repository: &Path,
) -> Result<(std::collections::BTreeSet<String>, usize)> {
    let layout = layout.clone();
    let already_indexed = |artifact_id: &ArtifactId, hash: &ContentHash| {
        let layout = layout.clone();
        let artifact_id = *artifact_id;
        let hash = hash.clone();
        async move {
            let Ok(store) = SqliteStore::open_read_only(&layout.database_path) else {
                return false;
            };
            matches!(
                maestria_ports::ArtifactRepository::get(&store, artifact_id),
                Ok(Some(artifact))
                    if artifact.content_hash.as_ref() == Some(&hash)
                        && artifact.index_status == IndexStatus::Indexed
            )
        }
    };
    let started = maestria_retrieval::MonotonicInstant::now();
    register_repository_sources_inner(
        index,
        repository,
        |input| session.submit(input),
        |artifact_id, path| wait_oldest_registered_via_db(&layout, artifact_id, path),
        already_indexed,
        |done, total| eprintln!("{}", registration_progress(started, done, total)),
    )
    .await
}

/// Register every file with indexed symbols as a canonical source artifact
/// through the live runtime handle (the daemon service path), then wait
/// until all are durably indexed. Same window, budget, and progress rules
/// as the session variant; state is polled from the in-memory kernel.
/// Files already durably indexed with the same content hash are skipped.
/// Returns the mismatched relative paths plus the skipped-source count.
///
/// # Cancellation
/// Cancelling stops submission between files; files already submitted are
/// processed by the runtime as usual and the next run reconciles any gap.
pub async fn register_repository_sources_with_runtime(
    runtime: &RuntimeHandle,
    index: &RepositoryCodeIndex,
    repository: &Path,
) -> Result<(std::collections::BTreeSet<String>, usize)> {
    let already_indexed = |artifact_id: &ArtifactId, hash: &ContentHash| {
        let runtime = runtime.clone();
        let artifact_id = *artifact_id;
        let hash = hash.clone();
        async move {
            let state = runtime.kernel_state().await;
            state.artifacts.get(&artifact_id).is_some_and(|artifact| {
                artifact.content_hash.as_ref() == Some(&hash)
                    && artifact.index_status == IndexStatus::Indexed
            })
        }
    };
    let started = maestria_retrieval::MonotonicInstant::now();
    register_repository_sources_inner(
        index,
        repository,
        |input| runtime.submit(input),
        |artifact_id, path| wait_oldest_registered_via_runtime(runtime, artifact_id, path),
        already_indexed,
        |done, total| {
            eprintln!("{}", registration_progress(started, done, total));
            set_repository_index_progress(Some(crate::api::RepositoryIndexProgress {
                phase: "registering".to_string(),
                total,
                registered: done,
            }));
        },
    )
    .await
}

/// One registration progress line: done/total with a live rate and elapsed
/// time.
fn registration_progress(
    started: maestria_retrieval::MonotonicInstant,
    done: usize,
    total: usize,
) -> String {
    let elapsed = started.elapsed();
    let rate = done as f64 / elapsed.as_secs_f64().max(0.001);
    let percent = if total == 0 {
        0.0
    } else {
        done as f64 / total as f64 * 100.0
    };
    format!(
        "repository sources: {done}/{total} ({percent:.1}%) {rate:.1}/s elapsed={}",
        format_elapsed(elapsed)
    )
}

fn format_elapsed(elapsed: std::time::Duration) -> String {
    let total_seconds = elapsed.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Shared registration body: build the expected content-hash map from the
/// indexed symbols, submit matching files through `submit`, and await each
/// oldest in-flight artifact through `wait_indexed` before submitting the
/// next window. Files `already_indexed` with the same content hash are
/// skipped. Progress is reported every ten files so long registrations are
/// visible. Returns the mismatched paths and the skipped count.
async fn register_repository_sources_inner<F, S, W, Wf, C, Cf, P>(
    index: &RepositoryCodeIndex,
    repository: &Path,
    mut submit: F,
    wait_indexed: W,
    already_indexed: C,
    progress: P,
) -> Result<(std::collections::BTreeSet<String>, usize)>
where
    F: FnMut(DomainInput) -> S,
    S: Future<Output = Result<DomainApplicationResult, RuntimeSubmissionError>>,
    W: Fn(ArtifactId, PathBuf) -> Wf,
    Wf: Future<Output = Result<()>>,
    C: Fn(&ArtifactId, &ContentHash) -> Cf,
    Cf: Future<Output = bool>,
    P: Fn(usize, usize),
{
    let mut expected: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for symbol in &index.symbols {
        expected
            .entry(symbol.provenance.file_path.clone())
            .or_insert_with(|| symbol.provenance.content_hash.clone());
    }
    if expected.is_empty() {
        return Ok((std::collections::BTreeSet::new(), 0));
    }
    let total = expected.len();
    let mut mismatched = std::collections::BTreeSet::new();
    let mut skipped = 0_usize;
    let mut done = 0_usize;
    let mut in_flight: VecDeque<(ArtifactId, PathBuf)> = VecDeque::new();
    for (relative_path, indexed_hash) in &expected {
        done += 1;
        if done.is_multiple_of(10) || done == total {
            progress(done, total);
        }
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
        let hash = ContentHash::new(content_hash)?;
        // Already-indexed files (a previous run, the watcher, or another
        // instance path) are skipped: re-submitting them re-runs the whole
        // durable pipeline for no new evidence.
        if already_indexed(&artifact_id, &hash).await {
            skipped += 1;
            continue;
        }
        let title = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name.to_string(),
            None => "artifact".to_string(),
        };
        submit(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title,
            source_path: path.display().to_string(),
            source_bytes: bytes,
            content_hash: hash,
        }))
        .await
        .with_context(|| format!("submit repository source artifact for {}", path.display()))?;
        in_flight.push_back((artifact_id, path));
        if in_flight.len() == REGISTRATION_IN_FLIGHT {
            let (artifact_id, path) = in_flight
                .pop_front()
                .ok_or_else(|| anyhow!("repository registration window drained unexpectedly"))?;
            wait_indexed(artifact_id, path).await?;
        }
    }
    while !in_flight.is_empty() {
        let (artifact_id, path) = in_flight
            .pop_front()
            .ok_or_else(|| anyhow!("repository registration window drained unexpectedly"))?;
        wait_indexed(artifact_id, path).await?;
    }
    if skipped > 0 {
        eprintln!(
            "skipped {} repository source(s) with no indexable content or \
             secret-like content (not searchable)",
            skipped
        );
    }
    Ok((mismatched, skipped))
}

/// Per-file wait budget for one repository source artifact to reach
/// `Indexed`. Not a hard deadline: the wait extends while the kernel event
/// log keeps growing (pipeline alive); a zero-progress budget fails.
const REGISTRATION_WAIT_BUDGET: Duration = Duration::from_secs(60);

/// Wait until `artifact_id` reaches `Indexed` (session path).
///
/// The steady-state poll reads only the artifact row — a full kernel-state
/// replay per poll would re-apply every persisted event (which dominates
/// batch ingestion time as the event log grows, per the CLI polling
/// guidance). A budget expiry extends while the kernel event log keeps
/// advancing; the replay is only done on expiry, so a slow-but-alive
/// pipeline never fails registration (see [`REGISTRATION_WAIT_BUDGET`]).
async fn wait_oldest_registered_via_db(
    layout: &InstanceLayout,
    artifact_id: ArtifactId,
    path: PathBuf,
) -> Result<()> {
    let wait_context = format!("waiting for repository source indexing: {}", path.display());
    let mut last_event_count = crate::load_kernel_state(layout)
        .context("seed repository registration progress")?
        .event_log
        .len();
    loop {
        let wait = tokio::time::timeout(REGISTRATION_WAIT_BUDGET, async {
            loop {
                if artifact_is_indexed(layout, &artifact_id) {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        match wait {
            Ok(result) => return result,
            Err(_elapsed) => {
                let state = crate::load_kernel_state(layout)
                    .context("assess repository registration progress")?;
                let event_count = state.event_log.len();
                let progressed = event_count > last_event_count;
                last_event_count = event_count;
                if !progressed {
                    return Err(anyhow!(
                        "repository source indexing stalled: kernel event log made no progress \
                         (while {wait_context})"
                    ));
                }
            }
        }
    }
}

/// Whether the persisted artifact row is durably indexed. Read failures are
/// transient (the caller's budget and event-log extension bound the wait).
fn artifact_is_indexed(layout: &InstanceLayout, artifact_id: &ArtifactId) -> bool {
    let Ok(store) = SqliteStore::open_read_only(&layout.database_path) else {
        return false;
    };
    matches!(
        maestria_ports::ArtifactRepository::get(&store, *artifact_id),
        Ok(Some(artifact)) if artifact.index_status == IndexStatus::Indexed
    )
}

/// Wait until `artifact_id` reaches `Indexed`, polling the in-memory kernel
/// state (runtime path). Same budget and extend-while-advancing rule as the
/// session variant.
async fn wait_oldest_registered_via_runtime(
    runtime: &RuntimeHandle,
    artifact_id: ArtifactId,
    path: PathBuf,
) -> Result<()> {
    let wait_context = format!("waiting for repository source indexing: {}", path.display());
    let mut last_event_count = runtime.kernel_state().await.event_log.len();
    loop {
        let wait = tokio::time::timeout(REGISTRATION_WAIT_BUDGET, async {
            loop {
                let state = runtime.kernel_state().await;
                if state
                    .artifacts
                    .get(&artifact_id)
                    .is_some_and(|artifact| artifact.index_status == IndexStatus::Indexed)
                {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await;
        match wait {
            Ok(result) => return result,
            Err(_elapsed) => {
                let event_count = runtime.kernel_state().await.event_log.len();
                let progressed = event_count > last_event_count;
                last_event_count = event_count;
                if !progressed {
                    return Err(anyhow!(
                        "repository source indexing stalled: kernel event log made no progress \
                         (while {wait_context})"
                    ));
                }
            }
        }
    }
}
