//! Per-file batch pipeline for the index command: submission, terminal-state
//! waiting, and outcome tallying.
//!
//! One responsibility per module: this module owns moving one file through
//! duplicate detection, runtime submission, and its terminal wait;
//! `super::index` owns selection, orchestration, and reporting.

use anyhow::{Context, Result, anyhow};
use maestria_core::{
    InstanceLayout, InstanceManifest, artifact_id_for_content_hash, build_artifact_detected_input,
    content_hash,
};
use maestria_domain::{ArtifactId, IndexStatus, KernelState, TaskId};
use maestria_governance::{PrivacyExclusions, Scope};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use super::index_metrics::{IndexMetrics, human_bytes};
use crate::helpers;

/// Shared context for processing a single file through indexing.
pub(super) struct ProcessContext<'a> {
    pub(super) scope: &'a Scope,
    pub(super) privacy: &'a PrivacyExclusions,
    pub(super) manifest: &'a InstanceManifest,
    pub(super) preexisting_state: &'a KernelState,
    pub(super) session: &'a maestria_daemon::MutationSession,
    pub(super) layout: &'a InstanceLayout,
    pub(super) index_timeout: Duration,
}

/// Process a single file through read, scope check, duplicate detection,
/// runtime submission, and wait-for-indexing.
///
/// Returns an error for scope violations, I/O failures, runtime submission errors,
/// Terminal per-file outcome for the batch summary.
pub(super) enum FileOutcome {
    Indexed,
    Unchanged,
    Skipped(String),
    Failed(String),
}

/// An artifact whose submission succeeded and whose terminal state is still
/// pending. The pipeline collects these across a submission window and waits
/// for them as a group, letting the runtime parse/index behind the CLI
/// instead of serializing every file behind its predecessor.
pub(super) struct InFlightArtifact {
    pub(super) artifact_id: ArtifactId,
    pub(super) path: PathBuf,
    pub(super) prefix: String,
    pub(super) size_human: String,
    pub(super) bytes: u64,
}

/// Result of the submission phase for one file: either the file reached a
/// terminal outcome during submission (pre-existing duplicate), or it is now
/// in flight and awaiting its terminal state.
pub(super) enum SubmitOutcome {
    Terminal(FileOutcome, u64),
    InFlight(InFlightArtifact),
}

/// Number of files submitted to the runtime before the batch waits for
/// terminal states. Eight matches the vector lane width: wide enough to keep
/// the runtime parsing behind the CLI's poll loops, narrow enough to bound
/// the read-only poll connections opened while awaiting a window.
pub(super) const SUBMIT_WINDOW: usize = 8;

/// Batch-wide outcome tallies shared by the submission and terminal-wait
/// phases, plus the count of files whose outcome is fully known (used for
/// progress reporting).
pub(super) struct BatchTallies {
    pub(super) completed: usize,
    pub(super) indexed: usize,
    pub(super) unchanged: usize,
    pub(super) skipped: usize,
    pub(super) failed: usize,
}

impl BatchTallies {
    pub(super) fn new() -> Self {
        Self {
            completed: 0,
            indexed: 0,
            unchanged: 0,
            skipped: 0,
            failed: 0,
        }
    }

    /// Apply one terminal file outcome to the tallies and byte metrics.
    pub(super) fn record(&mut self, outcome: FileOutcome, bytes: u64, metrics: &mut IndexMetrics) {
        match outcome {
            FileOutcome::Indexed => self.indexed += 1,
            FileOutcome::Unchanged => self.unchanged += 1,
            FileOutcome::Skipped(_) => self.skipped += 1,
            FileOutcome::Failed(_) => self.failed += 1,
        }
        metrics.add_bytes(bytes);
        self.completed += 1;
    }
}

/// Validate and submit one file to the runtime without waiting for its
/// terminal state. The caller continues the window on error; the summary
/// reports the failure. Terminal outcomes (unchanged duplicates) resolve
/// during submission itself.
pub(super) async fn submit_file(
    file: &Path,
    prefix: String,
    ctx: &ProcessContext<'_>,
) -> Result<SubmitOutcome> {
    let file = file
        .canonicalize()
        .with_context(|| format!("canonicalize index path {}", file.display()))?;
    // Preserve scope, privacy, and manifest checks before reading.
    if ctx.scope.check_read_containment(&file).is_err()
        || ctx.privacy.is_excluded(&file)
        || !ctx.manifest.allows_source(&file)
    {
        return Err(anyhow!(
            "index path is outside the instance read scope or excluded by policy: {}",
            file.display()
        ));
    }

    let bytes = fs::read(&file)?;
    let bytes_len = bytes.len() as u64;
    let size = human_bytes(bytes_len);
    let hash_string = content_hash(&bytes);
    let artifact_id = artifact_id_for_content_hash(&file, &hash_string);
    let hash = maestria_domain::ContentHash::new(hash_string.clone())?;
    // Check whether this exact artifact was already indexed before this session.
    if let Some(artifact) = ctx.preexisting_state.artifacts.get(&artifact_id)
        && artifact.content_hash.as_ref() == Some(&hash)
        && artifact.index_status == IndexStatus::Indexed
    {
        println!(
            "{prefix} unchanged artifact={} path={} ({size})",
            artifact.id,
            file.display()
        );
        return Ok(SubmitOutcome::Terminal(FileOutcome::Unchanged, bytes_len));
    }

    let input = build_artifact_detected_input(&file, bytes, hash_string)?;
    ctx.session
        .submit(input)
        .await
        .context("failed to submit artifact to runtime")?;
    Ok(SubmitOutcome::InFlight(InFlightArtifact {
        artifact_id,
        path: file.to_path_buf(),
        prefix,
        size_human: size,
        bytes: bytes_len,
    }))
}

