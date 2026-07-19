use maestria_domain::{
    RerankCandidateStatus, RetrievalModelFingerprint, SearchTraceConstraintScore,
    SearchTraceRerank, SearchTraceRerankCandidate,
};

use crate::types::{RankedCandidate, RerankLimits, RerankResult};

pub(super) fn reorder_visual_candidates(
    candidates: Vec<RankedCandidate>,
    visual_positions: &[usize],
    mut scored: Vec<(usize, u32)>,
    mut trace: Vec<SearchTraceRerankCandidate>,
    limits: RerankLimits,
    model: String,
    fingerprint: RetrievalModelFingerprint,
) -> RerankResult {
    scored.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    let selected = scored
        .iter()
        .take(limits.output_cap)
        .map(|(position, score)| (*position, *score))
        .collect::<Vec<_>>();
    let mut visual_order = selected
        .iter()
        .map(|(position, _)| *position)
        .collect::<Vec<_>>();
    visual_order.extend(visual_positions.iter().copied().filter(|position| {
        !selected
            .iter()
            .any(|(selected_position, _)| selected_position == position)
    }));

    let mut reranked = candidates.clone();
    for (slot, source_position) in visual_positions.iter().copied().zip(visual_order) {
        reranked[slot] = candidates[source_position].clone();
        if let Some((_, score)) = selected
            .iter()
            .find(|(selected_position, _)| *selected_position == source_position)
        {
            trace[slot] = SearchTraceRerankCandidate {
                candidate_id: reranked[slot].candidate.evidence_id,
                original_rank: candidates[source_position].rank,
                new_rank: Some(slot),
                status: RerankCandidateStatus::Reranked,
                relevance_score: Some(*score),
                constraint_score: Some(*score),
                constraint_scores: vec![SearchTraceConstraintScore {
                    name: "visual_cosine".to_string(),
                    score: *score,
                }],
            };
        } else {
            trace[slot] = SearchTraceRerankCandidate {
                candidate_id: reranked[slot].candidate.evidence_id,
                original_rank: candidates[source_position].rank,
                new_rank: Some(slot),
                status: RerankCandidateStatus::SkippedCap,
                relevance_score: None,
                constraint_score: None,
                constraint_scores: Vec::new(),
            };
        }
    }
    for (rank, candidate) in reranked.iter_mut().enumerate() {
        candidate.rank = rank;
    }
    for trace_candidate in &mut trace {
        trace_candidate.new_rank = reranked
            .iter()
            .position(|candidate| candidate.candidate.evidence_id == trace_candidate.candidate_id);
    }
    trace.sort_by_key(|candidate| candidate.candidate_id);
    RerankResult {
        candidates: reranked,
        trace: SearchTraceRerank {
            model,
            fingerprint,
            input_cap: limits.input_cap,
            score_cap: limits.score_cap,
            output_cap: limits.output_cap,
            candidates: trace,
        },
    }
}
