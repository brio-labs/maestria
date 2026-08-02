//! Shared database-busy retry policy.
//!
//! The daemon API and the CLI each carried their own copy of the SQLite
//! busy-retry loop and lock matcher, and the matchers drifted (R28). This
//! module is the single source of truth for both: the API's blocking
//! handlers, the API's async handlers, and the CLI's synchronous retry all
//! delegate here.

use std::future::Future;
use std::time::Duration;

/// Maximum number of attempts before a busy database operation fails.
pub const RETRY_ATTEMPTS: u32 = 80;

/// Delay between retry attempts while the database is busy.
pub const RETRY_DELAY: Duration = Duration::from_millis(50);

/// Whether `error` reports a transiently busy or locked database.
///
/// Union of the daemon's and CLI's previous matchers: any rendered message
/// containing "locked" or "busy" (case-insensitively), which covers
/// "database is locked", "database is busy", and "SQLITE_BUSY" variants.
pub fn is_database_busy(error: &impl std::fmt::Display) -> bool {
    let rendered = format!("{error:#}").to_lowercase();
    rendered.contains("locked") || rendered.contains("busy")
}

/// Run `operation`, retrying while the database is transiently busy.
///
/// Retries up to [`RETRY_ATTEMPTS`] times, sleeping [`RETRY_DELAY`] between
/// attempts. Non-busy errors propagate immediately; when the budget is
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
