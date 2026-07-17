use std::collections::BTreeSet;

use maestria_domain::{
    Artifact, ArtifactId, Evidence, EvidenceCandidate, EvidenceId, SearchOutcome, SearchPlan,
    SearchTrace, SearchTraceFilter, SearchTraceLane,
};

#[derive(Debug, Clone)]
pub struct SearchValidationContext<'a> {
    pub outcome: &'a SearchOutcome,
    pub plan: Option<&'a SearchPlan>,
    pub trace: Option<&'a SearchTrace>,
    pub evidence_by_id: &'a std::collections::BTreeMap<EvidenceId, Evidence>,
    pub artifacts_by_id: &'a std::collections::BTreeMap<ArtifactId, Artifact>,
}

impl<'a> SearchValidationContext<'a> {
    pub fn candidate_for(&self, evidence_id: EvidenceId) -> Option<&'a EvidenceCandidate> {
        self.outcome
            .evidence
            .iter()
            .find(|candidate| candidate.evidence_id == evidence_id)
    }

    pub fn candidate_ids(&self) -> impl Iterator<Item = EvidenceId> + '_ {
        self.outcome
            .evidence
            .iter()
            .map(|candidate| candidate.evidence_id)
    }

    pub fn has_duplicate_candidates(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.outcome
            .evidence
            .iter()
            .any(|candidate| !seen.insert(candidate.evidence_id))
    }

    pub fn duplicate_clusters(&self) -> usize {
        let mut seen = BTreeSet::new();
        self.outcome
            .evidence
            .iter()
            .filter_map(|candidate| candidate.duplicate_cluster)
            .filter(|cluster_id| !seen.insert(*cluster_id))
            .count()
    }

    pub fn is_trace_filtered_by(&self, filter: SearchTraceFilter) -> bool {
        self.trace
            .is_some_and(|trace| trace.filters.contains(&filter))
    }

    pub fn lane_count(&self) -> usize {
        self.trace.map_or(0, |trace| trace.lanes.len())
    }

    pub fn top_candidate_ids(&self, count: usize) -> Vec<EvidenceId> {
        self.outcome
            .evidence
            .iter()
            .take(count)
            .map(|candidate| candidate.evidence_id)
            .collect()
    }

    pub fn evidence_record(&self, evidence_id: EvidenceId) -> Option<&'a Evidence> {
        self.evidence_by_id.get(&evidence_id)
    }

    pub fn artifact_record(&self, artifact_id: ArtifactId) -> Option<&'a Artifact> {
        self.artifacts_by_id.get(&artifact_id)
    }

    pub fn trace_has_rewrites(&self) -> bool {
        self.trace.is_some_and(|trace| !trace.rewrites.is_empty())
    }

    pub fn trace_has_diversity(&self) -> bool {
        self.trace.is_some_and(|trace| {
            trace
                .diversity
                .as_ref()
                .is_some_and(|d| !d.candidates.is_empty())
        })
    }

    pub fn total_lanes_candidates(&self) -> usize {
        self.trace.map_or(0, |trace| {
            trace
                .lanes
                .iter()
                .map(|lane: &SearchTraceLane| lane.candidates.len())
                .sum()
        })
    }

    pub fn status_requires_evidence(&self) -> bool {
        !matches!(
            self.outcome.status,
            maestria_domain::SearchStatus::NoEvidenceFound
                | maestria_domain::SearchStatus::Abstained
                | maestria_domain::SearchStatus::DeniedByPolicy
                | maestria_domain::SearchStatus::QuarantinedForReview
        )
    }
}
