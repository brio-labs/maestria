//! DTO mirrors of the maestria-domain search *plan* policy types: execution
//! budget, retrieval-model fingerprint, and the trusted request-bound
//! authorization snapshot a plan carries.
//!
//! Each `Stored*` type here is a serde shape independent of `maestria_domain`,
//! with infallible `from_domain` encoding and validated, fallible
//! `try_into_domain` decoding. The types are re-exported from
//! `crate::payloads::stored_search_plan` so existing import paths keep
//! working unchanged.

use maestria_domain::{
    RetrievalModelFingerprint, RetrievalPolicySnapshot, ScopeId, SearchBudget, SearchBudgetLimits,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

use crate::payloads::stored_security::{StoredSensitivity, StoredTrustZone};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchBudget {
    pub(crate) max_tokens: u32,
    pub(crate) max_latency_ms: u32,
    pub(crate) max_queries: u32,
    pub(crate) max_stages: u32,
    pub(crate) max_web_requests: u32,
    pub(crate) max_bytes_read: u64,
    pub(crate) max_concurrency: u32,
    pub(crate) max_candidates: u32,
    pub(crate) max_work_units: u64,
}

impl StoredSearchBudget {
    pub(crate) fn from_domain(value: &SearchBudget) -> Self {
        Self {
            max_tokens: value.max_tokens(),
            max_latency_ms: value.max_latency_ms(),
            max_queries: value.max_queries(),
            max_stages: value.max_stages(),
            max_web_requests: value.max_web_requests(),
            max_bytes_read: value.max_bytes_read(),
            max_concurrency: value.max_concurrency(),
            max_candidates: value.max_candidates(),
            max_work_units: value.max_work_units(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchBudget, PortError> {
        SearchBudget::with_execution_limits(SearchBudgetLimits {
            max_tokens: self.max_tokens,
            max_latency_ms: self.max_latency_ms,
            max_queries: self.max_queries,
            max_stages: self.max_stages,
            max_web_requests: self.max_web_requests,
            max_bytes_read: self.max_bytes_read,
            max_concurrency: self.max_concurrency,
            max_candidates: self.max_candidates,
            max_work_units: self.max_work_units,
        })
        .map_err(|error| PortError::InvalidInputContext {
            context: "decode stored search budget",
            source: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct StoredRetrievalModelFingerprint(String);

impl StoredRetrievalModelFingerprint {
    pub(crate) fn from_domain(value: &RetrievalModelFingerprint) -> Self {
        Self(value.as_str().to_owned())
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalModelFingerprint, PortError> {
        RetrievalModelFingerprint::new(self.0).map_err(|error| PortError::InvalidInputContext {
            context: "decode stored retrieval model fingerprint",
            source: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredRetrievalPolicySnapshot {
    pub(crate) require_trust_zone: Option<StoredTrustZone>,
    pub(crate) max_sensitivity: Option<StoredSensitivity>,
    pub(crate) require_read_allowed: bool,
    /// Complete effective scope set. `None` is global; `Some` is restricted.
    pub(crate) effective_scopes: Option<Vec<u64>>,
    pub(crate) allow_unscoped_items: bool,
}

impl StoredRetrievalPolicySnapshot {
    pub(crate) fn from_domain(value: &RetrievalPolicySnapshot) -> Self {
        Self {
            require_trust_zone: value
                .require_trust_zone
                .as_ref()
                .map(StoredTrustZone::from_domain),
            max_sensitivity: value
                .max_sensitivity
                .as_ref()
                .map(StoredSensitivity::from_domain),
            require_read_allowed: value.require_read_allowed,
            effective_scopes: value
                .effective_scopes
                .as_ref()
                .map(|scopes| scopes.iter().map(ScopeId::value).collect()),
            allow_unscoped_items: value.allow_unscoped_items,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RetrievalPolicySnapshot, PortError> {
        Ok(RetrievalPolicySnapshot {
            require_trust_zone: self
                .require_trust_zone
                .map(StoredTrustZone::try_into_domain)
                .transpose()?,
            max_sensitivity: self
                .max_sensitivity
                .map(StoredSensitivity::try_into_domain)
                .transpose()?,
            require_read_allowed: self.require_read_allowed,
            effective_scopes: self
                .effective_scopes
                .map(|scopes| scopes.into_iter().map(ScopeId::new).collect()),
            allow_unscoped_items: self.allow_unscoped_items,
        })
    }
}
