/// Convert a platform-sized count to a bounded execution counter.
pub fn saturating_u64(value: usize) -> u64 {
    value as u64
}

/// Convert a counter to the current platform's collection index range.
pub fn saturating_usize(value: u64) -> usize {
    value.min(usize::MAX as u64) as usize
}

/// Convert a platform-sized count to a bounded 32-bit execution counter.
pub fn saturating_u32(value: usize) -> u32 {
    value.min(u32::MAX as usize) as u32
}

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use super::SearchCompatibilityError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchExecutionBudgetDto")]
pub struct SearchExecutionBudget {
    max_results: NonZeroU64,
    max_candidates: NonZeroU64,
    max_work_units: NonZeroU64,
    max_bytes_read: Option<NonZeroU64>,
}

#[derive(Deserialize)]
struct SearchExecutionBudgetDto {
    max_results: u64,
    max_candidates: u64,
    max_work_units: u64,
    max_bytes_read: Option<u64>,
}

impl TryFrom<SearchExecutionBudgetDto> for SearchExecutionBudget {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchExecutionBudgetDto) -> Result<Self, Self::Error> {
        Self::with_byte_limit(
            dto.max_results,
            dto.max_candidates,
            dto.max_work_units,
            dto.max_bytes_read.and_then(NonZeroU64::new),
        )
    }
}
impl Default for SearchExecutionBudget {
    fn default() -> Self {
        Self {
            max_results: NonZeroU64::MIN,
            max_candidates: NonZeroU64::MIN,
            max_work_units: NonZeroU64::MIN,
            max_bytes_read: None,
        }
    }
}
impl Default for SearchExecution {
    fn default() -> Self {
        Self {
            budget: SearchExecutionBudget::default(),
            usage: SearchExecutionUsage::default(),
            completion: SearchExecutionCompletion::Complete,
        }
    }
}

impl SearchExecutionBudget {
    pub fn new(
        max_results: u64,
        max_candidates: u64,
        max_work_units: u64,
        max_bytes_read: u64,
    ) -> Result<Self, SearchCompatibilityError> {
        Self::with_byte_limit(
            max_results,
            max_candidates,
            max_work_units,
            NonZeroU64::new(max_bytes_read),
        )
    }

    pub fn with_byte_limit(
        max_results: u64,
        max_candidates: u64,
        max_work_units: u64,
        max_bytes_read: Option<NonZeroU64>,
    ) -> Result<Self, SearchCompatibilityError> {
        let max_results = NonZeroU64::new(max_results).ok_or(
            SearchCompatibilityError::InvalidBudget("max_results must be greater than 0"),
        )?;
        let max_candidates = NonZeroU64::new(max_candidates).ok_or(
            SearchCompatibilityError::InvalidBudget("max_candidates must be greater than 0"),
        )?;
        let max_work_units = NonZeroU64::new(max_work_units).ok_or(
            SearchCompatibilityError::InvalidBudget("max_work_units must be greater than 0"),
        )?;
        Ok(Self {
            max_results,
            max_candidates,
            max_work_units,
            max_bytes_read,
        })
    }

    pub fn max_results(self) -> u64 {
        self.max_results.get()
    }

    pub fn max_candidates(self) -> u64 {
        self.max_candidates.get()
    }

    pub fn max_work_units(self) -> u64 {
        self.max_work_units.get()
    }

    pub fn max_bytes_read(self) -> Option<NonZeroU64> {
        self.max_bytes_read
    }

    pub fn bytes_unlimited(self) -> bool {
        self.max_bytes_read.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SearchExecutionUsage {
    pub results: u64,
    pub candidates: u64,
    pub work_units: u64,
    pub bytes_read: u64,
}

impl SearchExecutionUsage {
    pub const fn new(results: u64, candidates: u64, work_units: u64, bytes_read: u64) -> Self {
        Self {
            results,
            candidates,
            work_units,
            bytes_read,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchExecutionResource {
    Results,
    Candidates,
    WorkUnits,
    BytesRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchExecutionCompletion {
    Complete,
    Exhausted(SearchExecutionResource),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchExecution {
    pub budget: SearchExecutionBudget,
    pub usage: SearchExecutionUsage,
    pub completion: SearchExecutionCompletion,
}

impl SearchExecution {
    pub const fn new(
        budget: SearchExecutionBudget,
        usage: SearchExecutionUsage,
        completion: SearchExecutionCompletion,
    ) -> Self {
        Self {
            budget,
            usage,
            completion,
        }
    }
}
