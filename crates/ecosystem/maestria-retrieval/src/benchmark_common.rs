//! Shared benchmark report helpers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Unix epoch seconds, matching the repository benchmark reports' convention.
pub fn evaluation_date() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "unknown".to_string(),
    }
}
