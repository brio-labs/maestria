use anyhow::{Context, Result};
use maestria_governance::AutonomyProfile;
use tokio_util::sync::CancellationToken;

use crate::InstanceLifecycle;

/// Runs an instance until the process receives SIGINT.
///
/// Cancellation: installs a SIGINT handler that cancels the internal shutdown
/// token; the daemon then shuts down gracefully and the function returns the
/// shutdown result. The signal task is aborted once shutdown completes.
pub async fn run_instance(instance_dir: std::path::PathBuf) -> Result<()> {
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    let (signal_result_tx, mut signal_result_rx) = tokio::sync::oneshot::channel();
    let signal_task = tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => signal_shutdown.cancel(),
            Err(error) => {
                if signal_result_tx.send(error).is_err() {
                    tracing::debug!(
                        "SIGINT result receiver disconnected before signal error delivery"
                    );
                }
            }
        }
    });
    let result = run_instance_with_shutdown(instance_dir, shutdown).await;
    signal_task.abort();
    match result {
        Ok(()) => match signal_result_rx.try_recv() {
            Ok(error) => Err(anyhow::anyhow!("failed to wait for SIGINT: {error}")),
            Err(_) => Ok(()),
        },
        Err(error) => Err(error),
    }
}

/// The autonomy profile the daemon runtime runs under.
///
/// The default is `ReadOnly`: a background daemon must not take
/// medium-risk actions (vector indexing, validations, graph updates)
/// without an approval flow, so those effects are rejected at admission.
/// Operators who run the daemon as the trusted primary ingestion path for
/// large roots (e.g. a home directory) can opt into the permissive
/// profile explicitly via `MAESTRIA_DAEMON_PROFILE=trusted-workspace`.
fn daemon_profile() -> AutonomyProfile {
    let profile = match std::env::var("MAESTRIA_DAEMON_PROFILE").as_deref() {
        Ok("trusted-workspace") => AutonomyProfile::TrustedWorkspace,
        Ok(other) => {
            tracing::warn!(
                profile = %other,
                "unknown MAESTRIA_DAEMON_PROFILE; falling back to read-only"
            );
            AutonomyProfile::ReadOnly
        }
        _ => AutonomyProfile::ReadOnly,
    };
    tracing::info!(?profile, "daemon runtime profile");
    profile
}

/// Runs an instance until the provided shutdown token is cancelled.
///
/// Cancellation: cancelling `shutdown` triggers graceful teardown of the
/// lifecycle (runtime loop) followed by the API server; both shutdown results
/// are propagated (lifecycle errors win on double failure). Returns once the
/// instance is fully stopped.
pub async fn run_instance_with_shutdown(
    instance_dir: std::path::PathBuf,
    shutdown: CancellationToken,
) -> Result<()> {
    let layout = crate::instance_setup::prepare_instance(instance_dir)
        .with_context(|| "prepare instance layout")?;
    let profile = daemon_profile();
    let lifecycle = InstanceLifecycle::start(layout.clone(), profile).await?;
    let runtime = lifecycle.runtime_handle();
    let api = crate::api::ApiServer::start(layout, runtime).await?;
    println!("daemon_api_socket={}", api.socket_path().display());
    let lifecycle_result = lifecycle.run_until_shutdown(shutdown).await;
    let api_result = api.shutdown().await;
    lifecycle_result?;
    api_result
}
