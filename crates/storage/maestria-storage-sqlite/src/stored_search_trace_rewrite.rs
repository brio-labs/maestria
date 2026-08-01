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
            SearchRewriteOrigin::MissingSlot => Self::MissingSlot,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchRewriteOrigin, maestria_ports::PortError> {
        Ok(match self {
            Self::Original => SearchRewriteOrigin::Original,
            Self::Deterministic => SearchRewriteOrigin::Deterministic,
            Self::ModelProposal => SearchRewriteOrigin::ModelProposal,
            Self::Feedback => SearchRewriteOrigin::Feedback,
            Self::MissingSlot => SearchRewriteOrigin::MissingSlot,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchRewriteStage {
    InitialRetrieval,
    Reranking,
    IterativeRetrieval,
}

impl StoredSearchRewriteStage {
    pub(crate) fn from_domain(value: &SearchRewriteStage) -> Self {
        match value {
            SearchRewriteStage::InitialRetrieval => Self::InitialRetrieval,
            SearchRewriteStage::Reranking => Self::Reranking,
            SearchRewriteStage::IterativeRetrieval => Self::IterativeRetrieval,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchRewriteStage, maestria_ports::PortError> {
        Ok(match self {
            Self::InitialRetrieval => SearchRewriteStage::InitialRetrieval,
            Self::Reranking => SearchRewriteStage::Reranking,
            Self::IterativeRetrieval => SearchRewriteStage::IterativeRetrieval,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchRewriteAccounting {
    token_estimate: u32,
    latency_budget_units: u32,
    is_proposal: bool,
}

impl StoredSearchRewriteAccounting {
    pub(crate) fn from_domain(value: &SearchRewriteAccounting) -> Self {
        Self {
            token_estimate: value.token_estimate,
            latency_budget_units: value.latency_budget_units,
            is_proposal: value.is_proposal,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchRewriteAccounting, maestria_ports::PortError> {
        Ok(SearchRewriteAccounting {
            token_estimate: self.token_estimate,
            latency_budget_units: self.latency_budget_units,
            is_proposal: self.is_proposal,
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
        Self {
            query: value.query.clone(),
            origin: StoredSearchRewriteOrigin::from_domain(&value.origin),
            stage: StoredSearchRewriteStage::from_domain(&value.stage),
            accounting: StoredSearchRewriteAccounting::from_domain(&value.accounting),
            missing_slot: value.missing_slot.clone(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceRewrite, maestria_ports::PortError> {
        Ok(SearchTraceRewrite {
            query: self.query,
            origin: self.origin.try_into_domain()?,
            stage: self.stage.try_into_domain()?,
            accounting: self.accounting.try_into_domain()?,
            missing_slot: self.missing_slot,
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
            SearchRewriteOrigin::MissingSlot,
        ] {
            assert_eq!(
                StoredSearchRewriteOrigin::from_domain(&origin).try_into_domain()?,
                origin
            );
        }
        for stage in [
            SearchRewriteStage::InitialRetrieval,
            SearchRewriteStage::Reranking,
            SearchRewriteStage::IterativeRetrieval,
        ] {
            assert_eq!(
                StoredSearchRewriteStage::from_domain(&stage).try_into_domain()?,
                stage
            );
        }
        Ok(())
    }
}
