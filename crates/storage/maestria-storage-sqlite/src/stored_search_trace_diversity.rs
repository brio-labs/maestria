//! Diversity-stage wire mirrors for the stored search trace
//! (`StoredSearchTraceDiversity` and its candidate records). Re-exported by
//! `crate::payloads::stored_search_trace` so consumers keep a single import path.

use maestria_domain::{
    DiversityPlacement, DiversitySkipReason, DuplicateClusterId, EvidenceId, SearchTraceDiversity,
    SearchTraceDiversityCandidate,
};
use serde::{Deserialize, Serialize};

use crate::payloads::stored_search_trace::StoredSearchStopReason;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredDiversityPlacement {
    Selected(usize),
    Skipped(StoredDiversitySkipReason),
}

crate::stored_enum! {
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StoredDiversitySkipReason <=> DiversitySkipReason {
        DuplicateCluster,
        LowMarginalGain,
    }
}
impl StoredDiversityPlacement {
    pub(crate) fn from_domain(value: &DiversityPlacement) -> Self {
        match value {
            DiversityPlacement::Selected(rank) => Self::Selected(*rank),
            DiversityPlacement::Skipped(reason) => {
                Self::Skipped(StoredDiversitySkipReason::from_domain(*reason))
            }
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<DiversityPlacement, maestria_ports::PortError> {
        Ok(match self {
            Self::Selected(rank) => DiversityPlacement::Selected(rank),
            Self::Skipped(reason) => DiversityPlacement::Skipped(reason.try_into_domain()?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceDiversityCandidate {
    candidate_id: u64,
    original_rank: usize,
    placement: StoredDiversityPlacement,
    duplicate_cluster: Option<u64>,
    marginal_coverage: u8,
    coverage_keys: Vec<String>,
}

impl StoredSearchTraceDiversityCandidate {
    pub(crate) fn from_domain(value: &SearchTraceDiversityCandidate) -> Self {
        Self {
            candidate_id: value.candidate_id.value(),
            original_rank: value.original_rank,
            placement: StoredDiversityPlacement::from_domain(&value.placement),
            duplicate_cluster: value.duplicate_cluster.map(|id| id.value()),
            marginal_coverage: value.marginal_coverage,
            coverage_keys: value.coverage_keys.clone(),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SearchTraceDiversityCandidate, maestria_ports::PortError> {
        Ok(SearchTraceDiversityCandidate {
            candidate_id: EvidenceId::new(self.candidate_id),
            original_rank: self.original_rank,
            placement: self.placement.try_into_domain()?,
            duplicate_cluster: self.duplicate_cluster.map(DuplicateClusterId::new),
            marginal_coverage: self.marginal_coverage,
            coverage_keys: self.coverage_keys,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSearchTraceDiversity {
    distinct_sources: usize,
    distinct_documents: usize,
    distinct_sections: usize,
    required_claims: Vec<String>,
    required_subquestions: Vec<String>,
    covered_keys: Vec<String>,
    stop_reason: StoredSearchStopReason,
    candidates: Vec<StoredSearchTraceDiversityCandidate>,
}

impl StoredSearchTraceDiversity {
    pub(crate) fn from_domain(value: &SearchTraceDiversity) -> Self {
        Self {
            distinct_sources: value.distinct_sources,
            distinct_documents: value.distinct_documents,
            distinct_sections: value.distinct_sections,
            required_claims: value.required_claims.clone(),
            required_subquestions: value.required_subquestions.clone(),
            covered_keys: value.covered_keys.clone(),
            stop_reason: StoredSearchStopReason::from_domain(&value.stop_reason),
            candidates: value
                .candidates
                .iter()
                .map(StoredSearchTraceDiversityCandidate::from_domain)
                .collect(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<SearchTraceDiversity, maestria_ports::PortError> {
        Ok(SearchTraceDiversity {
            distinct_sources: self.distinct_sources,
            distinct_documents: self.distinct_documents,
            distinct_sections: self.distinct_sections,
            required_claims: self.required_claims,
            required_subquestions: self.required_subquestions,
            covered_keys: self.covered_keys,
            stop_reason: self.stop_reason.try_into_domain()?,
            candidates: self
                .candidates
                .into_iter()
                .map(StoredSearchTraceDiversityCandidate::try_into_domain)
                .collect::<Result<_, _>>()?,
        })
    }
}
