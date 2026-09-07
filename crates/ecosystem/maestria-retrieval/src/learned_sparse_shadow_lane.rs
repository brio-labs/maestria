//! Shadow-lane record construction: candidate conversion and typed lane
//! assembly for learned-sparse shadow observations.

use maestria_domain::{RetrievalReason, RetrievalScoreKind, SearchLaneStatus};

use super::learned_sparse_shadow_execution::{failed_lane, status_from_error};
use super::{LearnedSparseShadowCandidate, LearnedSparseShadowLane, LearnedSparseShadowLaneStatus};
use crate::types::RetrieverDescriptor;

pub(super) fn lane_from_batch(
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    sparse_identity: Option<maestria_ports::SparseIdentity>,
    batch: crate::types::CandidateBatch,
    max_candidates: usize,
    max_contributions: usize,
) -> LearnedSparseShadowLane {
    // Lane ranks are bounded by `max_candidates`; a rank that cannot be
    // represented in the typed lane contract degrades the whole lane
    // explicitly instead of fabricating a sentinel value (R24).
    let Ok(max_rank) = u32::try_from(max_candidates) else {
        return failed_lane(
            descriptor,
            namespace,
            sparse_identity,
            "shadow candidate rank exceeds the u32 lane contract",
        );
    };
    let candidates = batch
        .candidates
        .iter()
        .take(max_rank as usize)
        .enumerate()
        .filter_map(|(rank, candidate)| {
            // `rank < max_rank` (take bound), so `rank + 1 <= max_rank` fits u32.
            shadow_candidate(candidate, rank as u32 + 1, max_contributions)
        })
        .collect::<Vec<_>>();
    let status = match batch.status {
        SearchLaneStatus::Succeeded if candidates.is_empty() => {
            LearnedSparseShadowLaneStatus::IncompatibleIdentity
        }
        SearchLaneStatus::Succeeded => LearnedSparseShadowLaneStatus::Succeeded,
        SearchLaneStatus::Empty => LearnedSparseShadowLaneStatus::Empty,
        SearchLaneStatus::Failed { error } => status_from_error(&error),
    };
    LearnedSparseShadowLane {
        retriever_id: descriptor.id,
        representation: descriptor.representation,
        generation: descriptor.generation,
        namespace,
        sparse_identity,
        status,
        candidates,
    }
}

fn shadow_candidate(
    candidate: &maestria_domain::EvidenceCandidate,
    lane_rank: u32,
    max_contributions: usize,
) -> Option<LearnedSparseShadowCandidate> {
    let score = candidate
        .scores()
        .lane(&RetrievalScoreKind::LearnedSparse)?
        .clone();
    candidate.reasons().iter().find_map(|reason| {
        let RetrievalReason::LearnedSparse(reason) = reason else {
            return None;
        };
        let mut reason = reason.as_ref().clone();
        reason.contributions.truncate(max_contributions);
        Some(LearnedSparseShadowCandidate {
            evidence_id: candidate.evidence_id(),
            artifact_version: candidate.artifact_version(),
            source_span: candidate.source_span().clone(),
            lane_rank,
            score: score.clone(),
            reason,
        })
    })
}
