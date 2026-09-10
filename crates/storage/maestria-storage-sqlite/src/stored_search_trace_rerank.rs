//! Rerank-stage wire mirrors for the stored search trace
//! (`StoredSearchTraceRerank`, its candidates and constraint scores, plus
//! `StoredRerankPosition`). Re-exported by
//! `crate::payloads::stored_search_trace` so consumers keep a single import path.

use maestria_domain::{
    EvidenceId, LateInteractionProvenance, RerankPosition, SearchTraceConstraintScore,
    SearchTraceRerank, SearchTraceRerankCandidate,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    late_interaction: Option<LateInteractionProvenance>,
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
            late_interaction: value.late_interaction.clone(),
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
            late_interaction: self.late_interaction,
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
    use maestria_domain::{
        ContentHash, CorpusSnapshotId, IndexGenerationId, LateInteractionAggregation,
        LateInteractionProvenance, LateInteractionTokenContribution, RepresentationName,
        RerankPosition, RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank,
        RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale,
    };

    use super::*;

    fn hash(letter: char) -> Result<ContentHash, Box<dyn std::error::Error>> {
        Ok(ContentHash::new(format!(
            "sha256:{}",
            letter.to_string().repeat(64)
        ))?)
    }

    fn provenance() -> Result<LateInteractionProvenance, Box<dyn std::error::Error>> {
        let identity = RetrievalModelFingerprint::new(format!("sha256:{}", "a".repeat(64)))?;
        let mut components = std::collections::BTreeMap::new();
        components.insert("aggregation".into(), "QueryTokenMaxThenSum".into());
        components.insert("normalization".into(), "L2PerTokenV1".into());
        components.insert("provider".into(), "mlateon-onnx".into());
        components.insert("model".into(), "mlateon".into());
        components.insert("representation".into(), "multivector_text_v1".into());
        components.insert("generation_id".into(), "1".into());
        components.insert("corpus_snapshot".into(), "1".into());
        components.insert("identity_digest".into(), identity.as_str().into());
        Ok(LateInteractionProvenance {
            score: RetrievalLaneScore::new(
                RetrievalScoreKind::LateInteraction,
                -1_000_000,
                RetrievalRawRank::ranked(8),
                RetrievalScoreScale::fixed_point("late_interaction_maxsim_micros_v1", 1_000_000),
                RepresentationName::new("multivector_text_v1"),
                RetrievalScoreFingerprint {
                    identity,
                    components,
                },
            ),
            generation_id: IndexGenerationId::new(1),
            corpus_snapshot: CorpusSnapshotId::new(1),
            namespace: "multivector_text_v1".into(),
            source_snapshot_hash: hash('b')?,
            source_representation_hash: hash('c')?,
            query_hash: hash('d')?,
            aggregation: LateInteractionAggregation::QueryTokenMaxThenSum,
            query_token_count: 1,
            document_token_count: 1,
            query_truncated: false,
            document_truncated: false,
            contributions: vec![LateInteractionTokenContribution {
                query_position: 0,
                document_position: 0,
                similarity_micros: -1_000_000,
            }],
            omitted_contribution_count: 0,
            omitted_contribution_sum_micros: 0,
        })
    }

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

    #[test]
    fn late_provenance_round_trips_through_stored_candidate()
    -> Result<(), Box<dyn std::error::Error>> {
        let value = SearchTraceRerankCandidate {
            candidate_id: maestria_domain::EvidenceId::new(1),
            original_rank: 7,
            position: RerankPosition::Reranked(0),
            relevance_score: None,
            constraint_scores: Vec::new(),
            late_interaction: Some(provenance()?),
        };
        let stored = StoredSearchTraceRerankCandidate::from_domain(&value);
        assert_eq!(stored.try_into_domain()?, value);
        Ok(())
    }
}
