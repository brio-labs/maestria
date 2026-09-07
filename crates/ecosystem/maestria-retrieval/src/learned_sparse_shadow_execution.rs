use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maestria_domain::SearchPlan;
use maestria_ports::SearchQuery;

use crate::learned_sparse_policy::classify_query;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateRequest, RetrieverDescriptor};

use super::learned_sparse_shadow_store::{LearnedSparseShadowStore, bounded_error};
use super::{
    LearnedSparseShadowLane, LearnedSparseShadowLaneStatus, LearnedSparseShadowObservation,
    LearnedSparseShadowRoute, MAX_SHADOW_LATENCY_MS, MAX_SHADOW_RETRIEVERS, SHADOW_SCHEMA_VERSION,
};

type ShadowRetriever = (
    Arc<dyn CandidateRetriever>,
    RetrieverDescriptor,
    Option<maestria_domain::SparseNamespace>,
    Option<maestria_ports::SparseIdentity>,
);

/// Owned handle for one non-serving shadow execution.
///
/// Dropping the handle signals cancellation; the shadow thread observes it
/// between lanes and exits without recording a partial observation. A
/// successfully completed search may call [`Self::release`] to preserve the
/// existing fire-and-forget observation semantics.
pub(crate) struct LearnedSparseShadowTask {
    cancelled: Arc<AtomicBool>,
    released: bool,
}

impl Drop for LearnedSparseShadowTask {
    fn drop(&mut self) {
        if !self.released {
            self.cancelled.store(true, Ordering::Release);
        }
    }
}

impl LearnedSparseShadowTask {
    pub(crate) fn release(mut self) {
        self.released = true;
    }
}

pub(crate) fn spawn_learned_sparse_shadow(
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    plan: SearchPlan,
    authorization: maestria_governance::RetrievalAuthorizationContext,
    source_filter: Option<crate::types::CandidateSourceFilter>,
    store: LearnedSparseShadowStore,
) -> Option<LearnedSparseShadowTask> {
    let retrievers = retrievers
        .into_iter()
        .take(MAX_SHADOW_RETRIEVERS)
        .map(|retriever| {
            let descriptor = (*retriever.descriptor()).clone();
            let namespace = retriever.sparse_namespace();
            let sparse_identity = retriever.sparse_identity();
            (retriever, descriptor, namespace, sparse_identity)
        })
        .collect::<Vec<_>>();
    if retrievers.is_empty() {
        return None;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let thread_cancelled = Arc::clone(&cancelled);
    std::thread::Builder::new()
        .name("learned-sparse-shadow".to_string())
        .spawn(move || {
            let observation = run_shadow(
                retrievers,
                &plan,
                &authorization,
                source_filter.as_ref(),
                &thread_cancelled,
            );
            if !thread_cancelled.load(Ordering::Acquire) {
                store.record(observation);
            }
        })
        .ok();
    Some(LearnedSparseShadowTask {
        cancelled,
        released: false,
    })
}

fn run_shadow(
    retrievers: Vec<ShadowRetriever>,
    plan: &SearchPlan,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    source_filter: Option<&crate::types::CandidateSourceFilter>,
    cancelled: &AtomicBool,
) -> LearnedSparseShadowObservation {
    let started = crate::MonotonicInstant::now();
    let timeout_ms = u64::from(plan.budgets().max_latency_ms()).clamp(1, MAX_SHADOW_LATENCY_MS);
    let lanes = match plan.execution_budget() {
        Ok(execution_budget) => collect_shadow_lanes(ShadowLaneRequest {
            retrievers,
            plan,
            authorization,
            source_filter,
            execution_budget,
            started,
            lane_deadline: Duration::from_millis(timeout_ms),
            cancelled,
        }),
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
        query_id: plan.query_id(),
        query_class: classify_query(plan.original_query()),
        route: LearnedSparseShadowRoute::Shadow,
        corpus_snapshot: plan.corpus_snapshot(),
        index_generation: plan.index_generation(),
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        lanes,
    }
}

fn shadow_lane_budget(
    global: maestria_domain::SearchExecutionBudget,
    lanes: usize,
    lane: usize,
) -> Option<maestria_domain::SearchExecutionBudget> {
    let max_bytes = match global.max_bytes_read() {
        Some(limit) => Some(std::num::NonZeroU64::new(
            crate::engine::partition_allowance(limit.get(), lanes, lane),
        )?),
        None => None,
    };
    maestria_domain::SearchExecutionBudget::with_byte_limit(
        // Result ceiling is not partitioned: shadow lanes must observe the
        // full candidate depth to produce comparable observations.
        global.max_results(),
        crate::engine::partition_allowance(global.max_candidates(), lanes, lane),
        crate::engine::partition_allowance(global.max_work_units(), lanes, lane),
        max_bytes,
    )
    .ok()
}

struct ShadowLaneRequest<'a> {
    retrievers: Vec<ShadowRetriever>,
    plan: &'a SearchPlan,
    authorization: &'a maestria_governance::RetrievalAuthorizationContext,
    source_filter: Option<&'a crate::types::CandidateSourceFilter>,
    execution_budget: maestria_domain::SearchExecutionBudget,
    started: crate::MonotonicInstant,
    lane_deadline: Duration,
    cancelled: &'a AtomicBool,
}

