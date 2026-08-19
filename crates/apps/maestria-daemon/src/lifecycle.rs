use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{ArtifactId, DomainInput, KernelState, TaskId};
use maestria_governance::AutonomyProfile;
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_storage_sqlite::SqliteStore;
use std::collections::BTreeMap;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::approval_recovery::{reconcile_approval_repo, reconcile_pending_approvals};
use crate::instance_setup::{load_kernel_state, validate_recovery_scope};
use crate::lock::{
    acquire as acquire_instance_write_lock, try_acquire as try_acquire_instance_write_lock,
};
use crate::parser_resume::verify_pending_blobs;
use crate::projection_recovery::{
    reconcile_full_text_projection, reconcile_graph_projection, reconcile_projections,
};
use crate::recovery_inputs::RecoveryInputs;
use crate::recovery_staging::{
    RecoveryQueueStage, queue_recovery_inputs, recovery_artifact_ids, source_artifact_ids,
    validation_task_ids,
};
use crate::runtime_construction::build_runtime;
use crate::supervision_recovery::supervise_recovery;
use crate::vector_startup::{
    reconcile_retrieval_generations, reconcile_vector_projection_for_layout,
};

/// Preserve a startup/recovery failure together with a concurrent shutdown failure.
///
/// Shutdown failures must not be discarded when the primary operation already
/// failed: teardown errors (runtime-task joins, watcher joins) are as relevant
/// to the caller as the original failure (R24).
pub(crate) fn combine_failures(error: anyhow::Error, shutdown: Result<()>) -> anyhow::Error {
    match shutdown {
        Ok(()) => error,
        Err(shutdown_error) => error.context(format!(
            "lifecycle shutdown also failed: {shutdown_error:#}"
        )),
    }
}

/// Recovery work queued by the shared lifecycle and available to command-specific drain logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryQueue {
    pub artifact_ids: Vec<ArtifactId>,
    pub validation_task_ids: Vec<TaskId>,
}

/// Owns instance locking, startup recovery, runtime execution, and shutdown.
pub struct InstanceLifecycle {
    state: KernelState,
    layout: InstanceLayout,
    manifest: InstanceManifest,
    recovery: Option<RecoveryInputs>,
    recovery_queue: Option<RecoveryQueue>,
    paused_effects: usize,
    input_tx: mpsc::Sender<DomainInput>,
    runtime_handle: maestria_runtime::RuntimeHandle,
    shutdown_token: CancellationToken,
    runtime_task: Option<JoinHandle<Result<(), maestria_runtime::RuntimeRunError>>>,
    watcher_task: Option<JoinHandle<Result<()>>>,
    watched_artifacts: BTreeMap<String, (ArtifactId, String)>,
}

impl Drop for InstanceLifecycle {
    fn drop(&mut self) {
        // The runtime task owns the write lock and releases it only after its
        // effect executor has observed shutdown and drained.
        self.shutdown_token.cancel();
    }
}
impl InstanceLifecycle {
    /// Acquire the instance lock, repair projections, validate recovery, and start the runtime.
    ///
    /// # Cancellation
    /// If the future is dropped before completion, the instance write lock is released and the
    /// runtime is not started. Any partially constructed state is dropped.
    pub async fn start(layout: InstanceLayout, profile: AutonomyProfile) -> Result<Self> {
        Self::start_with_vector_reconcile(layout, profile, true).await
    }

    /// Start the lifecycle, returning `Ok(None)` when another process holds
    /// the instance write lock, so callers can degrade instead of blocking.
    pub(crate) async fn try_start_with_vector_reconcile(
        layout: InstanceLayout,
        profile: AutonomyProfile,
        rebuild_vector_projection: bool,
    ) -> Result<Option<Self>> {
        let Some(lock) = try_acquire_instance_write_lock(&layout)? else {
            return Ok(None);
        };
        Self::start_with_held_lock(layout, profile, rebuild_vector_projection, lock)
            .await
            .map(Some)
    }

