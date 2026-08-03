//! DTO mirrors of the maestria-domain search *evidence candidate* types.
//!
//! The stored row owns its own wire format: every `Stored*` type here is a
//! serde shape independent of `maestria_domain`, with infallible
//! `from_domain` encoding and validated, fallible `try_into_domain` decoding.
//! No legacy wire shapes are preserved. These types are re-exported from
//! `crate::payloads::stored_search_outcome` (the outcome facade) and from
//! `crate::payloads::stored_search`.

use maestria_domain::{
    ConflictSet, ConflictSetId, ContentRange, DuplicateClusterId, EvidenceCandidate,
    EvidenceCandidateDto, EvidenceCoverage, EvidenceCoverageDto, EvidenceSpan, FreshnessStatus,
    SourceLocation, StructureNodeId, TrustLabel,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search_scores::{StoredRetrievalReason, StoredRetrievalScoreSet};

fn span_decode_error(error: impl std::fmt::Display) -> PortError {
    PortError::InvalidInputContext {
        context: "decode stored evidence span",
        source: error.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSourceLocation {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
    },
    Page {
        page_start: u32,
        page_end: u32,
    },
    Region {
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Symbol {
        path: String,
        qualified_name: String,
    },
}

impl StoredSourceLocation {
    pub(crate) fn from_domain(value: &SourceLocation) -> Self {
        match value {
            SourceLocation::File {
                path,
                start_line,
                end_line,
            } => Self::File {
                path: path.clone(),
                start_line: *start_line,
                end_line: *end_line,
            },
            SourceLocation::Page {
                page_start,
                page_end,
            } => Self::Page {
                page_start: *page_start,
                page_end: *page_end,
            },
            SourceLocation::Region {
                page,
                x,
                y,
                width,
                height,
            } => Self::Region {
                page: *page,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            },
            SourceLocation::Symbol {
                path,
                qualified_name,
            } => Self::Symbol {
                path: path.clone(),
                qualified_name: qualified_name.clone(),
            },
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SourceLocation, PortError> {
        match self {
            Self::File {
                path,
                start_line,
                end_line,
            } => SourceLocation::file(path, start_line, end_line).map_err(span_decode_error),
            Self::Page {
                page_start,
                page_end,
            } => SourceLocation::page(page_start, page_end).map_err(span_decode_error),
            Self::Region {
                page,
                x,
                y,
                width,
                height,
            } => SourceLocation::region(page, x, y, width, height).map_err(span_decode_error),
            Self::Symbol {
                path,
                qualified_name,
            } => SourceLocation::symbol(path, qualified_name).map_err(span_decode_error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredContentRange {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl StoredContentRange {
    pub(crate) fn from_domain(value: &ContentRange) -> Self {
        Self {
            start: value.start(),
            end: value.end(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<ContentRange, PortError> {
        ContentRange::new(self.start, self.end).map_err(span_decode_error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidenceSpan {
    pub(crate) node_id: Option<u64>,
    pub(crate) location: StoredSourceLocation,
    pub(crate) range: StoredContentRange,
}

impl StoredEvidenceSpan {
    pub(crate) fn from_domain(value: &EvidenceSpan) -> Self {
        Self {
            node_id: value.node_id().map(|id| id.value()),
            location: StoredSourceLocation::from_domain(value.location()),
            range: StoredContentRange::from_domain(&value.range()),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<EvidenceSpan, PortError> {
        EvidenceSpan::new(
            self.node_id.map(StructureNodeId::new),
            self.location.try_into_domain()?,
            self.range.try_into_domain()?,
        )
        .map_err(|error| PortError::InvalidInputContext {
            context: "decode stored evidence span",
            source: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredTrustLabel {
    Verified,
    Unverified,
    Disputed,
    Deprecated,
}

impl StoredTrustLabel {
    pub(crate) fn from_domain(value: &TrustLabel) -> Self {
        match value {
            TrustLabel::Verified => Self::Verified,
            TrustLabel::Unverified => Self::Unverified,
            TrustLabel::Disputed => Self::Disputed,
            TrustLabel::Deprecated => Self::Deprecated,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<TrustLabel, PortError> {
        Ok(match self {
            Self::Verified => TrustLabel::Verified,
            Self::Unverified => TrustLabel::Unverified,
            Self::Disputed => TrustLabel::Disputed,
            Self::Deprecated => TrustLabel::Deprecated,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredFreshnessStatus {
    UpToDate,
    Stale,
    Unknown,
}

impl StoredFreshnessStatus {
    pub(crate) fn from_domain(value: &FreshnessStatus) -> Self {
        match value {
            FreshnessStatus::UpToDate => Self::UpToDate,
            FreshnessStatus::Stale => Self::Stale,
            FreshnessStatus::Unknown => Self::Unknown,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<FreshnessStatus, PortError> {
        Ok(match self {
            Self::UpToDate => FreshnessStatus::UpToDate,
            Self::Stale => FreshnessStatus::Stale,
            Self::Unknown => FreshnessStatus::Unknown,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidenceCandidate {
    pub(crate) evidence_id: u64,
    pub(crate) artifact_version: u64,
    pub(crate) source_span: StoredEvidenceSpan,
    pub(crate) scores: StoredRetrievalScoreSet,
    pub(crate) trust: StoredTrustLabel,
    pub(crate) freshness: StoredFreshnessStatus,
    pub(crate) duplicate_cluster: Option<u64>,
    pub(crate) reasons: Vec<StoredRetrievalReason>,
    pub(crate) coverage_keys: Vec<String>,
}

impl StoredEvidenceCandidate {
    pub(crate) fn from_domain(value: &EvidenceCandidate) -> Self {
        Self {
            evidence_id: value.evidence_id().value(),
            artifact_version: value.artifact_version().value(),
            source_span: StoredEvidenceSpan::from_domain(value.source_span()),
            scores: StoredRetrievalScoreSet::from_domain(value.scores()),
            trust: StoredTrustLabel::from_domain(&value.trust()),
            freshness: StoredFreshnessStatus::from_domain(&value.freshness()),
            duplicate_cluster: value.duplicate_cluster().map(|id| id.value()),
            reasons: value
                .reasons()
                .iter()
                .map(StoredRetrievalReason::from_domain)
                .collect(),
            coverage_keys: value.coverage_keys().to_vec(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<EvidenceCandidate, PortError> {
        EvidenceCandidate::new(EvidenceCandidateDto {
            evidence_id: maestria_domain::EvidenceId::new(self.evidence_id),
            artifact_version: maestria_domain::ArtifactVersionId::new(self.artifact_version),
            source_span: self.source_span.try_into_domain()?,
            scores: self.scores.try_into_domain()?,
            trust: self.trust.try_into_domain()?,
            freshness: self.freshness.try_into_domain()?,
            duplicate_cluster: self.duplicate_cluster.map(DuplicateClusterId::new),
            reasons: self
                .reasons
                .into_iter()
                .map(StoredRetrievalReason::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
            coverage_keys: self.coverage_keys,
        })
        .map_err(|error| PortError::InvalidInputContext {
            context: "decode stored evidence candidate",
            source: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredConflictSet {
    pub(crate) id: u64,
    pub(crate) candidates: Vec<StoredEvidenceCandidate>,
}

impl StoredConflictSet {
    pub(crate) fn from_domain(value: &ConflictSet) -> Self {
        Self {
            id: value.id.value(),
            candidates: value
                .candidates
                .iter()
                .map(StoredEvidenceCandidate::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<ConflictSet, PortError> {
        Ok(ConflictSet {
            id: ConflictSetId::new(self.id),
            candidates: self
                .candidates
                .into_iter()
                .map(StoredEvidenceCandidate::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidenceCoverage {
    pub(crate) percent_covered: u8,
    pub(crate) gaps_identified: Vec<String>,
    pub(crate) required_claims: Vec<String>,
    pub(crate) required_subquestions: Vec<String>,
    pub(crate) distinct_sources: usize,
    pub(crate) distinct_documents: usize,
    pub(crate) distinct_sections: usize,
    pub(crate) candidate_coverage_keys: Vec<String>,
}

impl StoredEvidenceCoverage {
    pub(crate) fn from_domain(value: &EvidenceCoverage) -> Self {
        Self {
            percent_covered: value.percent_covered(),
            gaps_identified: value.gaps_identified().to_vec(),
            required_claims: value.required_claims().to_vec(),
            required_subquestions: value.required_subquestions().to_vec(),
            distinct_sources: value.distinct_sources(),
            distinct_documents: value.distinct_documents(),
            distinct_sections: value.distinct_sections(),
            candidate_coverage_keys: value.candidate_coverage_keys().to_vec(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<EvidenceCoverage, PortError> {
        if self.percent_covered > 100 {
            return Err(PortError::InvalidInputContext {
                context: "decode stored evidence coverage",
                source: "percent_covered must be between 0 and 100".to_string(),
            });
        }
        EvidenceCoverage::new(EvidenceCoverageDto {
            percent_covered: self.percent_covered,
            gaps_identified: self.gaps_identified,
            required_claims: self.required_claims,
            required_subquestions: self.required_subquestions,
            distinct_sources: self.distinct_sources,
            distinct_documents: self.distinct_documents,
            distinct_sections: self.distinct_sections,
            candidate_coverage_keys: self.candidate_coverage_keys,
        })
        .map_err(|error| PortError::InvalidInputContext {
            context: "decode stored evidence coverage",
            source: error.to_string(),
        })
    }
}
#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use maestria_domain::{
        ContentRange, DuplicateClusterId, EvidenceCandidate, EvidenceCandidateDto, EvidenceSpan,
        FreshnessStatus, LearnedSparseContribution, LearnedSparseReason, RepresentationName,
        RetrievalLaneScore, RetrievalModelFingerprint, RetrievalRawRank, RetrievalReason,
        RetrievalScoreFingerprint, RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet,
        SourceLocation, StructureNodeId, TrustLabel,
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

    #[test]
    fn candidate_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let candidate = sample_candidate()?;
        let stored = StoredEvidenceCandidate::from_domain(&candidate);
        assert_eq!(stored.try_into_domain()?, candidate);
        Ok(())
    }
}