/// Wait for one submitted artifact to reach terminal persisted state.
/// Unsupported, failed, and quarantined parses carry no indexable content,
/// so the artifact never becomes `Indexed`; a terminal non-`Parsed` parse
/// status is done and reported as skipped. Polls the artifact row directly —
/// replaying the full event log per poll would dominate batch time as the
/// log grows.
pub(super) async fn await_file_terminal(
    pending: InFlightArtifact,
    ctx: &ProcessContext<'_>,
) -> FileOutcome {
    let outcome = std::cell::RefCell::new(None);
    let wait_result = helpers::wait_for_artifact_state(
        ctx.layout,
        pending.artifact_id,
        ctx.index_timeout,
        format!("waiting for artifact indexing: {}", pending.path.display()),
        |artifact| {
            if artifact.index_status == IndexStatus::Indexed {
                outcome.replace(Some(FileOutcome::Indexed));
                return true;
            }
            let skipped = match artifact.parse_status {
                Some(maestria_domain::ParseStatus::Unsupported) => Some("unsupported"),
                Some(maestria_domain::ParseStatus::Failed) => Some("failed"),
                Some(maestria_domain::ParseStatus::Quarantined) => Some("quarantined"),
                _ => None,
            };
            if let Some(reason) = skipped {
                outcome.replace(Some(FileOutcome::Skipped(reason.to_string())));
                true
            } else {
                false
            }
        },
    )
    .await;
    let outcome = match outcome.into_inner() {
        Some(outcome) => outcome,
        None => match wait_result {
            Ok(()) => FileOutcome::Failed("no terminal outcome".to_string()),
            Err(error) => FileOutcome::Failed(format!("terminal wait failed: {error:#}")),
        },
    };
    match &outcome {
        FileOutcome::Indexed => println!(
            "{} indexed artifact={} path={} ({})",
            pending.prefix,
            pending.artifact_id,
            pending.path.display(),
            pending.size_human
        ),
        FileOutcome::Unchanged => {}
        FileOutcome::Skipped(reason) => println!(
            "{} skipped artifact={} path={} ({}, {reason})",
            pending.prefix,
            pending.artifact_id,
            pending.path.display(),
            pending.size_human
        ),
        FileOutcome::Failed(reason) => eprintln!(
            "{} failed artifact={} path={} ({reason})",
            pending.prefix,
            pending.artifact_id,
            pending.path.display()
        ),
    }
    outcome
}

/// Wait until every artifact in `recovery_artifact_ids` has reached
/// `IndexStatus::Indexed`, or until `recovery_timeout` elapses.
pub(super) async fn drain_recovery(
    layout: &InstanceLayout,
    recovery_artifact_ids: &[ArtifactId],
    recovery_timeout: Duration,
) -> Result<()> {
    helpers::wait_for_kernel_state(
        layout,
        recovery_timeout,
        "waiting for recovery artifact indexing".to_string(),
        |state| {
            recovery_artifact_ids.iter().all(|id| {
                state
                    .artifacts
                    .get(id)
                    .is_some_and(|a| a.index_status == IndexStatus::Indexed)
            })
        },
    )
    .await?;
    Ok(())
}

/// Wait until every recovered validation task has a durable validation
/// report, or until `recovery_timeout` elapses.
pub(super) async fn drain_validation_recovery(
    layout: &InstanceLayout,
    validation_task_ids: &[TaskId],
    recovery_timeout: Duration,
) -> Result<()> {
    helpers::wait_for_kernel_state(
        layout,
        recovery_timeout,
        "waiting for recovered task validation reports".to_string(),
        |state| {
            validation_task_ids
                .iter()
                .all(|task_id| maestria_daemon::has_current_validation_report(state, *task_id))
        },
    )
    .await?;
    Ok(())
}
