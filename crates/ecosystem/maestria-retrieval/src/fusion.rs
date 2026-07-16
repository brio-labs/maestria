use maestria_domain::EvidenceCandidate;
use maestria_ports::SearchQuery;

use crate::traits::RankFusion;
use crate::types::{FusedCandidate, RetrievalError, RetrievalResult};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateIdentity {
    Cluster(maestria_domain::DuplicateClusterId),
    Exact(maestria_domain::EvidenceId),
}

const RRF_SCALE: u64 = 10_000_000;

/// Deterministic rank-only Reciprocal Rank Fusion.
pub struct FixedKRrf {
    pub k: usize,
}

impl FixedKRrf {
    pub fn new(k: usize) -> Self {
        Self { k }
    }
}

impl RankFusion for FixedKRrf {
    fn fuse(
        &self,
        _query: &SearchQuery,
        batches: &[crate::types::CandidateBatch],
    ) -> RetrievalResult<Vec<FusedCandidate>> {
        let k = u64::try_from(self.k).map_err(|_| {
            RetrievalError::Internal("RRF k does not fit the fixed-point denominator".to_string())
        })?;
        if k == 0 {
            return Err(RetrievalError::Internal(
                "RRF k must be greater than zero".to_string(),
            ));
        }
        let mut evidence_clusters = std::collections::BTreeMap::<
            maestria_domain::EvidenceId,
            maestria_domain::DuplicateClusterId,
        >::new();
        for batch in batches {
            if !matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded) {
                continue;
            }
            for candidate in &batch.candidates {
                if let Some(cluster) = candidate.duplicate_cluster {
                    evidence_clusters
                        .entry(candidate.evidence_id)
                        .and_modify(|existing| *existing = (*existing).min(cluster))
                        .or_insert(cluster);
                }
            }
        }

        let mut scores = std::collections::BTreeMap::<CandidateIdentity, u64>::new();
        let mut best_candidates =
            std::collections::BTreeMap::<CandidateIdentity, EvidenceCandidate>::new();
        for batch in batches {
            if !matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded) {
                continue;
            }
            let mut seen = std::collections::BTreeSet::new();
            let mut compact_rank = 0usize;
            for candidate in &batch.candidates {
                let identity =
                    candidate_identity(candidate, evidence_clusters.get(&candidate.evidence_id));
                if !seen.insert(identity.clone()) {
                    continue;
                }
                let rank = compact_rank.checked_add(1).ok_or_else(|| {
                    RetrievalError::Internal("RRF lane rank overflow".to_string())
                })?;
                let rank = u64::try_from(rank).map_err(|_| {
                    RetrievalError::Internal("RRF lane rank does not fit denominator".to_string())
                })?;
                let denominator = k.checked_add(rank).ok_or_else(|| {
                    RetrievalError::Internal("RRF denominator overflow".to_string())
                })?;
                let contribution = RRF_SCALE / denominator;
                compact_rank += 1;
                scores
                    .entry(identity.clone())
                    .and_modify(|score| *score = score.saturating_add(contribution))
                    .or_insert(contribution);
                let mut canonical_candidate = candidate.clone();
                if let CandidateIdentity::Cluster(cluster_id) = &identity {
                    canonical_candidate.duplicate_cluster = Some(*cluster_id);
                }
                let replace = best_candidates.get(&identity).is_none_or(|existing| {
                    candidate_order(&canonical_candidate) < candidate_order(existing)
                });
                if replace {
                    best_candidates.insert(identity, canonical_candidate);
                }
            }
        }

        let mut sorted = scores.into_iter().collect::<Vec<_>>();
        sorted.sort_by(|(left_id, left_score), (right_id, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_id.cmp(right_id))
        });
        Ok(sorted
            .into_iter()
            .filter_map(|(identity, score)| {
                best_candidates
                    .remove(&identity)
                    .map(|candidate| FusedCandidate {
                        candidate,
                        fused_score: score.min(u64::from(u32::MAX)) as u32,
                    })
            })
            .collect())
    }
}

fn candidate_identity(
    candidate: &EvidenceCandidate,
    normalized_cluster: Option<&maestria_domain::DuplicateClusterId>,
) -> CandidateIdentity {
    if let Some(cluster_id) = normalized_cluster.or(candidate.duplicate_cluster.as_ref()) {
        CandidateIdentity::Cluster(*cluster_id)
    } else {
        CandidateIdentity::Exact(candidate.evidence_id)
    }
}

fn candidate_order(candidate: &EvidenceCandidate) -> (u64, u64) {
    (
        candidate.evidence_id.value(),
        candidate.artifact_version.value(),
    )
}
