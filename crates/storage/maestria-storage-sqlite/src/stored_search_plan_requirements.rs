//! DTO mirrors of the maestria-domain search *plan* requirement types: stop
//! conditions and evidence requirements.
//!
//! Each `Stored*` type here is a serde shape independent of `maestria_domain`,
//! with infallible `from_domain` encoding and validated, fallible
//! `try_into_domain` decoding. The types are re-exported from
//! `crate::payloads::stored_search_plan` so existing import paths keep
//! working unchanged.

use maestria_domain::{EvidenceRequirements, StopConditions};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredStopConditions {
    pub(crate) max_results: u32,
    pub(crate) min_score_threshold: u32,
}

impl StoredStopConditions {
    pub(crate) fn from_domain(value: &StopConditions) -> Self {
        Self {
            max_results: value.max_results,
            min_score_threshold: value.min_score_threshold,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<StopConditions, PortError> {
        Ok(StopConditions {
            max_results: self.max_results,
            min_score_threshold: self.min_score_threshold,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidenceRequirements {
    pub(crate) require_primary_sources: bool,
    pub(crate) minimum_corroboration: u8,
    pub(crate) required_claims: Vec<String>,
    pub(crate) required_subquestions: Vec<String>,
    pub(crate) minimum_sources: usize,
    pub(crate) minimum_documents: usize,
    pub(crate) minimum_sections: usize,
}

impl StoredEvidenceRequirements {
    pub(crate) fn from_domain(value: &EvidenceRequirements) -> Self {
        Self {
            require_primary_sources: value.require_primary_sources,
            minimum_corroboration: value.minimum_corroboration,
            required_claims: value.required_claims.clone(),
            required_subquestions: value.required_subquestions.clone(),
            minimum_sources: value.minimum_sources,
            minimum_documents: value.minimum_documents,
            minimum_sections: value.minimum_sections,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<EvidenceRequirements, PortError> {
        Ok(EvidenceRequirements {
            require_primary_sources: self.require_primary_sources,
            minimum_corroboration: self.minimum_corroboration,
            required_claims: self.required_claims,
            required_subquestions: self.required_subquestions,
            minimum_sources: self.minimum_sources,
            minimum_documents: self.minimum_documents,
            minimum_sections: self.minimum_sections,
        })
    }
}
