use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::{ArtifactId, DomainInput, KernelState, TaskId};
use maestria_governance::AutonomyProfile;
use maestria_graph_sqlite::SqliteGraphIndex;
use maestria_storage_sqlite::SqliteStore;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::{
    RecoveryInputs, acquire_instance_write_lock, build_runtime, load_kernel_state,
    reconcile_approval_repo, reconcile_graph_projection, reconcile_pending_approvals,
    reconcile_projections, reconcile_retrieval_generations, reconcile_vector_projection_for_layout,
    supervise_recovery, validate_recovery_scope, verify_pending_blobs,
};

/// Recovery work queued by the shared lifecycle and available to command-specific drain logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryQueue {
    pub artifact_ids: Vec<ArtifactId>,
    pub validation_task_ids: Vec<TaskId>,
}

/// Owns instance locking, startup recovery, runtime execution, and shutdown.
pub struct InstanceLifecycle {
    layout: InstanceLayout,
    state: KernelState,
    recovery: Option<RecoveryInputs>,
    paused_effects: usize,
    input_tx: mpsc::Sender<DomainInput>,
    shutdown_token: CancellationToken,
    runtime_task: Option<JoinHandle<()>>,
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
    pub async fn start(layout: InstanceLayout, profile: AutonomyProfile) -> Result<Self> {
        let lock = acquire_instance_write_lock(&layout).await?;
        let mut state =
            load_kernel_state(&layout).with_context(|| "load persisted kernel state")?;
        let store = SqliteStore::open(&layout.database_path)
            .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
        let manifest_contents = std::fs::read_to_string(&layout.manifest_path)
            .with_context(|| "read instance manifest")?;
        let manifest = InstanceManifest::decode(&manifest_contents)
            .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
        reconcile_retrieval_generations(&layout, &mut state, &manifest)
            .with_context(|| "reconcile retrieval generations")?;

        reconcile_projections(&state, &store)
            .with_context(|| "reconcile projection repositories")?;
        let graph_index = SqliteGraphIndex::open(layout.graph_index_dir.join("projection.db"))
            .with_context(|| format!("open graph index {}", layout.graph_index_dir.display()))?;
        reconcile_graph_projection(&state, &graph_index)
            .with_context(|| "reconcile graph projection")?;
        reconcile_approval_repo(&state, &store).with_context(|| "reconcile approval repository")?;
        reconcile_pending_approvals(&state, &store, &store)
            .with_context(|| "reconcile pending approvals")?;
        reconcile_vector_projection_for_layout(&layout, &state)
            .with_context(|| "reconcile vector projection")?;

        let diagnostics = supervise_recovery(&state, &store)?;
        validate_recovery_scope(&layout, &diagnostics.inputs)
            .with_context(|| "validate recovery scope against instance manifest")?;
        verify_pending_blobs(&layout, &diagnostics.inputs.resume_parsers)
            .with_context(|| "verify pending parser blobs for resume")?;

        let (runtime, input_tx, input_rx, shutdown_token) =
            build_runtime(&layout, state.clone(), profile)?;
        let runtime = runtime.with_graceful_shutdown();
        let runtime_shutdown = shutdown_token.clone();
        let runtime_task = tokio::spawn(async move {
            let _instance_lock = lock;
            runtime.run(input_rx, runtime_shutdown).await;
        });
        info!(root = %layout.root.display(), "runtime started");

        Ok(Self {
            layout,
            state,
            recovery: Some(diagnostics.inputs),
            paused_effects: diagnostics.paused_effects.len(),
            input_tx,
            shutdown_token,
            runtime_task: Some(runtime_task),
        })
    }

    pub fn layout(&self) -> &InstanceLayout {
        &self.layout
    }

    pub fn state(&self) -> &KernelState {
        &self.state
    }

    pub fn paused_effect_count(&self) -> usize {
        self.paused_effects
    }

    pub fn input_sender(&self) -> mpsc::Sender<DomainInput> {
        self.input_tx.clone()
    }

    /// Queue recovery in dependency order: parsers, full-text, then validation.
    pub async fn queue_recovery(&mut self) -> Result<RecoveryQueue> {
        let recovery = self
            .recovery
            .take()
            .ok_or_else(|| anyhow!("recovery inputs already queued"))?;
        let artifact_ids = recovery_artifact_ids(&recovery);
        let validation_task_ids = validation_task_ids(&recovery);

        for input in recovery.resume_parsers {
            self.input_tx
                .send(input)
                .await
                .context("failed to queue resume parser")?;
        }
        for input in recovery.start_full_text {
            self.input_tx
                .send(input)
                .await
                .context("failed to queue restart full-text index")?;
        }
        for input in recovery.run_validations {
            self.input_tx
                .send(input)
                .await
                .context("failed to queue task validation")?;
        }

        Ok(RecoveryQueue {
            artifact_ids,
            validation_task_ids,
        })
    }

    pub async fn shutdown(mut self) -> Result<()> {
        self.shutdown_token.cancel();
        let Some(runtime_task) = self.runtime_task.take() else {
            return Ok(());
        };
        runtime_task
            .await
            .with_context(|| "runtime loop join failed")?;
        Ok(())
    }

    pub async fn run_until_ctrl_c(mut self) -> Result<()> {
        let result = async {
            self.queue_recovery().await?;
            tokio::signal::ctrl_c()
                .await
                .with_context(|| "wait for shutdown signal")
        }
        .await;
        info!(root = %self.layout.root.display(), "shutdown requested");
        let shutdown_result = self.shutdown().await;
        result?;
        shutdown_result
    }

    pub async fn run_until_shutdown(mut self, shutdown: CancellationToken) -> Result<()> {
        let result = self.queue_recovery().await;
        if result.is_ok() {
            shutdown.cancelled().await;
        }
        let shutdown_result = self.shutdown().await;
        result?;
        shutdown_result
    }
}

fn recovery_artifact_ids(recovery: &RecoveryInputs) -> Vec<ArtifactId> {
    recovery
        .resume_parsers
        .iter()
        .filter_map(|input| match input {
            DomainInput::ResumeParser(record) => Some(record.artifact_id),
            _ => None,
        })
        .chain(
            recovery
                .start_full_text
                .iter()
                .filter_map(|input| match input {
                    DomainInput::StartFullTextIndex(request) => Some(request.artifact_id),
                    _ => None,
                }),
        )
        .collect()
}

fn validation_task_ids(recovery: &RecoveryInputs) -> Vec<TaskId> {
    recovery
        .run_validations
        .iter()
        .filter_map(|input| match input {
            DomainInput::RequestTaskValidation(request) => Some(request.task_id),
            _ => None,
        })
        .collect()
}

pub async fn run_instance(instance_dir: std::path::PathBuf) -> Result<()> {
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    let signal_task = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_shutdown.cancel();
        }
    });
    let result = run_instance_with_shutdown(instance_dir, shutdown).await;
    signal_task.abort();
    result
}

pub async fn run_instance_with_shutdown(
    instance_dir: std::path::PathBuf,
    shutdown: CancellationToken,
) -> Result<()> {
    let layout =
        crate::prepare_instance(instance_dir).with_context(|| "prepare instance layout")?;
    let lifecycle = InstanceLifecycle::start(layout.clone(), AutonomyProfile::ReadOnly).await?;
    let api = crate::ApiServer::start(layout).await?;
    println!("daemon_api_socket={}", api.socket_path().display());
    let lifecycle_result = lifecycle.run_until_shutdown(shutdown).await;
    let api_result = api.shutdown().await;
    lifecycle_result?;
    api_result
}
