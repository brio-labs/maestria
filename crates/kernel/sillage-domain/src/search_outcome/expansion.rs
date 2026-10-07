//! Search expansion strategies and their trace records.
//!
//! One concept per module (R13): expansion strategy is a closed,
//! wire-stable enum; the trace expansion record carries it alongside the
//! added-candidate count.

use serde::{Deserialize, Serialize};

use crate::search::SearchCompatibilityError;

/// Closed set of query-expansion strategies a trace can record.
///
/// The wire form matches the pre-enum string values (`hierarchy`,
/// `hierarchy+graph`, `synonym`) so stored traces and rendered CLI output
/// stay byte-identical (R29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum SearchExpansionStrategy {
    Hierarchy,
    HierarchyGraph,
    Synonym,
}

impl SearchExpansionStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hierarchy => "hierarchy",
            Self::HierarchyGraph => "hierarchy+graph",
            Self::Synonym => "synonym",
        }
    }
}

impl std::fmt::Display for SearchExpansionStrategy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<SearchExpansionStrategy> for String {
    fn from(value: SearchExpansionStrategy) -> Self {
        value.as_str().to_string()
    }
}

impl TryFrom<String> for SearchExpansionStrategy {
    type Error = SearchCompatibilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "hierarchy" => Ok(Self::Hierarchy),
            "hierarchy+graph" => Ok(Self::HierarchyGraph),
            "synonym" => Ok(Self::Synonym),
            _ => Err(SearchCompatibilityError::TracePlanMismatch(
                "unknown search expansion strategy",
            )),
        }
    }
}

/// One recorded query expansion in a search trace.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchTraceExpansion {
    strategy: SearchExpansionStrategy,
    pub added_candidates: Option<u32>,
}

impl SearchTraceExpansion {
    pub fn new(strategy: SearchExpansionStrategy, added_candidates: Option<u32>) -> Self {
        Self {
            strategy,
            added_candidates,
        }
    }

    pub fn strategy(&self) -> SearchExpansionStrategy {
        self.strategy
    }
}

impl std::fmt::Debug for SearchTraceExpansion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchTraceExpansion")
            .field("strategy", &self.strategy.as_str())
            .field("added_candidates", &self.added_candidates)
            .finish()
    }
}
