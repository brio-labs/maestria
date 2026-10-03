//! Shared metric formatting helpers.

use std::time::Duration;

/// Format an elapsed duration as `M:SS` or `H:MM:SS` below one hour.
pub fn format_duration(elapsed: Duration) -> String {
    let total_seconds = elapsed.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Compute a per-second rate from a count and an elapsed duration.
pub fn rate_per_second(count: u64, elapsed: Duration) -> f64 {
    count as f64 / elapsed.as_secs_f64().max(0.001)
}
