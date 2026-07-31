use std::sync::Arc;
use std::time::Duration;

use maestria_domain::{RetrievalReason, RetrievalScoreKind, SearchLaneStatus, SearchPlan};
use maestria_ports::SearchQuery;

use crate::learned_sparse_policy::classify_query;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateRequest, RetrieverDescriptor};

use super::learned_sparse_shadow_store::{LearnedSparseShadowStore, bounded_error};
use super::{
    LearnedSparseShadowCandidate, LearnedSparseShadowLane, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowObservation, LearnedSparseShadowRoute, MAX_SHADOW_LATENCY_MS,
    MAX_SHADOW_RETRIEVERS, SHADOW_SCHEMA_VERSION,
};

type ShadowRetriever = (
    Arc<dyn CandidateRetriever>,
    RetrieverDescriptor,
    Option<maestria_domain::SparseNamespace>,
    Option<maestria_ports::SparseIdentity>,
);

/// Owned handle for one non-serving shadow execution.
///
/// Dropping the handle aborts the provider task. A successfully completed
/// search may call [`Self::release`] to preserve the existing fire-and-forget
/// observation semantics.
pub(crate) struct LearnedSparseShadowTask {
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for LearnedSparseShadowTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl LearnedSparseShadowTask {
    pub(crate) fn release(mut self) {
        let _released = self.handle.take();
    }
}

pub(crate) fn spawn_learned_sparse_shadow(
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    plan: SearchPlan,
    authorization: maestria_governance::RetrievalAuthorizationContext,
    store: LearnedSparseShadowStore,
) -> Option<LearnedSparseShadowTask> {
    let retrievers = retrievers
        .into_iter()
        .take(MAX_SHADOW_RETRIEVERS)
        .map(|retriever| {
            let descriptor = retriever.descriptor();
            let namespace = retriever.sparse_namespace();
            let sparse_identity = retriever.sparse_identity();
            (retriever, descriptor, namespace, sparse_identity)
        })
        .collect::<Vec<_>>();
    if retrievers.is_empty() {
        return None;
    }
    Some(LearnedSparseShadowTask {
        handle: Some(tokio::spawn(async move {
            let observation = run_shadow(retrievers, plan, authorization).await;
            store.record(observation);
        })),
    })
}

async fn run_shadow(
    retrievers: Vec<ShadowRetriever>,
    plan: SearchPlan,
    authorization: maestria_governance::RetrievalAuthorizationContext,
) -> LearnedSparseShadowObservation {
    let started = tokio::time::Instant::now();
    let timeout_ms = u64::from(plan.budgets.max_latency_ms()).clamp(1, MAX_SHADOW_LATENCY_MS);
    let shadow_retrievers = retrievers.clone();
    let lanes = match plan.execution_budget() {
        Ok(execution_budget) => match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            collect_shadow_lanes(shadow_retrievers, &plan, &authorization, execution_budget),
        )
        .await
        {
            Ok(lanes) => lanes,
            Err(_) => retrievers
                .iter()
                .map(
                    |(_, descriptor, namespace, sparse_identity)| LearnedSparseShadowLane {
                        retriever_id: descriptor.id.clone(),
                        representation: descriptor.representation.clone(),
                        generation: descriptor.generation,
                        namespace: namespace.clone(),
                        sparse_identity: sparse_identity.clone(),
                        status: LearnedSparseShadowLaneStatus::TimedOut,
                        candidates: Vec::new(),
                    },
                )
                .collect(),
        },
        Err(error) => {
            let error = bounded_error(&format!("invalid shadow execution budget: {error}"));
            retrievers
                .into_iter()
                .map(|(_, descriptor, namespace, sparse_identity)| {
                    failed_lane(descriptor, namespace, sparse_identity, &error)
                })
                .collect()
        }
    };
    LearnedSparseShadowObservation {
        schema_version: SHADOW_SCHEMA_VERSION,
        query_id: plan.query_id,
        query_class: classify_query(&plan.original_query),
        route: LearnedSparseShadowRoute::Shadow,
        corpus_snapshot: plan.corpus_snapshot,
        index_generation: plan.index_generation,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        lanes,
    }
}

fn partition_allowance(total: u64, lanes: usize, lane: usize) -> u64 {
    let lanes = lanes.max(1) as u64;
    let base = total / lanes;
    base + if (lane as u64) < (total % lanes) {
        1
    } else {
        0
    }
}

fn shadow_lane_budget(
    global: maestria_domain::SearchExecutionBudget,
    lanes: usize,
    lane: usize,
) -> Option<maestria_domain::SearchExecutionBudget> {
    let max_bytes = match global.max_bytes_read() {
        Some(limit) => Some(std::num::NonZeroU64::new(partition_allowance(
            limit.get(),
            lanes,
            lane,
        ))?),
        None => None,
    };
    maestria_domain::SearchExecutionBudget::with_byte_limit(
        partition_allowance(global.max_results(), lanes, lane),
        partition_allowance(global.max_candidates(), lanes, lane),
        partition_allowance(global.max_work_units(), lanes, lane),
        max_bytes,
    )
    .ok()
}