    /// Start the lifecycle, optionally skipping the vector-projection rebuild.
    ///
    /// Search surfaces skip the rebuild because the search runtime serves from
    /// whatever vector rows already exist and degrades explicitly when the
    /// embedding provider is unavailable; rebuilding would re-embed the whole
    /// corpus on every search command. Store-open and consistency failures for
    /// the other projections remain fatal either way.
    pub(crate) async fn start_with_vector_reconcile(
        layout: InstanceLayout,
        profile: AutonomyProfile,
        rebuild_vector_projection: bool,
    ) -> Result<Self> {
        let lock = acquire_instance_write_lock(&layout).await?;
        Self::start_with_held_lock(layout, profile, rebuild_vector_projection, lock).await
    }

    async fn start_with_held_lock(
        layout: InstanceLayout,
        profile: AutonomyProfile,
        rebuild_vector_projection: bool,
        lock: crate::lock::InstanceWriteLock,
    ) -> Result<Self> {
        let mut state =
            load_kernel_state(&layout).with_context(|| "load persisted kernel state")?;
        let store = SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        let watched_artifacts = source_artifact_ids(&store)?;
        let manifest_contents = std::fs::read_to_string(&layout.manifest_path)
            .with_context(|| "read instance manifest")?;
        let manifest = InstanceManifest::decode(&manifest_contents)
            .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
        reconcile_retrieval_generations(&layout, &mut state, &manifest)
            .with_context(|| "reconcile retrieval generations")?;

        reconcile_projections(&state, &store)
            .with_context(|| "reconcile projection repositories")?;
        let search_index =
            crate::projection_open::open_full_text_index(&layout, &state, true, true)
                .with_context(|| "open full-text projection")?;
        reconcile_full_text_projection(&state, &*search_index)
            .with_context(|| "reconcile full-text projection")?;
        drop(search_index);
        let graph_index = SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?;
        reconcile_graph_projection(&state, &graph_index)
            .with_context(|| "reconcile graph projection")?;
        reconcile_approval_repo(&state, &store).with_context(|| "reconcile approval repository")?;
        reconcile_pending_approvals(&state, &store, &store)
            .with_context(|| "reconcile pending approvals")?;
        if rebuild_vector_projection {
            reconcile_vector_projection_for_layout(&layout, &state)
                .with_context(|| "reconcile vector projection")?;
        }
        if manifest
            .sparse
            .as_ref()
            .is_some_and(|config| config.enabled)
        {
            crate::sparse_startup::reconcile_sparse_projection_for_layout(
                &layout, &mut state, &manifest,
            )
            .with_context(|| "reconcile learned-sparse projection")?;
        }

        let diagnostics = supervise_recovery(&state, &store)?;
        validate_recovery_scope(&layout, &diagnostics.inputs)
            .with_context(|| "validate recovery scope against instance manifest")?;
        verify_pending_blobs(&layout, &diagnostics.inputs.resume_parsers)
            .with_context(|| "verify pending parser blobs for resume")?;
        let recovery_queue = RecoveryQueue {
            artifact_ids: recovery_artifact_ids(&diagnostics.inputs),
            validation_task_ids: validation_task_ids(&diagnostics.inputs),
        };

        let (runtime, input_tx, input_rx, shutdown_token) =
            build_runtime(&layout, state.clone(), profile)?;
        let runtime_handle = runtime.handle();
        let runtime = runtime.with_graceful_shutdown();
        let runtime_shutdown = shutdown_token.clone();
        let runtime_task = tokio::spawn(async move {
            let _instance_lock = lock;
            runtime.run(input_rx, runtime_shutdown).await
        });
        info!(root = %layout.root.display(), "runtime started");
        Ok(Self {
            layout,
            state,
            recovery: Some(diagnostics.inputs),
            recovery_queue: Some(recovery_queue),
            paused_effects: diagnostics.paused_effects.len(),
            manifest,
            input_tx,
            runtime_handle,
            shutdown_token,
            runtime_task: Some(runtime_task),
            watcher_task: None,
            watched_artifacts,
        })
    }

    pub fn state(&self) -> &KernelState {
        &self.state
    }

    pub fn paused_effect_count(&self) -> usize {
        self.paused_effects
    }

