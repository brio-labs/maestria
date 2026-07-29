use anyhow::{Context, Result};
use maestria_governance::AutonomyProfile;
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
    let layout = crate::instance_setup::prepare_instance(instance_dir)
        .with_context(|| "prepare instance layout")?;
    let lifecycle = InstanceLifecycle::start(layout.clone(), AutonomyProfile::ReadOnly).await?;
    let runtime = lifecycle.runtime_handle();
    let api = crate::api::ApiServer::start(layout, runtime).await?;
    println!("daemon_api_socket={}", api.socket_path().display());
    let lifecycle_result = lifecycle.run_until_shutdown(shutdown).await;
    let api_result = api.shutdown().await;
    lifecycle_result?;
    api_result
}
