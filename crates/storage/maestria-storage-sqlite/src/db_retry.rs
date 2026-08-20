//! Shared database-busy retry policy (re-exported from `maestria-sqlite-support`).
//!
//! The SQLite layer is the single owner of the `SQLITE_BUSY` / `SQLITE_LOCKED`
//! classification and the retry budget. Callers in daemon API/CLI and the
//! runtime import the constants and matcher from here so the policy cannot
//! drift (R28).

pub use maestria_sqlite_support::{RETRY_ATTEMPTS, RETRY_DELAY, is_database_busy};
use std::future::Future;
use std::time::Duration;

/// Run `operation`, retrying while the database is transiently busy.
/// exhausted the last busy error is returned.
pub fn run_database_retry<T, E>(operation: impl Fn() -> Result<T, E>) -> Result<T, E>
where
    E: std::fmt::Display,
{
    let mut remaining = RETRY_ATTEMPTS;
    loop {
        let can_retry = remaining > 1;
        remaining = remaining.saturating_sub(1);
        match operation() {
            Ok(output) => return Ok(output),
            Err(error) if is_database_busy(&error) && can_retry => {
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Async twin of [`run_database_retry`] for operations that themselves await;
/// both share the same attempt/delay policy constants.
///
/// # Cancellation
/// Dropping the returned future aborts the retry loop and cancels the
/// in-flight `operation().await`; no further attempts are made and the
/// operation's own cancellation semantics apply to the in-flight call.
pub async fn run_database_retry_async<T, E, F, Fut>(mut operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut remaining = RETRY_ATTEMPTS;
    loop {
        let can_retry = remaining > 1;
        remaining = remaining.saturating_sub(1);
        match operation().await {
            Ok(output) => return Ok(output),
            Err(error) if is_database_busy(&error) && can_retry => {
                tokio::time::sleep(RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Whether the error should be retried with the shared busy policy (extracts
/// the check so callers that need custom sleep can reuse the matcher).
pub fn should_retry_busy<E: std::fmt::Display>(error: &E, remaining: u32) -> bool {
    is_database_busy(error) && remaining > 1
}

pub fn retry_delay() -> Duration {
    RETRY_DELAY
}

pub fn retry_attempts() -> u32 {
    RETRY_ATTEMPTS
}
