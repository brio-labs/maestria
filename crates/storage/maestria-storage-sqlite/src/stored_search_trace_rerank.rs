//! Rerank-stage wire mirrors for the stored search trace
//! (`StoredSearchTraceRerank`, its candidates and constraint scores, plus
//! `StoredRerankPosition`). Re-exported by
//! `crate::payloads::stored_search_trace` so consumers keep a single import path.

use maestria_domain::{
    EvidenceId, RerankPosition, SearchTraceConstraintScore, SearchTraceRerank,
    SearchTraceRerankCandidate,
};
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search::StoredRetrievalModelFingerprint;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRerankPosition {
    Reranked(usize),
    SkippedCap,
    SkippedNotApplicable,
    ErrorFallback(String),
}

impl StoredRerankPosition {
    pub(crate) fn from_domain(value: &RerankPosition) -> Self {
        match value {
            RerankPosition::Reranked(rank) => Self::Reranked(*rank),
            RerankPosition::SkippedCap => Self::SkippedCap,
            RerankPosition::SkippedNotApplicable => Self::SkippedNotApplicable,
            RerankPosition::ErrorFallback(message) => Self::ErrorFallback(message.clone()),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<RerankPosition, maestria_ports::PortError> {
        Ok(match self {
            Self::Reranked(rank) => RerankPosition::Reranked(rank),
            Self::SkippedCap => RerankPosition::SkippedCap,
            Self::SkippedNotApplicable => RerankPosition::SkippedNotApplicable,
            Self::ErrorFallback(message) => RerankPosition::ErrorFallback(message),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceConstraintScore {
    name: String,
    score: u32,
}

impl StoredSearchTraceConstraintScore {
    pub(crate) fn from_domain(value: &SearchTraceConstraintScore) -> Self {
        Self {
            name: value.name.clone(),
            score: value.score,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchTraceConstraintScore, maestria_ports::PortError> {
        Ok(SearchTraceConstraintScore {
            name: self.name,
            score: self.score,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceRerankCandidate {
    candidate_id: u64,
    original_rank: usize,
    position: StoredRerankPosition,
    relevance_score: Option<u32>,
    constraint_scores: Vec<StoredSearchTraceConstraintScore>,
}

impl StoredSearchTraceRerankCandidate {
    pub(crate) fn from_domain(value: &SearchTraceRerankCandidate) -> Self {
        Self {
            candidate_id: value.candidate_id.value(),
            original_rank: value.original_rank,
            position: StoredRerankPosition::from_domain(&value.position),
            relevance_score: value.relevance_score,
            constraint_scores: value
                .constraint_scores
                .iter()
                .map(StoredSearchTraceConstraintScore::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchTraceRerankCandidate, maestria_ports::PortError> {
        Ok(SearchTraceRerankCandidate {
            candidate_id: EvidenceId::new(self.candidate_id),
            original_rank: self.original_rank,
            position: self.position.try_into_domain()?,
            relevance_score: self.relevance_score,
            constraint_scores: self
                .constraint_scores
                .into_iter()
                .map(StoredSearchTraceConstraintScore::try_into_domain)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceRerank {
    model: String,
    fingerprint: StoredRetrievalModelFingerprint,
    input_cap: usize,
    score_cap: usize,
    output_cap: usize,
    candidates: Vec<StoredSearchTraceRerankCandidate>,
}

impl StoredSearchTraceRerank {
    pub(crate) fn from_domain(value: &SearchTraceRerank) -> Self {
        Self {
            model: value.model.clone(),
            fingerprint: StoredRetrievalModelFingerprint::from_domain(&value.fingerprint),
            input_cap: value.input_cap,
            score_cap: value.score_cap,
            output_cap: value.output_cap,
            candidates: value
                .candidates
                .iter()
                .map(StoredSearchTraceRerankCandidate::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceRerank, maestria_ports::PortError> {
        Ok(SearchTraceRerank {
            model: self.model,
            fingerprint: self.fingerprint.try_into_domain()?,
            input_cap: self.input_cap,
            score_cap: self.score_cap,
            output_cap: self.output_cap,
            candidates: self
                .candidates
                .into_iter()
                .map(StoredSearchTraceRerankCandidate::try_into_domain)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::RerankPosition;

    use super::*;

    #[test]
    fn rerank_position_variants_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        for position in [
            RerankPosition::Reranked(3),
            RerankPosition::SkippedCap,
            RerankPosition::SkippedNotApplicable,
            RerankPosition::ErrorFallback("boom".to_owned()),
        ] {
            assert_eq!(
                StoredRerankPosition::from_domain(&position).try_into_domain()?,
                position
            );
        }
        Ok(())
    }
}
