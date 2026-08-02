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
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
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
            let detail = last_error
                .lock()
                .ok()
                .and_then(|error| error.clone())
                .map_or_else(String::new, |error| format!(" {error}"));
            Err(anyhow::anyhow!("timed out while {wait_context}{detail}"))
        }
    }
}
