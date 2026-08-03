//! Rerank records of a search trace.
//!
//! One concept per module (R13): a rerank stage's model identity,
//! placement positions, and per-candidate scores live here, separate from
//! the raw candidates and lanes that precede reranking.

use serde::{Deserialize, Serialize};

use crate::ids::EvidenceId;
use crate::search::RetrievalModelFingerprint;

/// Final placement of one candidate after a rerank stage.
///
/// The new rank exists only on [`RerankPosition::Reranked`]; skipped and
/// failed candidates carry no rank, so "promoted" and "retained with a new
/// rank" can never disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RerankPosition {
    /// Candidate promoted by the reranker; carries its new rank.
    Reranked(usize),
    /// Candidate skipped because a cap was reached.
    SkippedCap,
    /// Rerank stage does not apply to this candidate.
    SkippedNotApplicable,
    /// Scorer failed; the candidate is retained through the fallback path.
    ErrorFallback(String),
}

/// One named constraint score produced by the reranker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceConstraintScore {
    pub name: String,
    pub score: u32,
}

/// One candidate's placement after a rerank stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRerankCandidate {
    pub candidate_id: EvidenceId,
    pub original_rank: usize,
    pub position: RerankPosition,
    pub relevance_score: Option<u32>,
    pub constraint_scores: Vec<SearchTraceConstraintScore>,
}

/// The rerank stage recorded in a trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRerank {
    pub model: String,
    pub fingerprint: RetrievalModelFingerprint,
    pub input_cap: usize,
    pub score_cap: usize,
    pub output_cap: usize,
    pub candidates: Vec<SearchTraceRerankCandidate>,
}
