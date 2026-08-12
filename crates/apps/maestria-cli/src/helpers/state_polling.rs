use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use maestria_daemon::db_retry::{is_database_busy, run_database_retry};
use maestria_domain::KernelState;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::{sleep, timeout};

pub(crate) fn load_kernel_state_with_retry(
    layout: &InstanceLayout,
    context: &'static str,
) -> Result<KernelState> {
    retry_db_busy(context, || {
        maestria_daemon::load_kernel_state(layout).with_context(|| context)
    })
}

/// Retry a synchronous database operation while the instance is transiently
/// locked, delegating the retry cadence to the shared daemon policy
/// (`maestria_daemon::db_retry`).
///
/// The shared daemon constants (`RETRY_ATTEMPTS` / `RETRY_DELAY`) drive the
/// loop. A busy error that outlives the retry budget is reported as a
/// timeout with the last underlying error, matching the historical CLI
/// wording.
pub(crate) fn retry_db_busy<T>(context: &str, operation: impl Fn() -> Result<T>) -> Result<T> {
    match run_database_retry(operation) {
        Ok(output) => Ok(output),
        Err(error) if is_database_busy(&error) => {
            Err(anyhow::anyhow!("timed out while {context}: {error}"))
        }
        Err(error) => Err(error),
    }
}

/// Poll persisted kernel state until `predicate` holds, within `timeout_budget`.
///
/// This is the single CLI policy for waiting on durable kernel state: command
/// modules pass a predicate instead of restating the poll-and-retry loop.
/// Transient database-lock errors are detected with the shared daemon matcher
/// ([`is_database_busy`]) and retried at the CLI polling cadence; the last such
/// error is preserved in the timeout message. The returned state is the one
/// that satisfied the predicate.
///
/// Prefer [`wait_for_artifact_state`] for per-artifact waits: polling the
/// full replayed kernel state costs a full event-log replay per poll, which
/// dominates batch ingestion time as the log grows.
pub(crate) async fn wait_for_artifact_state(
    layout: &InstanceLayout,
    artifact_id: maestria_domain::ArtifactId,
    timeout_budget: Duration,
    wait_context: String,
    mut predicate: impl FnMut(&maestria_domain::Artifact) -> bool,
) -> Result<()> {
    let last_error = Arc::new(Mutex::new(None::<String>));
    let result = timeout(timeout_budget, async {
        loop {
            let state = retry_db_busy(&wait_context, || {
                let store =
                    maestria_storage_sqlite::SqliteStore::open_read_only(&layout.database_path)?;
                maestria_ports::ArtifactRepository::get(&store, artifact_id)
                    .with_context(|| format!("load artifact state while {wait_context}"))
            });
            match state {
                Ok(Some(artifact)) => {
                    if predicate(&artifact) {
                        return Ok(());
                    }
                }
                Ok(None) => {}
                Err(error) if is_database_busy(&error) => {
                    let mut slot = last_error
                        .lock()
                        .map_err(|_| anyhow::anyhow!("state-poll mutex poisoned"))?;
                    *slot = Some(error.to_string());
                }
                Err(error) => return Err(error),
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await;

    match result {
        Ok(inner) => inner,
        Err(_elapsed) => {
            let detail = match last_error.lock() {
                Ok(error) => error
                    .as_deref()
                    .map_or_else(String::new, |error| format!(" {error}")),
                Err(_) => " state-poll mutex poisoned while reading last error".to_string(),
            };
            Err(anyhow::anyhow!("timed out while {wait_context}{detail}"))
        }
    }
}
pub(crate) async fn wait_for_kernel_state(
    layout: &InstanceLayout,
    timeout_budget: Duration,
    wait_context: String,
    predicate: impl Fn(&KernelState) -> bool,
) -> Result<KernelState> {
    let last_error = Arc::new(Mutex::new(None::<String>));
    let last_error_for_wait = Arc::clone(&last_error);
    let result = timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| format!("load kernel state while {wait_context}"))
            {
                Ok(state) => {
                    if predicate(&state) {
                        return Ok::<_, anyhow::Error>(state);
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if is_database_busy(&error) => {
                    match last_error_for_wait.lock() {
                        Ok(mut slot) => *slot = Some(error.to_string()),
                        Err(_) => {
                            return Err(anyhow::anyhow!(
                                "record last database-busy error: state-poll mutex poisoned"
                            ));
                        }
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(state)) => Ok(state),
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => {
            let detail = match last_error.lock() {
                Ok(error) => error
                    .as_deref()
                    .map_or_else(String::new, |error| format!(" {error}")),
                Err(_) => " state-poll mutex poisoned while reading last error".to_string(),
            };
            Err(anyhow::anyhow!("timed out while {wait_context}{detail}"))
        }
    }
}