async fn collect_shadow_lanes(
    retrievers: Vec<ShadowRetriever>,
    plan: &SearchPlan,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    execution_budget: maestria_domain::SearchExecutionBudget,
) -> Vec<LearnedSparseShadowLane> {
    let mut lanes = Vec::with_capacity(retrievers.len());
    let lane_count = retrievers.len().max(1);
    for (lane_index, (retriever, descriptor, namespace, sparse_identity)) in
        retrievers.into_iter().enumerate()
    {
        let Some(execution_budget) = shadow_lane_budget(execution_budget, lane_count, lane_index)
        else {
            lanes.push(failed_lane(
                descriptor,
                namespace,
                sparse_identity,
                "shadow execution budget exhausted before lane allocation",
            ));
            continue;
        };
        let query_limit = maestria_domain::saturating_usize(execution_budget.max_results());
        let max_candidates = maestria_domain::saturating_usize(execution_budget.max_candidates());
        let max_contributions =
            maestria_domain::saturating_usize(execution_budget.max_work_units());
        let request = CandidateRequest {
            plan: plan.clone(),
            query: SearchQuery {
                q: plan.original_query.clone(),
                limit: query_limit,
                offset: 0,
                execution_budget,
            },
            execution_budget,
            expected_generation: descriptor.generation,
            authorization: authorization.clone(),
        };
        let lane = match retriever.retrieve(request).await {
            Ok(batch)
                if batch.execution.budget != execution_budget
                    || batch.execution.usage.results
                        < maestria_domain::saturating_u64(batch.candidates.len())
                    || batch.execution.usage.candidates
                        < maestria_domain::saturating_u64(batch.candidates.len())
                    || batch.execution.usage.work_units
                        < maestria_domain::saturating_u64(batch.candidates.len()) =>
            {
                failed_lane(
                    descriptor,
                    namespace.clone(),
                    sparse_identity.clone(),
                    "shadow lane returned invalid execution metadata",
                )
            }
            Ok(batch) if batch.generation != Some(descriptor.generation) => failed_lane(
                descriptor,
                namespace.clone(),
                sparse_identity.clone(),
                "shadow lane returned an incompatible generation",
            ),
            Ok(batch) => lane_from_batch(
                descriptor,
                namespace,
                sparse_identity,
                batch,
                max_candidates,
                max_contributions,
            ),
            Err(error) => failed_lane(descriptor, namespace, sparse_identity, &error.to_string()),
        };
        lanes.push(lane);
    }
    lanes
}

fn lane_from_batch(
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    sparse_identity: Option<maestria_ports::SparseIdentity>,
    batch: crate::types::CandidateBatch,
    max_candidates: usize,
    max_contributions: usize,
) -> LearnedSparseShadowLane {
    let candidates = batch
        .candidates
        .iter()
        .take(max_candidates)
        .enumerate()
        .filter_map(|(rank, candidate)| shadow_candidate(candidate, rank, max_contributions))
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
    rank: usize,
    max_contributions: usize,
) -> Option<LearnedSparseShadowCandidate> {
    let score = candidate
        .scores
        .lane(&RetrievalScoreKind::LearnedSparse)?
        .clone();
    candidate.reasons.iter().find_map(|reason| {
        let RetrievalReason::LearnedSparse(reason) = reason else {
            return None;
        };
        let mut reason = reason.as_ref().clone();
        reason.contributions.truncate(max_contributions);
        Some(LearnedSparseShadowCandidate {
            evidence_id: candidate.evidence_id,
            artifact_version: candidate.artifact_version,
            source_span: candidate.source_span.clone(),
            lane_rank: match u32::try_from(rank.saturating_add(1)) {
                Ok(value) => value,
                Err(e) => {
                    let _ = e;
                    u32::MAX
                }
            },
            score: score.clone(),
            reason,
        })
    })
}

fn failed_lane(
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    sparse_identity: Option<maestria_ports::SparseIdentity>,
    error: &str,
) -> LearnedSparseShadowLane {
    LearnedSparseShadowLane {
        retriever_id: descriptor.id,
        representation: descriptor.representation,
        generation: descriptor.generation,
        namespace,
        sparse_identity,
        status: status_from_error(error),
        candidates: Vec::new(),
    }
}

fn status_from_error(error: &str) -> LearnedSparseShadowLaneStatus {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("privacy") {
        LearnedSparseShadowLaneStatus::PrivacyRejected
    } else if normalized.contains("security") || normalized.contains("secret scanner") {
        LearnedSparseShadowLaneStatus::SecurityFiltered
    } else if normalized.contains("stale") || normalized.contains("generation") {
        LearnedSparseShadowLaneStatus::StaleGeneration
    } else if normalized.contains("incompatible") || normalized.contains("identity") {
        LearnedSparseShadowLaneStatus::IncompatibleIdentity
    } else if normalized.contains("budget") || normalized.contains("exhaust") {
        LearnedSparseShadowLaneStatus::BudgetExhausted
    } else {
        LearnedSparseShadowLaneStatus::Failed {
            error: bounded_error(error),
        }
    }
}
