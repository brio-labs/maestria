//! Shared benchmark report helpers.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Unix epoch seconds, matching the repository benchmark reports' convention.
pub fn evaluation_date() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

pub const MAX_MEASUREMENT_REASON_CHARS: usize = 512;

/// A benchmark value that records why telemetry is absent instead of treating it as zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Measurement<T> {
    Measured(T),
    Unavailable { reason: String },
    NotApplicable { reason: String },
}

impl<T> Measurement<T> {
    pub fn measured(value: T) -> Self {
        Self::Measured(value)
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    pub fn not_applicable(reason: impl Into<String>) -> Self {
        Self::NotApplicable {
            reason: reason.into(),
        }
    }

    pub fn measured_value(&self) -> Option<&T> {
        match self {
            Self::Measured(value) => Some(value),
            Self::Unavailable { .. } | Self::NotApplicable { .. } => None,
        }
    }

    pub fn is_measured(&self) -> bool {
        matches!(self, Self::Measured(_))
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let reason = match self {
            Self::Measured(_) => return Ok(()),
            Self::Unavailable { reason } | Self::NotApplicable { reason } => reason,
        };
        if reason.trim().is_empty() {
            return Err("measurement reason must not be empty");
        }
        if reason.chars().count() > MAX_MEASUREMENT_REASON_CHARS {
            return Err("measurement reason exceeds the bounded limit");
        }
        Ok(())
    }
}
