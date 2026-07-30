use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Passed,
    Failed,
    NotDetected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseRetentionPolicy {
    NoRetention,
    ProviderDefined,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseProviderDisclosure {
    pub remote: bool,
    pub retention: LearnedSparseRetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseOperationMeasurement {
    pub elapsed_ms: Measurement<u64>,
    pub throughput_items_per_second: Measurement<u64>,
    pub cost_micros: Measurement<u64>,
    pub energy_millijoules: Measurement<u64>,
}

impl LearnedSparseOperationMeasurement {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.elapsed_ms.validate()?;
        self.throughput_items_per_second.validate()?;
        self.cost_micros.validate()?;
        self.energy_millijoules.validate()
    }
}
