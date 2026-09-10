//! Rerank records of a search trace.
//!
//! One concept per module (R13): a rerank stage's model identity,
//! placement positions, and per-candidate scores live here, separate from
//! the raw candidates and lanes that precede reranking.

use serde::{Deserialize, Serialize};

use crate::ids::{CorpusSnapshotId, EvidenceId, IndexGenerationId};
use crate::search::{
    ContentHash, LateInteractionAggregation, RetrievalLaneScore, RetrievalModelFingerprint,
    RetrievalScoreKind, RetrievalScoreScale, SearchCompatibilityError,
};

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

/// One bounded signed token contribution retained in late-interaction trace
/// provenance. Tensors never enter the persisted trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionTokenContribution {
    pub query_position: u32,
    pub document_position: u32,
    pub similarity_micros: i64,
}

/// Explanatory provenance for one successful late-interaction candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateInteractionProvenance {
    pub score: RetrievalLaneScore,
    pub generation_id: IndexGenerationId,
    pub corpus_snapshot: CorpusSnapshotId,
    pub namespace: String,
    pub source_snapshot_hash: ContentHash,
    pub source_representation_hash: ContentHash,
    pub query_hash: ContentHash,
    pub aggregation: LateInteractionAggregation,
    pub query_token_count: u32,
    pub document_token_count: u32,
    pub query_truncated: bool,
    pub document_truncated: bool,
    pub contributions: Vec<LateInteractionTokenContribution>,
    pub omitted_contribution_count: u32,
    pub omitted_contribution_sum_micros: i64,
}

impl LateInteractionProvenance {
    pub fn validate(&self) -> Result<(), SearchCompatibilityError> {
        if self.score.score_kind != RetrievalScoreKind::LateInteraction
            || self.generation_id.value() == 0
            || self.corpus_snapshot.value() == 0
            || self.namespace != "multivector_text_v1"
            || self.aggregation != LateInteractionAggregation::QueryTokenMaxThenSum
            || self.namespace.chars().any(char::is_control)
            || self.query_token_count == 0
            || self.document_token_count == 0
            || self.contributions.len() > 16
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "invalid late-interaction provenance identity or bounds",
            ));
        }
        if !matches!(self.score.representation.as_str(), "multivector_text_v1")
            || !matches!(self.score.raw_rank, crate::RetrievalRawRank::Ranked { rank } if rank > 0)
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "late-interaction provenance has invalid representation or raw rank",
            ));
        }
        for key in [
            "aggregation",
            "normalization",
            "provider",
            "model",
            "representation",
            "generation_id",
            "corpus_snapshot",
            "identity_digest",
        ] {
            if self
                .score
                .fingerprint
                .components
                .get(key)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(SearchCompatibilityError::InvalidScoreProvenance(
                    "late-interaction provenance fingerprint is incomplete",
                ));
            }
        }
        let components = &self.score.fingerprint.components;
        if components.get("identity_digest")
            != Some(&self.score.fingerprint.identity.as_str().to_string())
            || components.get("representation") != Some(&self.namespace)
            || components.get("generation_id") != Some(&self.generation_id.value().to_string())
            || components.get("corpus_snapshot") != Some(&self.corpus_snapshot.value().to_string())
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "late-interaction provenance identity components disagree",
            ));
        }
        match &self.score.scale {
            RetrievalScoreScale::FixedPoint {
                name, denominator, ..
            } if name == "late_interaction_maxsim_micros_v1" && *denominator == 1_000_000 => {}
            _ => {
                return Err(SearchCompatibilityError::InvalidScoreProvenance(
                    "late-interaction provenance requires its fixed-point scale",
                ));
            }
        }
        validate_late_interaction_contributions(self)?;
        Ok(())
    }
}

fn validate_late_interaction_contributions(
    provenance: &LateInteractionProvenance,
) -> Result<(), SearchCompatibilityError> {
    if provenance.contributions.len() as u32 + provenance.omitted_contribution_count
        != provenance.query_token_count
    {
        return Err(SearchCompatibilityError::InvalidScoreProvenance(
            "late-interaction contribution counts do not reconstruct query count",
        ));
    }
    let mut previous: Option<&LateInteractionTokenContribution> = None;
    let mut retained_sum = 0_i64;
    for contribution in &provenance.contributions {
        if contribution.query_position >= provenance.query_token_count
            || contribution.document_position >= provenance.document_token_count
            || previous.is_some_and(|prior| {
                prior.query_position == contribution.query_position
                    || contribution_order(prior, contribution) == std::cmp::Ordering::Greater
            })
        {
            return Err(SearchCompatibilityError::InvalidScoreProvenance(
                "late-interaction contributions are invalid or unordered",
            ));
        }
        retained_sum = retained_sum
            .checked_add(contribution.similarity_micros)
            .ok_or(SearchCompatibilityError::InvalidScoreProvenance(
                "late-interaction contribution sum overflows",
            ))?;
        previous = Some(contribution);
    }
    let reconstructed = retained_sum
        .checked_add(provenance.omitted_contribution_sum_micros)
        .ok_or(SearchCompatibilityError::InvalidScoreProvenance(
            "late-interaction aggregate overflows",
        ))?;
    if reconstructed != provenance.score.raw_score {
        return Err(SearchCompatibilityError::InvalidScoreProvenance(
            "late-interaction aggregate does not match contributions",
        ));
    }
    Ok(())
}

fn contribution_order(
    left: &LateInteractionTokenContribution,
    right: &LateInteractionTokenContribution,
) -> std::cmp::Ordering {
    right
        .similarity_micros
        .unsigned_abs()
        .cmp(&left.similarity_micros.unsigned_abs())
        .then_with(|| left.query_position.cmp(&right.query_position))
        .then_with(|| left.document_position.cmp(&right.document_position))
}

/// One candidate's placement after a rerank stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTraceRerankCandidate {
    pub candidate_id: EvidenceId,
    pub original_rank: usize,
    pub position: RerankPosition,
    pub relevance_score: Option<u32>,
    pub constraint_scores: Vec<SearchTraceConstraintScore>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub late_interaction: Option<LateInteractionProvenance>,
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