fn collect_shadow_lanes(request: ShadowLaneRequest<'_>) -> Vec<LearnedSparseShadowLane> {
    let ShadowLaneRequest {
        retrievers,
        plan,
        authorization,
        source_filter,
        execution_budget,
        started,
        lane_deadline,
        cancelled,
    } = request;
    let mut lanes = Vec::with_capacity(retrievers.len());
    let lane_count = retrievers.len().max(1);
    let lane_meta: Vec<(
        RetrieverDescriptor,
        Option<maestria_domain::SparseNamespace>,
        Option<maestria_ports::SparseIdentity>,
    )> = retrievers
        .iter()
        .map(|(_, descriptor, namespace, sparse_identity)| {
            (
                descriptor.clone(),
                namespace.clone(),
                sparse_identity.clone(),
            )
        })
        .collect();
    for (lane_index, (retriever, descriptor, namespace, sparse_identity)) in
        retrievers.into_iter().enumerate()
    {
        if cancelled.load(Ordering::Acquire) {
            return lanes;
        }
        if started.elapsed() >= lane_deadline {
            for (descriptor, namespace, sparse_identity) in lane_meta.iter().skip(lane_index) {
                lanes.push(timed_out_lane(
                    descriptor.clone(),
                    namespace.clone(),
                    sparse_identity.clone(),
                ));
            }
            return lanes;
        }
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
        let lane = evaluate_shadow_lane(
            ShadowLaneJob {
                retriever,
                descriptor,
                namespace,
                sparse_identity,
                execution_budget,
            },
            plan,
            authorization,
            source_filter,
        );
        lanes.push(lane);
    }
    lanes
}

/// One planned shadow lane ready for execution and validation.
struct ShadowLaneJob {
    retriever: Arc<dyn CandidateRetriever>,
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    sparse_identity: Option<maestria_ports::SparseIdentity>,
    execution_budget: maestria_domain::SearchExecutionBudget,
}

/// Executes one shadow lane and validates its execution metadata.
fn evaluate_shadow_lane(
    job: ShadowLaneJob,
    plan: &SearchPlan,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    source_filter: Option<&crate::types::CandidateSourceFilter>,
) -> LearnedSparseShadowLane {
    let ShadowLaneJob {
        retriever,
        descriptor,
        namespace,
        sparse_identity,
        execution_budget,
    } = job;
    let query_limit = maestria_domain::saturating_usize(execution_budget.max_results());
    let max_candidates = maestria_domain::saturating_usize(execution_budget.max_candidates());
    let max_contributions = maestria_domain::saturating_usize(execution_budget.max_work_units());
    let request = CandidateRequest {
        plan: std::sync::Arc::new(plan.clone()),
        query: SearchQuery {
            q: plan.original_query().to_string(),
            limit: query_limit,
            offset: 0,
            execution_budget,
        },
        execution_budget,
        expected_generation: descriptor.generation,
        authorization: authorization.clone(),
        source_filter: source_filter.cloned(),
    };
    match retriever.retrieve(request) {
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
        Ok(batch) => super::learned_sparse_shadow_lane::lane_from_batch(
            descriptor,
            namespace,
            sparse_identity,
            batch,
            max_candidates,
            max_contributions,
        ),
        Err(error) => failed_lane(descriptor, namespace, sparse_identity, &error.to_string()),
    }
}

fn timed_out_lane(
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    sparse_identity: Option<maestria_ports::SparseIdentity>,
) -> LearnedSparseShadowLane {
    LearnedSparseShadowLane {
        retriever_id: descriptor.id,
        representation: descriptor.representation,
        generation: descriptor.generation,
        namespace,
        sparse_identity,
        status: LearnedSparseShadowLaneStatus::TimedOut,
        candidates: Vec::new(),
    }
}

pub(super) fn failed_lane(
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

pub(super) fn status_from_error(error: &str) -> LearnedSparseShadowLaneStatus {
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
