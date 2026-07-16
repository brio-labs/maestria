use crate::types::{SourceGroundedCardHit, SourceGroundedSearchHit};
use maestria_domain::{ArtifactId, CardId, EvidenceId};
use std::collections::BTreeMap;

const RRF_K: u64 = 60;
const RRF_SCALE: u64 = 1_000_000;

#[derive(Clone)]
pub(super) enum RetrievalCandidate {
    Card(SourceGroundedCardHit),
    Chunk(SourceGroundedSearchHit),
    EvidenceId(EvidenceId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RetrievalLane {
    Cards,
    VectorChunks,
    ExactChunks,
    LexicalChunks,
    Hierarchy,
}

#[derive(Clone)]
pub(super) struct RankedRetrievalCandidate {
    pub candidate: RetrievalCandidate,
    pub lane: RetrievalLane,
    pub rank: usize,
    pub priority_score: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateKey {
    Card(CardId),
    Evidence(EvidenceId),
}

impl RetrievalCandidate {
    fn key(&self) -> CandidateKey {
        match self {
            Self::Card(hit) => CandidateKey::Card(hit.card.id),
            Self::Chunk(hit) => CandidateKey::Evidence(hit.evidence.id),
            Self::EvidenceId(id) => CandidateKey::Evidence(*id),
        }
    }
    pub(super) fn identity(&self) -> (u8, u64) {
        match self.key() {
            CandidateKey::Card(id) => (0, id.value()),
            CandidateKey::Evidence(id) => (1, id.value()),
        }
    }
    pub(super) fn artifact_id(&self) -> Option<ArtifactId> {
        match self {
            Self::Card(hit) => Some(hit.artifact.id),
            Self::Chunk(hit) => Some(hit.artifact.id),
            Self::EvidenceId(_) => None,
        }
    }

    pub(super) fn score(&self) -> u32 {
        match self {
            Self::Card(hit) => hit.score,
            Self::Chunk(hit) => hit.score,
            Self::EvidenceId(_) => 0,
        }
    }
}

/// Annotates an already ranked lane with its stable ordinal positions.
pub(super) fn rank_lane(
    lane: RetrievalLane,
    candidates: Vec<RetrievalCandidate>,
) -> Vec<RankedRetrievalCandidate> {
    candidates
        .into_iter()
        .enumerate()
        .map(|(rank, candidate)| RankedRetrievalCandidate {
            priority_score: candidate.score(),
            candidate,
            lane,
            rank,
        })
        .collect()
}

pub(super) fn rank_expanded(
    lane: RetrievalLane,
    candidates: Vec<RetrievalCandidate>,
    priorities: &BTreeMap<(u8, u64), u32>,
    fallback_priority: u32,
) -> Vec<RankedRetrievalCandidate> {
    candidates
        .into_iter()
        .enumerate()
        .map(|(rank, candidate)| RankedRetrievalCandidate {
            priority_score: priorities
                .get(&candidate.identity())
                .copied()
                .map_or(fallback_priority, |score| score),
            candidate,
            lane,
            rank,
        })
        .collect()
}

/// Fuses lane results with fixed-point Reciprocal Rank Fusion.
///
/// Ranks are one-based in the denominator and candidates are deduplicated by
/// their stable domain identity. Ties use the identity ordering, so identical
/// lane inputs always produce identical results.
pub(super) fn fuse(
    limit: usize,
    lanes: Vec<Vec<RankedRetrievalCandidate>>,
) -> Vec<RankedRetrievalCandidate> {
    if limit == 0 {
        return Vec::new();
    }
    let mut fused: BTreeMap<CandidateKey, FusedCandidate> = BTreeMap::new();
    for lane in lanes {
        for ranked in lane {
            let key = ranked.candidate.key();
            let contribution = RRF_SCALE / (RRF_K + ranked.rank as u64 + 1);
            match fused.get_mut(&key) {
                Some(existing) => {
                    existing.score = existing.score.saturating_add(contribution);
                    if prefer_candidate(&ranked.candidate, &existing.candidate) {
                        existing.candidate = ranked.candidate;
                        existing.lane = ranked.lane;
                    }
                }
                None => {
                    fused.insert(
                        key,
                        FusedCandidate {
                            candidate: ranked.candidate,
                            lane: ranked.lane,
                            score: contribution,
                        },
                    );
                }
            }
        }
    }

    let mut values: Vec<_> = fused.into_values().collect();
    values.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.candidate.key().cmp(&right.candidate.key()))
            .then_with(|| left.lane.cmp(&right.lane))
    });
    values
        .into_iter()
        .take(limit)
        .enumerate()
        .map(|(rank, value)| RankedRetrievalCandidate {
            candidate: value.candidate,
            priority_score: value.score.min(u64::from(u32::MAX)) as u32,
            lane: value.lane,
            rank,
        })
        .collect()
}

struct FusedCandidate {
    candidate: RetrievalCandidate,
    lane: RetrievalLane,
    score: u64,
}

fn prefer_candidate(left: &RetrievalCandidate, right: &RetrievalCandidate) -> bool {
    match (left, right) {
        (RetrievalCandidate::Chunk(left), RetrievalCandidate::Chunk(right)) => {
            left.lexical_metadata.is_some() && right.lexical_metadata.is_none()
                || left.score > right.score
        }
        (RetrievalCandidate::Card(left), RetrievalCandidate::Card(right)) => {
            left.lexical_metadata.is_some() && right.lexical_metadata.is_none()
                || left.score > right.score
        }
        (RetrievalCandidate::Chunk(_), RetrievalCandidate::EvidenceId(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{RetrievalCandidate, RetrievalLane, fuse, rank_lane};
    use maestria_domain::EvidenceId;

    #[test]
    fn duplicate_candidates_accumulate_reciprocal_rank() {
        let first = rank_lane(
            RetrievalLane::LexicalChunks,
            vec![RetrievalCandidate::EvidenceId(EvidenceId::new(1))],
        );
        let second = rank_lane(
            RetrievalLane::VectorChunks,
            vec![RetrievalCandidate::EvidenceId(EvidenceId::new(1))],
        );
        let fused = fuse(10, vec![first, second]);
        assert_eq!(fused.len(), 1);
        assert_eq!(
            fused[0].candidate.key(),
            super::CandidateKey::Evidence(EvidenceId::new(1))
        );
        assert_eq!(fused[0].rank, 0);
    }

    #[test]
    fn ties_are_ordered_by_stable_identity() {
        let first = rank_lane(
            RetrievalLane::LexicalChunks,
            vec![RetrievalCandidate::EvidenceId(EvidenceId::new(2))],
        );
        let second = rank_lane(
            RetrievalLane::VectorChunks,
            vec![RetrievalCandidate::EvidenceId(EvidenceId::new(1))],
        );
        let fused = fuse(10, vec![first, second]);
        let ids: Vec<_> = fused
            .into_iter()
            .map(|candidate| match candidate.candidate {
                RetrievalCandidate::EvidenceId(id) => id,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ids, vec![EvidenceId::new(1), EvidenceId::new(2)]);
    }

    #[test]
    fn zero_limit_is_a_typed_empty_result() {
        let lane = rank_lane(
            RetrievalLane::LexicalChunks,
            vec![RetrievalCandidate::EvidenceId(EvidenceId::new(1))],
        );
        assert!(fuse(0, vec![lane]).is_empty());
    }
}
