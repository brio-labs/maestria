use serde::{Deserialize, Serialize};

use super::SearchCompatibilityError;

fn default_candidate_budget() -> u32 {
    10_000
}

fn default_work_budget() -> u64 {
    100_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SearchBudgetDto")]
pub struct SearchBudget {
    max_tokens: u32,
    max_latency_ms: u32,
    max_queries: u32,
    max_stages: u32,
    max_web_requests: u32,
    max_bytes_read: u64,
    max_concurrency: u32,
    max_candidates: u32,
    max_work_units: u64,
}

#[derive(Deserialize)]
struct SearchBudgetDto {
    max_tokens: u32,
    max_latency_ms: u32,
    max_queries: u32,
    max_stages: u32,
    max_web_requests: u32,
    max_bytes_read: u64,
    max_concurrency: u32,
    max_candidates: u32,
    max_work_units: u64,
}

/// Complete resource limits used to construct a governed search budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchBudgetLimits {
    pub max_tokens: u32,
    pub max_latency_ms: u32,
    pub max_queries: u32,
    pub max_stages: u32,
    pub max_web_requests: u32,
    pub max_bytes_read: u64,
    pub max_concurrency: u32,
    pub max_candidates: u32,
    pub max_work_units: u64,
}

impl TryFrom<SearchBudgetDto> for SearchBudget {
    type Error = SearchCompatibilityError;

    fn try_from(dto: SearchBudgetDto) -> Result<Self, Self::Error> {
        Self::with_execution_limits(SearchBudgetLimits {
            max_tokens: dto.max_tokens,
            max_latency_ms: dto.max_latency_ms,
            max_queries: dto.max_queries,
            max_stages: dto.max_stages,
            max_web_requests: dto.max_web_requests,
            max_bytes_read: dto.max_bytes_read,
            max_concurrency: dto.max_concurrency,
            max_candidates: dto.max_candidates,
            max_work_units: dto.max_work_units,
        })
    }
}

impl SearchBudget {
    pub fn new(max_tokens: u32, max_latency_ms: u32) -> Result<Self, SearchCompatibilityError> {
        Self::with_limits(max_tokens, max_latency_ms, 1, 1, 0)
    }

    pub fn with_limits(
        max_tokens: u32,
        max_latency_ms: u32,
        max_queries: u32,
        max_stages: u32,
        max_web_requests: u32,
    ) -> Result<Self, SearchCompatibilityError> {
        Self::with_resource_limits(
            max_tokens,
            max_latency_ms,
            max_queries,
            max_stages,
            max_web_requests,
            0,
            1,
        )
    }

    pub fn with_execution_limits(
        limits: SearchBudgetLimits,
    ) -> Result<Self, SearchCompatibilityError> {
        let SearchBudgetLimits {
            max_tokens,
            max_latency_ms,
            max_queries,
            max_stages,
            max_web_requests,
            max_bytes_read,
            max_concurrency,
            max_candidates,
            max_work_units,
        } = limits;
        if max_candidates == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_candidates must be greater than 0",
            ));
        }
        if max_work_units == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_work_units must be greater than 0",
            ));
        }
        if max_tokens == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_tokens must be greater than 0",
            ));
        }
        if max_latency_ms == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_latency_ms must be greater than 0",
            ));
        }
        if max_queries == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_queries must be greater than 0",
            ));
        }
        if max_stages == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_stages must be greater than 0",
            ));
        }
        if max_concurrency == 0 {
            return Err(SearchCompatibilityError::InvalidBudget(
                "max_concurrency must be greater than 0",
            ));
        }
        Ok(Self {
            max_tokens,
            max_latency_ms,
            max_queries,
            max_stages,
            max_web_requests,
            max_bytes_read,
            max_concurrency,
            max_candidates,
            max_work_units,
        })
    }

    pub fn with_resource_limits(
        max_tokens: u32,
        max_latency_ms: u32,
        max_queries: u32,
        max_stages: u32,
        max_web_requests: u32,
        max_bytes_read: u64,
        max_concurrency: u32,
    ) -> Result<Self, SearchCompatibilityError> {
        Self::with_execution_limits(SearchBudgetLimits {
            max_tokens,
            max_latency_ms,
            max_queries,
            max_stages,
            max_web_requests,
            max_bytes_read,
            max_concurrency,
            max_candidates: default_candidate_budget(),
            max_work_units: default_work_budget(),
        })
    }

    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    pub fn max_latency_ms(&self) -> u32 {
        self.max_latency_ms
    }

    pub fn max_queries(&self) -> u32 {
        self.max_queries
    }

    pub fn max_stages(&self) -> u32 {
        self.max_stages
    }

    pub fn max_web_requests(&self) -> u32 {
        self.max_web_requests
    }

    pub fn max_bytes_read(&self) -> u64 {
        self.max_bytes_read
    }

    pub fn max_concurrency(&self) -> u32 {
        self.max_concurrency
    }

    pub fn max_candidates(&self) -> u32 {
        self.max_candidates
    }

    pub fn max_work_units(&self) -> u64 {
        self.max_work_units
    }

    pub fn execution_budget(
        &self,
        max_results: u32,
    ) -> Result<super::SearchExecutionBudget, SearchCompatibilityError> {
        super::SearchExecutionBudget::new(
            u64::from(max_results),
            u64::from(self.max_candidates),
            self.max_work_units,
            self.max_bytes_read,
        )
    }
}
