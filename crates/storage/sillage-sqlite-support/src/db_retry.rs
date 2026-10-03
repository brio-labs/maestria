//! Shared database-busy retry policy.
//!
//! The SQLite layer produces "database is locked" / "database is busy"
//! failures (`SQLITE_BUSY`). Callers across daemon API, CLI, and runtime
//! share this matcher and the retry constants so the classification and the
//! retry budget cannot drift (R28: the layer producing an error defines it).

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
