//! Stored wire form of a search-trace expansion (R37 DTO boundary).

use maestria_domain::{SearchExpansionStrategy, SearchTraceExpansion};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceExpansion {
    strategy: String,
    added_candidates: Option<u32>,
}

impl StoredSearchTraceExpansion {
    pub(crate) fn from_domain(value: &SearchTraceExpansion) -> Self {
        Self {
            strategy: value.strategy().as_str().to_string(),
            added_candidates: value.added_candidates,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceExpansion, maestria_ports::PortError> {
        let strategy = SearchExpansionStrategy::try_from(self.strategy).map_err(|error| {
            maestria_ports::PortError::InvalidInputContext {
                context: "decode stored search expansion strategy",
                source: error.to_string(),
            }
        })?;
        Ok(SearchTraceExpansion::new(strategy, self.added_candidates))
    }
}
