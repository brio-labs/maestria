use anyhow::{Context, Result, anyhow};
use maestria_governance::AutonomyProfile;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::InstanceLifecycle;

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
    let input_tx = lifecycle.input_sender();

    let state =
        crate::load_kernel_state(&layout).with_context(|| "load kernel state for api context")?;
    let manifest_contents = std::fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = maestria_core::InstanceManifest::decode(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let adapters = Arc::new(
        crate::build_adapters(
            &layout,
            &state,
            &manifest,
            None,
            maestria_retrieval::RepositoryExecutionPolicy::Shadow,
            true,
        )
        .with_context(|| "build api adapters")?,
    );
    let governance = Arc::new(maestria_runtime::Governance {
        classifier: Arc::new(maestria_governance::DefaultRiskClassifier),
        approval_gate: Arc::new(maestria_governance::DefaultApprovalGate),
        validation_gate: Arc::new(maestria_governance::DefaultValidationGate::new(true)),
        memory_promotion_gate: Arc::new(maestria_governance::DefaultMemoryPromotionGate),
    });

    let api = crate::ApiServer::start(layout, input_tx, adapters, governance).await?;
    println!("daemon_api_socket={}", api.socket_path().display());
    let lifecycle_result = lifecycle.run_until_shutdown(shutdown).await;
    let api_result = api.shutdown().await;
    lifecycle_result?;
    api_result
}