    pub fn runtime_handle(&self) -> maestria_runtime::RuntimeHandle {
        self.runtime_handle.clone()
    }
    fn start_watcher(&mut self) {
        if self.watcher_task.is_none() {
            self.watcher_task = Some(crate::watcher::spawn(
                self.layout.clone(),
                self.manifest.clone(),
                self.input_tx.clone(),
                self.watched_artifacts.clone(),
                self.shutdown_token.clone(),
            ));
        }
    }

    /// Queue recovery in dependency order: parsers, full-text, then validation.
    ///
    /// A failed or cancelled queue attempt leaves every input not yet accepted by the channel in
    /// `self.recovery`, so callers can retry without losing or duplicating work.
    pub async fn queue_recovery(&mut self) -> Result<RecoveryQueue> {
        let recovery_queue = self
            .recovery_queue
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("recovery inputs already queued"))?;
        {
            let recovery = self
                .recovery
                .as_mut()
                .ok_or_else(|| anyhow!("recovery inputs already queued"))?;

            for (stage, inputs) in [
                (
                    RecoveryQueueStage::ResumeParser,
                    &mut recovery.resume_parsers,
                ),
                (RecoveryQueueStage::FullText, &mut recovery.start_full_text),
                (
                    RecoveryQueueStage::Validation,
                    &mut recovery.run_validations,
                ),
            ] {
                queue_recovery_inputs(&self.runtime_handle, inputs, stage).await?;
            }
        }

        self.recovery = None;
        self.recovery_queue = None;
        Ok(recovery_queue)
    }

    /// Signal shutdown and await the runtime and watcher tasks.
    ///
    /// # Cancellation
    /// Once called, the shutdown token is cancelled. If this future is dropped before the tasks
    /// have joined, shutdown remains in progress but completion is not awaited.
    pub async fn shutdown(mut self) -> Result<()> {
        self.shutdown_token.cancel();
        if let Some(watcher_task) = self.watcher_task.take() {
            watcher_task
                .await
                .with_context(|| "continuous ingestion watcher join failed")??;
        }
        let Some(runtime_task) = self.runtime_task.take() else {
            return Ok(());
        };
        runtime_task
            .await
            .with_context(|| "runtime loop join failed")??;
        Ok(())
    }

    /// Run until the external `shutdown` token is triggered, or until the runtime stops itself.
    ///
    /// # Cancellation
    /// If the future is dropped before either shutdown condition is observed, recovery may be
    /// partially queued and the watcher may have started, but shutdown is not performed.
    pub async fn run_until_shutdown(mut self, shutdown: CancellationToken) -> Result<()> {
        let result = self.queue_recovery().await;
        if let Err(error) = result {
            let shutdown_result = self.shutdown().await;
            return Err(combine_failures(error, shutdown_result));
        }

        self.start_watcher();

        let (termination, runtime_result) = match self.runtime_task.as_mut() {
            Some(runtime_task) => {
                tokio::select! {
                    biased;
                    runtime_result = runtime_task => {
                        (RuntimeTermination::TaskCompleted, Some(runtime_result))
                    }
                    () = self.shutdown_token.cancelled() => {
                        (RuntimeTermination::InternalShutdown, None)
                    }
                    () = shutdown.cancelled() => (RuntimeTermination::ExternalShutdown, None),
                }
            }
            None => (RuntimeTermination::InternalShutdown, None),
        };
        if matches!(termination, RuntimeTermination::TaskCompleted) {
            self.runtime_task.take();
        }

        let shutdown_result = self.shutdown().await;
        match termination {
            RuntimeTermination::ExternalShutdown => shutdown_result,
            RuntimeTermination::InternalShutdown => match shutdown_result {
                Err(error) => Err(error),
                Ok(()) => Err(anyhow!(
                    "runtime requested shutdown before external shutdown"
                )),
            },
            RuntimeTermination::TaskCompleted => match runtime_result {
                Some(Err(error)) => Err(anyhow!(error).context("runtime loop join failed")),
                Some(Ok(Err(error))) => Err(anyhow!(error).context("runtime loop failed")),
                Some(Ok(Ok(()))) | None => Err(anyhow!("runtime loop stopped unexpectedly")),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RuntimeTermination {
    ExternalShutdown,
    InternalShutdown,
    TaskCompleted,
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
