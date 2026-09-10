use serde::{Deserialize, Serialize};

pub use crate::benchmark_common::{MAX_MEASUREMENT_REASON_CHARS, Measurement};

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
