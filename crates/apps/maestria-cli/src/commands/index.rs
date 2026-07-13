use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest, artifact_id_for, content_hash};
use maestria_daemon::RecoveryInputs;
use maestria_domain::{
    ArtifactDetected, ArtifactId, DomainInput, IndexStatus, KernelState, TaskId,
};
use maestria_governance::{AutonomyProfile, PrivacyExclusions, Scope};
use std::{fs, path::PathBuf, time::Duration};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

use crate::helpers;

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Shared context for processing a single file through indexing.
struct ProcessContext<'a> {
    scope: &'a Scope,
    privacy: &'a PrivacyExclusions,
    manifest: &'a InstanceManifest,
    preexisting_state: &'a KernelState,
    input_tx: &'a mpsc::Sender<DomainInput>,
    layout: &'a InstanceLayout,
    index_timeout: Duration,
}

/// Collect recovery artifact IDs and validation task IDs, then queue all
/// parser, full-text, and validation recovery inputs onto the runtime channel.
///
/// Resume parsers are enqueued first so parsing completes before full-text
/// indexing begins, preserving bounded-channel ordering. Validation requests
/// follow indexing inputs because they do not depend on artifact draining.
async fn queue_recovery(
    recovery: RecoveryInputs,
    input_tx: &mpsc::Sender<DomainInput>,
) -> Result<(Vec<ArtifactId>, Vec<TaskId>)> {
    // Collect recovery IDs before consuming the recovery struct so we can
    // drain pending work before shutting down the one-shot runtime.
    let recovery_artifact_ids: Vec<ArtifactId> = {
        let resume_ids = recovery.resume_parsers.iter().filter_map(|input| {
            if let DomainInput::ResumeParser(r) = input {
                Some(r.artifact_id)
            } else {
                None
            }
        });
        let ft_ids = recovery.start_full_text.iter().filter_map(|input| {
            if let DomainInput::StartFullTextIndex(s) = input {
                Some(s.artifact_id)
            } else {
                None
            }
        });
        resume_ids.chain(ft_ids).collect()
    };
    let validation_task_ids: Vec<TaskId> = recovery
        .run_validations
        .iter()
        .filter_map(|input| match input {
            DomainInput::RequestTaskValidation(request) => Some(request.task_id),
            _ => None,
        })
        .collect();

    // Queue pending ResumeParser inputs first so parsing completes before
    // full-text indexing begins.
    for input in recovery.resume_parsers {
        input_tx
            .send(input)
            .await
            .context("failed to queue resume parser")?;
    }

    // Queue pending StartFullTextIndex inputs after resume parsers.
    for input in recovery.start_full_text {
        input_tx
            .send(input)
            .await
            .context("failed to queue restart full-text index")?;
    }

    // Queue validation requests after indexing recovery inputs.
    for input in recovery.run_validations {
        input_tx
            .send(input)
            .await
            .context("failed to queue task validation")?;
    }

    Ok((recovery_artifact_ids, validation_task_ids))
}

