//! Rerank-stage wire mirrors for the stored search trace
//! (`StoredSearchTraceRerank`, its candidates and constraint scores, plus
//! `StoredRerankCandidateStatus`). Re-exported by
//! `crate::payloads::stored_search_trace` so consumers keep a single import path.

use maestria_domain::{
    EvidenceId, RerankCandidateStatus, SearchTraceConstraintScore, SearchTraceRerank,
    SearchTraceRerankCandidate,
};
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search::StoredRetrievalModelFingerprint;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRerankCandidateStatus {
    Reranked,
    SkippedCap,
    SkippedNotApplicable,
    ErrorFallback(String),
}

impl StoredRerankCandidateStatus {
    pub(crate) fn from_domain(value: &RerankCandidateStatus) -> Self {
        match value {
            RerankCandidateStatus::Reranked => Self::Reranked,
            RerankCandidateStatus::SkippedCap => Self::SkippedCap,
            RerankCandidateStatus::SkippedNotApplicable => Self::SkippedNotApplicable,
            RerankCandidateStatus::ErrorFallback(message) => Self::ErrorFallback(message.clone()),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<RerankCandidateStatus, maestria_ports::PortError> {
        Ok(match self {
            Self::Reranked => RerankCandidateStatus::Reranked,
            Self::SkippedCap => RerankCandidateStatus::SkippedCap,
            Self::SkippedNotApplicable => RerankCandidateStatus::SkippedNotApplicable,
            Self::ErrorFallback(message) => RerankCandidateStatus::ErrorFallback(message),
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
    new_rank: Option<usize>,
    status: StoredRerankCandidateStatus,
    relevance_score: Option<u32>,
    constraint_score: Option<u32>,
    constraint_scores: Vec<StoredSearchTraceConstraintScore>,
}

impl StoredSearchTraceRerankCandidate {
    pub(crate) fn from_domain(value: &SearchTraceRerankCandidate) -> Self {
        Self {
            candidate_id: value.candidate_id.value(),
            original_rank: value.original_rank,
            new_rank: value.new_rank,
            status: StoredRerankCandidateStatus::from_domain(&value.status),
            relevance_score: value.relevance_score,
            constraint_score: value.constraint_score,
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
            new_rank: self.new_rank,
            status: self.status.try_into_domain()?,
            relevance_score: self.relevance_score,
            constraint_score: self.constraint_score,
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
    use maestria_domain::RerankCandidateStatus;

    use super::*;

    #[test]
    fn rerank_status_variants_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        for status in [
            RerankCandidateStatus::Reranked,
            RerankCandidateStatus::SkippedCap,
            RerankCandidateStatus::SkippedNotApplicable,
            RerankCandidateStatus::ErrorFallback("boom".to_owned()),
        ] {
            assert_eq!(
                StoredRerankCandidateStatus::from_domain(&status).try_into_domain()?,
                status
            );
        }
        Ok(())
    }
}
