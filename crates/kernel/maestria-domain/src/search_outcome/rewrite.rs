//! Rewrite and filter records of a search trace.
//!
//! One concept per module (R13): the rewrite pipeline's typed records
//! (origin, stage, accounting), the trace's filter set, and the terminal
//! stop reason all describe *how* a search reached its outcome, separate
//! from the candidates it produced.

use serde::{Deserialize, Serialize};

/// Why a query rewrite entered the trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchRewriteOrigin {
    Original,
    Deterministic,
    ModelProposal,
    Feedback,
    /// A rewrite that fills a declared missing-evidence slot; the slot
    /// identity lives on the variant so a missing-slot rewrite without a
    /// named slot is unrepresentable (R56).
    MissingSlot {
        slot: String,
    },
}

/// Which search stage a rewrite belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchRewriteStage {
    InitialRetrieval,
    Reranking,
    IterativeRetrieval,
}

/// Budget accounting for one rewrite record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRewriteAccounting {
    pub token_estimate: u32,
    pub latency_budget_units: u32,
}

/// One recorded query rewrite in a search trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRewrite {
    pub query: String,
    pub origin: SearchRewriteOrigin,
    pub stage: SearchRewriteStage,
    pub accounting: SearchRewriteAccounting,
}

/// Why the search stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStopReason {
    ResultsLimit,
    EvidenceComplete,
    RequirementsUnmet,
    NoEvidence,
    LowMarginalGain,
    BudgetExhausted,
    PolicyDenied,
    Abstained,
}

/// Which retrieval-lane checks filtered a candidate out of the trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchTraceFilter {
    Scope,
    Acl,
    Trust,
    Sensitivity,
    Quarantine,
    PromptInjection,
    Freshness,
}
