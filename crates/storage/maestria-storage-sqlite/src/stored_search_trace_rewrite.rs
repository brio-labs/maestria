//! Rewrite-stage wire mirrors for the stored search trace
//! (`StoredSearchTraceRewrite`, its origin/stage enums and accounting).
//! Re-exported by `crate::payloads::stored_search_trace` so consumers keep a
//! single import path.

use maestria_domain::{
    SearchRewriteAccounting, SearchRewriteOrigin, SearchRewriteStage, SearchTraceRewrite,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchRewriteOrigin {
    Original,
    Deterministic,
    ModelProposal,
    Feedback,
    MissingSlot,
}

impl StoredSearchRewriteOrigin {
    pub(crate) fn from_domain(value: &SearchRewriteOrigin) -> Self {
        match value {
            SearchRewriteOrigin::Original => Self::Original,
            SearchRewriteOrigin::Deterministic => Self::Deterministic,
            SearchRewriteOrigin::ModelProposal => Self::ModelProposal,
            SearchRewriteOrigin::Feedback => Self::Feedback,
            SearchRewriteOrigin::MissingSlot { .. } => Self::MissingSlot,
        }
    }

    /// Converts the stored origin back to the domain type.
    ///
    /// The `MissingSlot` variant carries its slot in the domain; the stored
    /// representation keeps it in the sibling `missing_slot` column, so the
    /// caller supplies the decoded slot and this conversion rejects a
    /// missing-slot origin without a slot (R56).
    pub(crate) fn try_into_domain(
        self,
        missing_slot: Option<String>,
    ) -> Result<SearchRewriteOrigin, maestria_ports::PortError> {
        Ok(match self {
            Self::Original => SearchRewriteOrigin::Original,
            Self::Deterministic => SearchRewriteOrigin::Deterministic,
            Self::ModelProposal => SearchRewriteOrigin::ModelProposal,
            Self::Feedback => SearchRewriteOrigin::Feedback,
            Self::MissingSlot => {
                let slot =
                    missing_slot.ok_or_else(|| maestria_ports::PortError::InternalContext {
                        context: "stored rewrite is a missing-slot rewrite without a slot",
                        source: "missing_slot column is NULL".to_string(),
                    })?;
                SearchRewriteOrigin::MissingSlot { slot }
            }
        })
    }
}

crate::stored_enum! {
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StoredSearchRewriteStage <=> SearchRewriteStage {
        InitialRetrieval,
        Reranking,
        IterativeRetrieval,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchRewriteAccounting {
    token_estimate: u32,
    latency_budget_units: u32,
}

impl StoredSearchRewriteAccounting {
    pub(crate) fn from_domain(value: &SearchRewriteAccounting) -> Self {
        Self {
            token_estimate: value.token_estimate,
            latency_budget_units: value.latency_budget_units,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchRewriteAccounting, maestria_ports::PortError> {
        Ok(SearchRewriteAccounting {
            token_estimate: self.token_estimate,
            latency_budget_units: self.latency_budget_units,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceRewrite {
    query: String,
    origin: StoredSearchRewriteOrigin,
    stage: StoredSearchRewriteStage,
    accounting: StoredSearchRewriteAccounting,
    missing_slot: Option<String>,
}

impl StoredSearchTraceRewrite {
    pub(crate) fn from_domain(value: &SearchTraceRewrite) -> Self {
        let missing_slot = match &value.origin {
            SearchRewriteOrigin::MissingSlot { slot } => Some(slot.clone()),
            _ => None,
        };
        Self {
            query: value.query.clone(),
            origin: StoredSearchRewriteOrigin::from_domain(&value.origin),
            stage: StoredSearchRewriteStage::from_domain(value.stage),
            accounting: StoredSearchRewriteAccounting::from_domain(&value.accounting),
            missing_slot,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceRewrite, maestria_ports::PortError> {
        Ok(SearchTraceRewrite {
            query: self.query,
            origin: self.origin.try_into_domain(self.missing_slot)?,
            stage: self.stage.try_into_domain()?,
            accounting: self.accounting.try_into_domain()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::{SearchRewriteOrigin, SearchRewriteStage};

    use super::*;

    #[test]
    fn rewrite_enum_variants_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        for origin in [
            SearchRewriteOrigin::Original,
            SearchRewriteOrigin::Deterministic,
            SearchRewriteOrigin::ModelProposal,
            SearchRewriteOrigin::Feedback,
            SearchRewriteOrigin::MissingSlot {
                slot: "missing claim".to_string(),
            },
        ] {
            let slot = match &origin {
                SearchRewriteOrigin::MissingSlot { slot } => Some(slot.clone()),
                _ => None,
            };
            assert_eq!(
                StoredSearchRewriteOrigin::from_domain(&origin).try_into_domain(slot)?,
                origin
            );
        }
        for stage in [
            SearchRewriteStage::InitialRetrieval,
            SearchRewriteStage::Reranking,
            SearchRewriteStage::IterativeRetrieval,
        ] {
            assert_eq!(
                StoredSearchRewriteStage::from_domain(stage).try_into_domain()?,
                stage
            );
        }
        Ok(())
    }
}
