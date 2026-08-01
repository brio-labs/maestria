//! Per-lane execution mirrors for the stored search trace: the lane record
//! (`StoredSearchTraceLane`, `StoredSearchTraceLaneCandidate`,
//! `StoredSearchLaneStatus`) and the `SearchExecution` budget/usage/completion
//! tree (`StoredSearchExecutionBudget`, `StoredSearchExecutionUsage`,
//! `StoredSearchExecutionResource`, `StoredSearchExecutionCompletion`,
//! `StoredSearchExecution`). Re-exported by `crate::payloads::stored_search_trace`
//! so consumers keep a single import path.

use std::num::NonZeroU64;

use maestria_domain::{
    ArtifactVersionId, DuplicateClusterId, EvidenceId, IndexGenerationId, SearchExecution,
    SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionResource,
    SearchExecutionUsage, SearchLaneStatus, SearchTraceLane, SearchTraceLaneCandidate,
};
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search::{
    StoredEvidenceSpan, StoredRetrievalReason, StoredRetrievalScoreSet,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceLaneCandidate {
    evidence_id: u64,
    artifact_version: u64,
    source_span: StoredEvidenceSpan,
    lane_rank: u32,
    duplicate_cluster: Option<u64>,
    scores: StoredRetrievalScoreSet,
    reasons: Vec<StoredRetrievalReason>,
}

impl StoredSearchTraceLaneCandidate {
    pub(crate) fn from_domain(value: &SearchTraceLaneCandidate) -> Self {
        Self {
            evidence_id: value.evidence_id.value(),
            artifact_version: value.artifact_version.value(),
            source_span: StoredEvidenceSpan::from_domain(&value.source_span),
            lane_rank: value.lane_rank,
            duplicate_cluster: value.duplicate_cluster.map(|id| id.value()),
            scores: StoredRetrievalScoreSet::from_domain(&value.scores),
            reasons: value
                .reasons
                .iter()
                .map(StoredRetrievalReason::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchTraceLaneCandidate, maestria_ports::PortError> {
        Ok(SearchTraceLaneCandidate {
            evidence_id: EvidenceId::new(self.evidence_id),
            artifact_version: ArtifactVersionId::new(self.artifact_version),
            source_span: self.source_span.try_into_domain()?,
            lane_rank: self.lane_rank,
            duplicate_cluster: self.duplicate_cluster.map(DuplicateClusterId::new),
            scores: self.scores.try_into_domain()?,
            reasons: self
                .reasons
                .into_iter()
                .map(StoredRetrievalReason::try_into_domain)
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchLaneStatus {
    Succeeded,
    Empty,
    Failed { error: String },
}

impl StoredSearchLaneStatus {
    pub(crate) fn from_domain(value: &SearchLaneStatus) -> Self {
        match value {
            SearchLaneStatus::Succeeded => Self::Succeeded,
            SearchLaneStatus::Empty => Self::Empty,
            SearchLaneStatus::Failed { error } => Self::Failed {
                error: error.clone(),
            },
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchLaneStatus, maestria_ports::PortError> {
        Ok(match self {
            Self::Succeeded => SearchLaneStatus::Succeeded,
            Self::Empty => SearchLaneStatus::Empty,
            Self::Failed { error } => SearchLaneStatus::Failed { error },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceLane {
    retriever_id: String,
    query: String,
    generation: Option<u64>,
    status: StoredSearchLaneStatus,
    candidates: Vec<StoredSearchTraceLaneCandidate>,
    execution: StoredSearchExecution,
}

impl StoredSearchTraceLane {
    pub(crate) fn from_domain(value: &SearchTraceLane) -> Self {
        Self {
            retriever_id: value.retriever_id.clone(),
            query: value.query.clone(),
            generation: value.generation.map(|id| id.value()),
            status: StoredSearchLaneStatus::from_domain(&value.status),
            candidates: value
                .candidates
                .iter()
                .map(StoredSearchTraceLaneCandidate::from_domain)
                .collect(),
            execution: StoredSearchExecution::from_domain(&value.execution),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceLane, maestria_ports::PortError> {
        Ok(SearchTraceLane {
            retriever_id: self.retriever_id,
            query: self.query,
            generation: self.generation.map(IndexGenerationId::new),
            status: self.status.try_into_domain()?,
            candidates: self
                .candidates
                .into_iter()
                .map(StoredSearchTraceLaneCandidate::try_into_domain)
                .collect::<Result<_, _>>()?,
            execution: self.execution.try_into_domain()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchExecutionBudget {
    max_results: u64,
    max_candidates: u64,
    max_work_units: u64,
    max_bytes_read: Option<u64>,
}

impl StoredSearchExecutionBudget {
    pub(crate) fn from_domain(value: &SearchExecutionBudget) -> Self {
        Self {
            max_results: value.max_results(),
            max_candidates: value.max_candidates(),
            max_work_units: value.max_work_units(),
            max_bytes_read: value.max_bytes_read().map(|limit| limit.get()),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchExecutionBudget, maestria_ports::PortError> {
        SearchExecutionBudget::with_byte_limit(
            self.max_results,
            self.max_candidates,
            self.max_work_units,
            self.max_bytes_read.and_then(NonZeroU64::new),
        )
        .map_err(|error| maestria_ports::PortError::InvalidInputContext {
            context: "stored search execution budget is invalid",
            source: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchExecutionUsage {
    results: u64,
    candidates: u64,
    work_units: u64,
    bytes_read: u64,
}

impl StoredSearchExecutionUsage {
    pub(crate) fn from_domain(value: &SearchExecutionUsage) -> Self {
        Self {
            results: value.results,
            candidates: value.candidates,
            work_units: value.work_units,
            bytes_read: value.bytes_read,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchExecutionUsage, maestria_ports::PortError> {
        Ok(SearchExecutionUsage::new(
            self.results,
            self.candidates,
            self.work_units,
            self.bytes_read,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchExecutionResource {
    Results,
    Candidates,
    WorkUnits,
    BytesRead,
}

impl StoredSearchExecutionResource {
    pub(crate) fn from_domain(value: &SearchExecutionResource) -> Self {
        match value {
            SearchExecutionResource::Results => Self::Results,
            SearchExecutionResource::Candidates => Self::Candidates,
            SearchExecutionResource::WorkUnits => Self::WorkUnits,
            SearchExecutionResource::BytesRead => Self::BytesRead,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchExecutionResource, maestria_ports::PortError> {
        Ok(match self {
            Self::Results => SearchExecutionResource::Results,
            Self::Candidates => SearchExecutionResource::Candidates,
            Self::WorkUnits => SearchExecutionResource::WorkUnits,
            Self::BytesRead => SearchExecutionResource::BytesRead,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredSearchExecutionCompletion {
    Complete,
    Exhausted(StoredSearchExecutionResource),
}

impl StoredSearchExecutionCompletion {
    pub(crate) fn from_domain(value: &SearchExecutionCompletion) -> Self {
        match value {
            SearchExecutionCompletion::Complete => Self::Complete,
            SearchExecutionCompletion::Exhausted(resource) => {
                Self::Exhausted(StoredSearchExecutionResource::from_domain(resource))
            }
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchExecutionCompletion, maestria_ports::PortError> {
        Ok(match self {
            Self::Complete => SearchExecutionCompletion::Complete,
            Self::Exhausted(resource) => {
                SearchExecutionCompletion::Exhausted(resource.try_into_domain()?)
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchExecution {
    budget: StoredSearchExecutionBudget,
    usage: StoredSearchExecutionUsage,
    completion: StoredSearchExecutionCompletion,
}

impl StoredSearchExecution {
    pub(crate) fn from_domain(value: &SearchExecution) -> Self {
        Self {
            budget: StoredSearchExecutionBudget::from_domain(&value.budget),
            usage: StoredSearchExecutionUsage::from_domain(&value.usage),
            completion: StoredSearchExecutionCompletion::from_domain(&value.completion),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchExecution, maestria_ports::PortError> {
        Ok(SearchExecution::new(
            self.budget.try_into_domain()?,
            self.usage.try_into_domain()?,
            self.completion.try_into_domain()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::{SearchExecutionCompletion, SearchExecutionResource, SearchLaneStatus};

    use super::*;
    use crate::payloads::stored_search_trace::StoredSearchTrace;
    use crate::payloads::stored_search_trace::stored_search_trace_tests::sample_trace;

    #[test]
    fn execution_budget_rejects_zero_limits() {
        let stored = StoredSearchExecutionBudget {
            max_results: 0,
            max_candidates: 10,
            max_work_units: 10,
            max_bytes_read: None,
        };
        let result = stored.try_into_domain();
        assert!(matches!(
            result,
            Err(maestria_ports::PortError::InvalidInputContext {
                context: "stored search execution budget is invalid",
                ..
            })
        ));
    }

    #[test]
    fn trace_rejects_lane_with_invalid_execution_budget() -> Result<(), Box<dyn std::error::Error>>
    {
        let stored = StoredSearchTrace::from_domain(&sample_trace()?);
        let mut json = serde_json::to_value(&stored)?;
        json["lanes"][0]["execution"]["budget"]["max_results"] = 0.into();
        let stored: StoredSearchTrace = serde_json::from_value(json)?;
        assert!(stored.try_into_domain().is_err());
        Ok(())
    }

    #[test]
    fn lane_enum_variants_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        for status in [
            SearchLaneStatus::Succeeded,
            SearchLaneStatus::Empty,
            SearchLaneStatus::Failed {
                error: "boom".to_owned(),
            },
        ] {
            assert_eq!(
                StoredSearchLaneStatus::from_domain(&status).try_into_domain()?,
                status
            );
        }
        for resource in [
            SearchExecutionResource::Results,
            SearchExecutionResource::Candidates,
            SearchExecutionResource::WorkUnits,
            SearchExecutionResource::BytesRead,
        ] {
            assert_eq!(
                StoredSearchExecutionResource::from_domain(&resource).try_into_domain()?,
                resource
            );
        }
        for completion in [
            SearchExecutionCompletion::Complete,
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::WorkUnits),
        ] {
            assert_eq!(
                StoredSearchExecutionCompletion::from_domain(&completion).try_into_domain()?,
                completion
            );
        }
        Ok(())
    }
}
