//! Shared fixtures for the vector-sqlite test families (Rule 26: fixtures
//! are shared through explicit helpers, never copied between test modules).

use maestria_domain::SearchExecutionBudget;
use maestria_ports::PortError;

pub(crate) fn search_budget(limit: u64) -> Result<SearchExecutionBudget, PortError> {
    SearchExecutionBudget::new(limit, 10_000, 100_000, 0).map_err(|error| {
        PortError::InternalContext {
            context: "construct test search execution budget",
            source: error.to_string(),
        }
    })
}
