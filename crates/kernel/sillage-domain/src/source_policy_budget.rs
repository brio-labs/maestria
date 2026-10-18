use serde::{Deserialize, Serialize};

use super::SourceChoiceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SourceResourceBudgetDto")]
pub struct SourceResourceBudget {
    max_entries: u32,
    max_file_bytes: u64,
    max_total_source_bytes: u64,
    /// Shared source-session admission quota, not a physical-memory or RSS cap.
    /// Includes owned documentary state, temporary work and declared native estimates.
    max_in_memory_bytes: u64,
    max_results: u32,
    max_work_units: u64,
    max_duration_ms: u32,
}

#[derive(Deserialize)]
struct SourceResourceBudgetDto {
    max_entries: u32,
    max_file_bytes: u64,
    max_total_source_bytes: u64,
    max_in_memory_bytes: u64,
    max_results: u32,
    max_work_units: u64,
    max_duration_ms: u32,
}

impl SourceResourceBudget {
    pub fn try_new(
        max_entries: u32,
        max_file_bytes: u64,
        max_total_source_bytes: u64,
        max_in_memory_bytes: u64,
        max_results: u32,
        max_work_units: u64,
        max_duration_ms: u32,
    ) -> Result<Self, SourceChoiceError> {
        let valid = max_entries > 0
            && max_entries <= 16_384
            && max_file_bytes > 0
            && max_file_bytes <= 8 * 1024 * 1024
            && max_total_source_bytes >= max_file_bytes
            && max_total_source_bytes <= 128 * 1024 * 1024
            && max_in_memory_bytes >= max_file_bytes
            && max_in_memory_bytes <= 256 * 1024 * 1024
            && (1..=100).contains(&max_results)
            && (1..=1_000_000_000).contains(&max_work_units)
            && (1..=60_000).contains(&max_duration_ms);
        if !valid {
            return Err(SourceChoiceError::InvalidBudget);
        }
        Ok(Self {
            max_entries,
            max_file_bytes,
            max_total_source_bytes,
            max_in_memory_bytes,
            max_results,
            max_work_units,
            max_duration_ms,
        })
    }

    pub const fn session_default() -> Self {
        Self {
            max_entries: 256,
            max_file_bytes: 1_048_576,
            max_total_source_bytes: 16_777_216,
            max_in_memory_bytes: 67_108_864,
            max_results: 10,
            max_work_units: 2_000_000,
            max_duration_ms: 10_000,
        }
    }

    pub const fn max_entries(self) -> u32 {
        self.max_entries
    }

    pub const fn max_file_bytes(self) -> u64 {
        self.max_file_bytes
    }

    pub const fn max_total_source_bytes(self) -> u64 {
        self.max_total_source_bytes
    }

    /// Admission quota for session-owned state and temporary/native estimates.
    /// Caller-owned copies after transfer are outside it; process-tree RSS is measured separately.
    pub const fn max_in_memory_bytes(self) -> u64 {
        self.max_in_memory_bytes
    }

    pub const fn max_results(self) -> u32 {
        self.max_results
    }

    pub const fn max_work_units(self) -> u64 {
        self.max_work_units
    }

    pub const fn max_duration_ms(self) -> u32 {
        self.max_duration_ms
    }
}

impl TryFrom<SourceResourceBudgetDto> for SourceResourceBudget {
    type Error = SourceChoiceError;

    fn try_from(dto: SourceResourceBudgetDto) -> Result<Self, Self::Error> {
        Self::try_new(
            dto.max_entries,
            dto.max_file_bytes,
            dto.max_total_source_bytes,
            dto.max_in_memory_bytes,
            dto.max_results,
            dto.max_work_units,
            dto.max_duration_ms,
        )
    }
}
