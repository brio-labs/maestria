//! DTO mirrors of the maestria-domain search *outcome* core.
//!
//! The stored row owns its own wire format: every `Stored*` type here is a
//! serde shape independent of `maestria_domain`, with infallible
//! `from_domain` encoding and validated, fallible `try_into_domain` decoding.
//! No legacy wire shapes are preserved. The outcome-side types are
//! re-exported from `crate::payloads::stored_search` alongside the plan-side
//! types in [`crate::payloads::stored_search_plan`].
//!
//! This module is a facade: [`StoredSearchStatus`] and [`StoredSearchOutcome`]
//! live here, while the evidence-candidate and retrieval-score DTOs live in
//! `crate::payloads::stored_search_candidate` and
//! `crate::payloads::stored_search_scores`. Every moved type is re-exported
//! below so existing `crate::payloads::stored_search_outcome::*` import paths
//! keep working unchanged.

use maestria_domain::{IndexGenerationId, SearchOutcome, SearchStatus, SearchTraceId};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search_plan::StoredRetrievalModelFingerprint;

pub(crate) use crate::payloads::stored_search_candidate::*;
pub(crate) use crate::payloads::stored_search_scores::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchStatus {
    Answerable,
    AnswerableWithWarnings,
    EvidenceIncomplete,
    SourcesConflict,
    StaleEvidenceOnly,
    NoEvidenceFound,
    Abstained,
    DeniedByPolicy,
    QuarantinedForReview,
}

