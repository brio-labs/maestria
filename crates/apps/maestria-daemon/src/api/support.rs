//! Shared API-boundary mechanics, kept independent of operation-specific services.
//!
//! Retry policy and persisted state loading are used by multiple handlers. Keeping them here
//! prevents search, model-agent, and read services from depending on one another's internals.
//!
//! The database-busy retry policy itself lives in [`maestria_storage_sqlite::db_retry`]; the
//! wrappers here adapt it to the daemon's blocking-handler convention.

use std::future::Future;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_domain::KernelState;

pub(super) async fn run_database_retry<T, F>(operation_name: &str, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: Fn() -> Result<T> + Send + Sync + 'static,
{
    let operation = Arc::new(operation);
    tokio::task::spawn_blocking(move || {
        maestria_storage_sqlite::db_retry::run_database_retry(move || operation())
    })
    .await
    .map_err(|error| anyhow!("{operation_name} task failed: {error}"))?
}

/// Async twin of [`run_database_retry`] for handlers whose operation itself
/// awaits; both share the retry policy in [`maestria_storage_sqlite::db_retry`].
pub(super) async fn run_database_retry_async<T, F, Fut>(
    _operation_name: &str,
    operation: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    maestria_storage_sqlite::db_retry::run_database_retry_async(operation).await
}

pub(super) fn load_state_and_manifest(
    layout: &InstanceLayout,
) -> Result<(KernelState, InstanceManifest)> {
    let state = crate::instance_setup::load_kernel_state(layout)?;
    let manifest = InstanceManifest::decode(&std::fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    Ok((state, manifest))
}

/// Load the manifest plus the kernel-state slice read-only search assembly
/// consumes (the index-generation registry), skipping the full event-log
/// replay. See [`crate::instance_setup::load_search_generations_state`].
pub(super) fn load_search_generations_and_manifest(
    layout: &InstanceLayout,
) -> Result<(KernelState, InstanceManifest)> {
    let state = crate::instance_setup::load_search_generations_state(layout)?;
    let manifest = InstanceManifest::decode(&std::fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    Ok((state, manifest))
}
