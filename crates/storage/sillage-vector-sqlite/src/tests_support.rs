//! Shared fixtures for the vector-sqlite test families (Rule 26: fixtures
//! are shared through explicit helpers, never copied between test modules).

use sillage_domain::SearchExecutionBudget;
use sillage_ports::PortError;

pub(crate) fn search_budget(limit: u64) -> Result<SearchExecutionBudget, PortError> {
    sillage_test_support::search_budget(limit).map_err(|error| {
        PortError::internal("construct test search execution budget", error.to_string())
    })
}