impl StoredSearchStatus {
    pub(crate) fn from_domain(value: &SearchStatus) -> Self {
        match value {
            SearchStatus::Answerable => Self::Answerable,
            SearchStatus::AnswerableWithWarnings => Self::AnswerableWithWarnings,
            SearchStatus::EvidenceIncomplete => Self::EvidenceIncomplete,
            SearchStatus::SourcesConflict => Self::SourcesConflict,
            SearchStatus::StaleEvidenceOnly => Self::StaleEvidenceOnly,
            SearchStatus::NoEvidenceFound => Self::NoEvidenceFound,
            SearchStatus::Abstained => Self::Abstained,
            SearchStatus::DeniedByPolicy => Self::DeniedByPolicy,
            SearchStatus::QuarantinedForReview => Self::QuarantinedForReview,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchStatus, PortError> {
        Ok(match self {
            Self::Answerable => SearchStatus::Answerable,
            Self::AnswerableWithWarnings => SearchStatus::AnswerableWithWarnings,
            Self::EvidenceIncomplete => SearchStatus::EvidenceIncomplete,
            Self::SourcesConflict => SearchStatus::SourcesConflict,
            Self::StaleEvidenceOnly => SearchStatus::StaleEvidenceOnly,
            Self::NoEvidenceFound => SearchStatus::NoEvidenceFound,
            Self::Abstained => SearchStatus::Abstained,
            Self::DeniedByPolicy => SearchStatus::DeniedByPolicy,
            Self::QuarantinedForReview => SearchStatus::QuarantinedForReview,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchOutcome {
    pub(crate) trace: u64,
    pub(crate) trace_data: Option<Box<crate::payloads::stored_search_trace::StoredSearchTrace>>,
    pub(crate) fingerprint: StoredRetrievalModelFingerprint,
    pub(crate) index_generation: u64,
    pub(crate) status: StoredSearchStatus,
    pub(crate) evidence: Vec<StoredEvidenceCandidate>,
    pub(crate) coverage: StoredEvidenceCoverage,
    pub(crate) conflicts: Vec<StoredConflictSet>,
}

impl StoredSearchOutcome {
    pub(crate) fn from_domain(value: &SearchOutcome) -> Self {
        Self {
            trace: value.trace.value(),
            trace_data: value.trace_data.as_ref().map(|trace| {
                Box::new(
                    crate::payloads::stored_search_trace::StoredSearchTrace::from_domain(trace),
                )
            }),
            fingerprint: StoredRetrievalModelFingerprint::from_domain(&value.fingerprint),
            index_generation: value.index_generation.value(),
            status: StoredSearchStatus::from_domain(&value.status),
            evidence: value
                .evidence
                .iter()
                .map(StoredEvidenceCandidate::from_domain)
                .collect(),
            coverage: StoredEvidenceCoverage::from_domain(&value.coverage),
            conflicts: value
                .conflicts
                .iter()
                .map(StoredConflictSet::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchOutcome, PortError> {
        let mut outcome = SearchOutcome {
            trace: SearchTraceId::new(self.trace),
            trace_data: self
                .trace_data
                .map(|trace| trace.try_into_domain().map(Box::new))
                .transpose()?,
            fingerprint: self.fingerprint.try_into_domain()?,
            index_generation: IndexGenerationId::new(self.index_generation),
            status: self.status.try_into_domain()?,
            evidence: self
                .evidence
                .into_iter()
                .map(StoredEvidenceCandidate::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
            coverage: self.coverage.try_into_domain()?,
            conflicts: self
                .conflicts
                .into_iter()
                .map(StoredConflictSet::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
        };
        outcome.canonicalize_score_provenance().map_err(|error| {
            PortError::InvalidInputContext {
                context: "decode stored search outcome",
                source: error.to_string(),
            }
        })?;
        Ok(outcome)
    }
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use maestria_domain::{
        ConflictSet, ConflictSetId, ContentRange, DuplicateClusterId, EvidenceCandidate,
        EvidenceCandidateDto, EvidenceCoverage, EvidenceCoverageDto, EvidenceSpan, FreshnessStatus,
        IndexGenerationId, LearnedSparseContribution, LearnedSparseReason, RepresentationName,
        RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank, RetrievalReason,
        RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet,
        SearchOutcome, SearchStatus, SearchTraceId, SourceLocation, StructureNodeId, TrustLabel,
    };

    use super::*;

    fn sample_candidate() -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
        let lane = RetrievalLaneScore::new(
            RetrievalScoreKind::Exact,
            1,
            RetrievalRawRank::ranked(1),
            RetrievalScoreScale::Binary,
            RepresentationName::new("text/plain"),
            RetrievalScoreFingerprint::new(
                RetrievalModelFingerprint::new("fp-v1".to_string())?,
                BTreeMap::from([("model".to_string(), "exact".to_string())]),
            ),
        );
        Ok(EvidenceCandidate::new(EvidenceCandidateDto {
            evidence_id: maestria_domain::EvidenceId::new(41),
            artifact_version: maestria_domain::ArtifactVersionId::new(42),
            source_span: EvidenceSpan::new(
                Some(StructureNodeId::new(3)),
                SourceLocation::file("/repo/src/lib.rs".to_string(), 10, 20)?,
                ContentRange::new(100, 250)?,
            )?,
            scores: RetrievalScoreSet::new(vec![lane])?,
            trust: TrustLabel::Verified,
            freshness: FreshnessStatus::UpToDate,
            duplicate_cluster: Some(DuplicateClusterId::new(11)),
            reasons: vec![
                RetrievalReason::ExactMatch,
                RetrievalReason::LearnedSparse(Box::new(LearnedSparseReason::new(vec![
                    LearnedSparseContribution {
                        term_id: 5,
                        contribution_micros: 42,
                    },
                ]))),
            ],
            coverage_keys: vec!["doc:7".to_string()],
        })?)
    }

    fn sample_outcome() -> Result<SearchOutcome, Box<dyn std::error::Error>> {
        Ok(SearchOutcome {
            trace: SearchTraceId::new(9),
            trace_data: None,
            fingerprint: RetrievalModelFingerprint::new("model-v1".to_string())?,
            index_generation: IndexGenerationId::new(3),
            status: SearchStatus::AnswerableWithWarnings,
            evidence: vec![sample_candidate()?],
            coverage: EvidenceCoverage::new(EvidenceCoverageDto {
                percent_covered: 80,
                gaps_identified: vec!["gap-a".to_string()],
                required_claims: Vec::new(),
                required_subquestions: Vec::new(),
                distinct_sources: 2,
                distinct_documents: 1,
                distinct_sections: 3,
                candidate_coverage_keys: Vec::new(),
            })?,
            conflicts: vec![ConflictSet {
                id: ConflictSetId::new(4),
                candidates: vec![sample_candidate()?],
            }],
        })
    }

    #[test]
    fn outcome_round_trips_without_trace_data() -> Result<(), Box<dyn std::error::Error>> {
        let outcome = sample_outcome()?;
        let stored = StoredSearchOutcome::from_domain(&outcome);
        assert_eq!(stored.try_into_domain()?, outcome);
        Ok(())
    }
}