/// Process a single file through read, scope check, duplicate detection,
/// runtime submission, and wait-for-indexing.
///
/// Returns an error for scope violations, I/O failures, channel send errors,
/// indexing errors, or timeouts.  The caller is responsible for shutting down
/// the runtime on error.
async fn process_file(file: &PathBuf, ctx: &ProcessContext<'_>) -> Result<()> {
    // Preserve scope, privacy, and manifest checks before reading.
    if ctx.scope.check_read_containment(file).is_err()
        || ctx.privacy.is_excluded(file)
        || !ctx.manifest.allows_source(file)
    {
        return Err(anyhow!(
            "index path is outside the instance read scope or excluded by policy: {}",
            file.display()
        ));
    }

    let bytes = fs::read(file)?;
    let artifact_id = artifact_id_for(file, &bytes);
    let hash = content_hash(&bytes);
    // Check whether this exact artifact was already indexed before this session.
    if let Some(artifact) = ctx.preexisting_state.artifacts.get(&artifact_id)
        && artifact.content_hash.as_deref() == Some(&hash)
        && artifact.index_status == IndexStatus::Indexed
    {
        println!("unchanged artifact={} path={}", artifact.id, file.display());
        return Ok(());
    }

    let title = match file.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.to_string(),
        None => "unknown".to_string(),
    };

    ctx.input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title,
            source_path: file.display().to_string(),
            source_bytes: bytes,
            content_hash: hash,
        }))
        .await
        .context("failed to submit artifact to runtime")?;

    // Wait for the artifact to reach terminal persisted state.
    let result = timeout(ctx.index_timeout, async {
        loop {
            match maestria_daemon::load_kernel_state(ctx.layout)
                .with_context(|| "reload kernel state for indexing wait")
            {
                Ok(state) => {
                    if let Some(artifact) = state.artifacts.get(&artifact_id)
                        && artifact.index_status == IndexStatus::Indexed
                    {
                        println!("indexed artifact={} path={}", artifact.id, file.display());
                        return Ok::<_, anyhow::Error>(());
                    }
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => Err(anyhow!(
            "timeout waiting for artifact indexing: {}",
            file.display()
        )),
    }
}

/// Poll kernel state until every artifact in `recovery_artifact_ids` has
/// reached `IndexStatus::Indexed`, or until `recovery_timeout` elapses.
async fn drain_recovery(
    layout: &InstanceLayout,
    recovery_artifact_ids: &[ArtifactId],
    recovery_timeout: Duration,
) -> Result<()> {
    let result = timeout(recovery_timeout, async {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| "reload kernel state for recovery drain")
            {
                Ok(state) => {
                    if recovery_artifact_ids.iter().all(|id| {
                        state
                            .artifacts
                            .get(id)
                            .is_some_and(|a| a.index_status == IndexStatus::Indexed)
                    }) {
                        return Ok::<_, anyhow::Error>(());
                    }
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => Err(anyhow!("timeout waiting for recovery artifact indexing")),
    }
}

/// Poll kernel state until every recovered validation task has a durable
/// validation report, or until `recovery_timeout` elapses.
async fn drain_validation_recovery(
    layout: &InstanceLayout,
    validation_task_ids: &[TaskId],
    recovery_timeout: Duration,
) -> Result<()> {
    let result = timeout(recovery_timeout, async {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| "reload kernel state for validation recovery drain")
            {
                Ok(state) => {
                    if validation_task_ids.iter().all(|task_id| {
                        maestria_daemon::has_current_validation_report(&state, *task_id)
                    }) {
                        return Ok::<_, anyhow::Error>(());
                    }
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) if helpers::is_db_locked(&error) => {
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => Err(anyhow!(
            "timeout waiting for recovered task validation reports"
        )),
    }
}

// ---------------------------------------------------------------------------
// Public command
// ---------------------------------------------------------------------------

pub async fn run(instance_dir: PathBuf, path: PathBuf, recursive: bool) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let _instance_lock = maestria_daemon::acquire_instance_write_lock(&layout).await?;
    let manifest = helpers::load_manifest(&layout)?;
    let scope = Scope::new(
        manifest.read_roots.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
    );
    let privacy = PrivacyExclusions::default();
    let files = helpers::collect_index_files(&path, recursive)?;
    if files.is_empty() {
        return Err(anyhow!(
            "no files selected for indexing at {}",
            path.display()
        ));
    }

    // Load persistent kernel state to check which artifacts are already indexed.
    let initial_state = maestria_daemon::load_kernel_state(&layout)
        .with_context(|| "load kernel state for indexing")?;

    // Repair projection repositories before runtime start so that
    // artifact, chunk, card, and evidence lookups succeed even if the
    // previous process crashed after event append but before a
    // projection write.
    {
        let store = maestria_storage_sqlite::SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        maestria_daemon::reconcile_projections(&initial_state, &store)
            .with_context(|| "reconcile projection repositories")?;
    }
    let preexisting_state = initial_state.clone();
    // Compute recovery inputs before state is moved into build_runtime.
    let recovery = maestria_daemon::recovery_inputs(&initial_state);

    // Validate recovery scope from the instance manifest before touching
    // blobs or building the runtime.  Out-of-scope and excluded pending
    // parsers fail fast with a descriptive error.
    maestria_daemon::validate_recovery_scope(&layout, &recovery)
        .with_context(|| "validate recovery scope against instance manifest")?;

    maestria_daemon::verify_pending_blobs(&layout, &recovery.resume_parsers)
        .with_context(|| "verify pending parser blobs for resume")?;

    // Build a one-shot runtime with a non-critical profile that allows
    // PersistEvent / ParseArtifact effects.
    let (runtime, input_tx, input_rx, shutdown_token) =
        maestria_daemon::build_runtime(&layout, initial_state, AutonomyProfile::TrustedWorkspace)?;

    let runtime_task = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    let result = async {
        let (recovery_artifact_ids, validation_task_ids) =
            queue_recovery(recovery, &input_tx).await?;
        let index_timeout = Duration::from_secs(30);
        let ctx = ProcessContext {
            scope: &scope,
            privacy: &privacy,
            manifest: &manifest,
            preexisting_state: &preexisting_state,
            input_tx: &input_tx,
            layout: &layout,
            index_timeout,
        };

        for file in &files {
            process_file(file, &ctx).await?;
        }

        // Recovery inputs run concurrently with fresh file processing. Drain
        // them before shutting down so no pending work is silently aborted.
        if !recovery_artifact_ids.is_empty() {
            drain_recovery(&layout, &recovery_artifact_ids, Duration::from_secs(60)).await?;
        }
        if !validation_task_ids.is_empty() {
            drain_validation_recovery(&layout, &validation_task_ids, Duration::from_secs(60))
                .await?;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    shutdown_token.cancel();
    let _ = runtime_task.await;
    result
}
