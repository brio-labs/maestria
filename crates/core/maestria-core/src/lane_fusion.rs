use crate::ports::CorePorts;
use crate::rank_fusion::{RankedRetrievalCandidate, RetrievalCandidate, RetrievalLane, rank_lane};
use crate::retrieval_lanes::{search_cards, search_chunks, search_vector_chunks};
use crate::types::{RetrievalLaneReport, RetrievalLaneStatus, SourceGroundedSearchHit};
use maestria_ports::VectorSearchQuery;

pub(super) struct LaneRun {
    pub(super) ranked: Vec<RankedRetrievalCandidate>,
    pub(super) report: RetrievalLaneReport,
}

fn lane_id(lane: RetrievalLane) -> &'static str {
    match lane {
        RetrievalLane::Cards => "cards",
        RetrievalLane::VectorChunks => "dense_chunks",
        RetrievalLane::ExactChunks => "exact_chunks",
        RetrievalLane::LexicalChunks => "lexical_chunks",
        RetrievalLane::Hierarchy => "hierarchy",
    }
}

fn trace_candidate(
    hit: &SourceGroundedSearchHit,
    rank: usize,
    reason: maestria_domain::RetrievalReason,
    semantic: bool,
) -> Option<maestria_domain::SearchTraceLaneCandidate> {
    let candidate = crate::trace_candidates::evidence_candidate_from_hit(
        hit.clone(),
        &reason,
        semantic,
        &maestria_domain::FreshnessRequirement::Any,
        0,
    )?;
    Some(maestria_domain::SearchTraceLaneCandidate {
        evidence_id: candidate.evidence_id,
        artifact_version: candidate.artifact_version,
        source_span: candidate.source_span,
        lane_rank: (rank + 1) as u32,
        duplicate_cluster: candidate.duplicate_cluster,
        scores: candidate.scores,
        reasons: candidate.reasons,
    })
}

pub(super) fn run_cards_lane(
    ports: &CorePorts<'_>,
    query: &str,
    limit: usize,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> LaneRun {
    let lane = RetrievalLane::Cards;
    match search_cards(ports, query, limit, policy) {
        Ok(cards) => {
            let status = if cards.is_empty() {
                RetrievalLaneStatus::Empty
            } else {
                RetrievalLaneStatus::Succeeded
            };
            let ranked = rank_lane(
                lane,
                cards
                    .into_iter()
                    .map(|card| RetrievalCandidate::Card(Box::new(card)))
                    .collect(),
            );
            LaneRun {
                ranked,
                report: RetrievalLaneReport {
                    retriever_id: lane_id(lane).to_string(),
                    query: query.to_string(),
                    status,
                    candidates: Vec::new(),
                },
            }
        }
        Err(error) => LaneRun {
            ranked: Vec::new(),
            report: RetrievalLaneReport {
                retriever_id: lane_id(lane).to_string(),
                query: query.to_string(),
                status: RetrievalLaneStatus::Failed {
                    error: error.to_string(),
                },
                candidates: Vec::new(),
            },
        },
    }
}

pub(super) fn run_chunk_lane(
    ports: &CorePorts<'_>,
    lane: RetrievalLane,
    query: &str,
    limit: usize,
    vector_query: Option<VectorSearchQuery>,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> LaneRun {
    let result = if lane == RetrievalLane::VectorChunks {
        search_vector_chunks(ports, query, limit, vector_query, policy)
    } else {
        search_chunks(ports, query, limit, policy)
    };
    let semantic = lane == RetrievalLane::VectorChunks;
    let reason = if semantic {
        maestria_domain::RetrievalReason::SemanticSimilarity
    } else {
        maestria_domain::RetrievalReason::ExactMatch
    };
    match result {
        Ok((hits, _)) => {
            let status = if hits.is_empty() {
                RetrievalLaneStatus::Empty
            } else {
                RetrievalLaneStatus::Succeeded
            };
            let report_candidates = hits
                .iter()
                .enumerate()
                .filter_map(|(rank, hit)| trace_candidate(hit, rank, reason.clone(), semantic))
                .collect::<Vec<_>>();
            let ranked = rank_lane(
                lane,
                hits.into_iter()
                    .map(|hit| RetrievalCandidate::Chunk(Box::new(hit)))
                    .collect(),
            );
            LaneRun {
                ranked,
                report: RetrievalLaneReport {
                    retriever_id: lane_id(lane).to_string(),
                    query: query.to_string(),
                    status,
                    candidates: report_candidates,
                },
            }
        }
        Err(error) => LaneRun {
            ranked: Vec::new(),
            report: RetrievalLaneReport {
                query: query.to_string(),
                retriever_id: lane_id(lane).to_string(),
                status: RetrievalLaneStatus::Failed {
                    error: error.to_string(),
                },
                candidates: Vec::new(),
            },
        },
    }
}
