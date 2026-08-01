//! Shared API-boundary mechanics, kept independent of operation-specific services.
//!
//! Retry policy and persisted state loading are used by multiple handlers. Keeping them here
//! prevents search, model-agent, and read services from depending on one another's internals.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::KernelState;

pub(super) const DATABASE_RETRY_ATTEMPTS: usize = 80;
pub(super) const DATABASE_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(super) async fn run_database_retry<T, F>(operation_name: &str, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: Fn() -> Result<T> + Send + Sync + 'static,
{
    let operation = Arc::new(operation);
    for attempt in 0..DATABASE_RETRY_ATTEMPTS {
        let op = Arc::clone(&operation);
        let result = tokio::task::spawn_blocking(move || op())
            .await
            .map_err(|error| anyhow!("{operation_name} task failed: {error}"))?;
        match result {
            Ok(response) => return Ok(response),
            Err(error) if is_database_locked(&error) && attempt + 1 < DATABASE_RETRY_ATTEMPTS => {
                tokio::time::sleep(DATABASE_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(anyhow!("{operation_name} retries exhausted"))
}

/// Async twin of [`run_database_retry`] for handlers whose operation itself
/// awaits; both share the same attempt/delay policy constants.
pub(super) async fn run_database_retry_async<T, F, Fut>(
    operation_name: &str,
    mut operation: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    for attempt in 0..DATABASE_RETRY_ATTEMPTS {
        match operation().await {
            Ok(response) => return Ok(response),
            Err(error) if is_database_locked(&error) && attempt + 1 < DATABASE_RETRY_ATTEMPTS => {
                tokio::time::sleep(DATABASE_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(anyhow!("{operation_name} retries exhausted"))
}

pub(super) fn is_database_locked(error: &anyhow::Error) -> bool {
    let rendered = format!("{error:#}");
    rendered.contains("locked") || rendered.contains("busy")
}

pub(super) fn load_state_and_manifest(
    layout: &InstanceLayout,
) -> Result<(KernelState, InstanceManifest)> {
    let state = crate::instance_setup::load_kernel_state(layout)?;
    let manifest = InstanceManifest::decode(&std::fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    Ok((state, manifest))
}
