//! Scoped-thread lane workers for parallel candidate generation.
//!
//! The dispatch planner produces [`LaneJob`]s; [`run_lane_jobs`] executes
//! them on scoped worker threads bounded by the plan's `max_concurrency`
//! budget and returns results in completion order.

use std::sync::Arc;

use maestria_domain::{SearchExecutionBudget, SearchPlan};

use super::engine_budget::LanePermits;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateRequest, RetrievalError, RetrievalResult};

/// One executable retrieval lane prepared by the dispatch planner.
pub(super) struct LaneJob {
    pub(super) index: usize,
    pub(super) descriptor: crate::types::RetrieverDescriptor,
    pub(super) allocation: SearchExecutionBudget,
    pub(super) retriever: Arc<dyn CandidateRetriever>,
    pub(super) request: CandidateRequest,
}

/// One completed lane worker: the planned lane plus its batch result.
pub(super) type RetrieverTask = (
    usize,
    crate::types::RetrieverDescriptor,
    SearchExecutionBudget,
    RetrievalResult<crate::types::CandidateBatch>,
);

/// Runs every planned lane on a scoped worker thread, bounded by the
/// plan's `max_concurrency` budget, and joins all workers before
/// returning.
pub(super) fn run_lane_jobs(
    plan: &SearchPlan,
    jobs: Vec<LaneJob>,
) -> RetrievalResult<Vec<RetrieverTask>> {
    let concurrency =
        maestria_domain::saturating_usize(u64::from(plan.budgets().max_concurrency())).max(1);
    let lane_permits = LanePermits::new(concurrency);
    std::thread::scope(|scope| -> RetrievalResult<Vec<RetrieverTask>> {
        let permits = &lane_permits;
        let handles: Vec<_> = jobs
            .into_iter()
            .map(|job| {
                let LaneJob {
                    index,
                    descriptor,
                    allocation,
                    retriever,
                    request,
                } = job;
                scope.spawn(move || {
                    let _permit = permits.acquire();
                    (index, descriptor, allocation, retriever.retrieve(request))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| RetrievalError::Internal("retriever lane failed".to_string()))
            })
            .collect()
    })
}
