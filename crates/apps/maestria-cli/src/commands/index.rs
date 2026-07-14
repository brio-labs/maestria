use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest, artifact_id_for, content_hash};
use maestria_domain::{
    ArtifactDetected, ArtifactId, DomainInput, IndexStatus, KernelState, TaskId,
};
use maestria_governance::{PrivacyExclusions, Scope};
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

    let mut lifecycle = maestria_daemon::InstanceLifecycle::start(
        layout.clone(),
        maestria_governance::AutonomyProfile::TrustedWorkspace,
    )
    .await?;
    if lifecycle.paused_effect_count() > 0 {
        println!(
            "paused {} in-flight harness effects",
            lifecycle.paused_effect_count()
        );
    }
    let preexisting_state = lifecycle.state().clone();
    let input_tx = lifecycle.input_sender();

    let result = async {
        let recovery = lifecycle.queue_recovery().await?;
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

        if !recovery.artifact_ids.is_empty() {
            drain_recovery(&layout, &recovery.artifact_ids, Duration::from_secs(60)).await?;
        }
        if !recovery.validation_task_ids.is_empty() {
            drain_validation_recovery(
                &layout,
                &recovery.validation_task_ids,
                Duration::from_secs(60),
            )
            .await?;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let shutdown_result = lifecycle.shutdown().await;
    result?;
    shutdown_result
}
