//! DTO mirrors of the maestria-domain search plan/outcome core.
//!
//! The stored row owns its own wire format: every `Stored*` type here is a
//! serde shape independent of `maestria_domain`, with infallible
//! `from_domain` encoding and validated, fallible `try_into_domain` decoding.
//! No legacy wire shapes are preserved.
//!
//! This module is a façade: the plan-side DTOs live in
//! `crate::payloads::stored_search_plan` and the outcome-side DTOs in
//! `crate::payloads::stored_search_outcome`. Every type is re-exported here
//! so existing `crate::payloads::stored_search::*` import paths keep working
//! unchanged.

pub(crate) use crate::payloads::stored_search_outcome::{
    StoredConflictSet, StoredEvidenceSpan, StoredFreshnessStatus, StoredRetrievalReason,
    StoredRetrievalScoreSet, StoredSearchOutcome, StoredTrustLabel,
};
pub(crate) use crate::payloads::stored_search_plan::{
    StoredCorpusScope, StoredEvidenceRequirements, StoredFreshnessRequirement, StoredModalitySet,
    StoredRetrievalModelFingerprint, StoredSearchBudget, StoredSearchIntent, StoredSearchPlan,
    StoredSearchStage, StoredStopConditions,
};
