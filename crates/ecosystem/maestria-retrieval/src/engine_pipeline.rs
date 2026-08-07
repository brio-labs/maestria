use maestria_domain::{
    SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionUsage, SearchPlan,
    SearchTraceLaneCandidateDto,
};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::traits::CandidateRetriever;
use crate::types::{CandidateRequest, RetrievalError, RetrievalResult};

#[path = "engine_budget.rs"]
mod engine_budget;
pub use engine_budget::lane_budget;
pub(super) use engine_budget::{
    add_usage, execution_with_budget, remaining_budget, usage_within_budget,
};

#[path = "engine_diversity.rs"]
mod engine_diversity;
pub use engine_diversity::reconcile_status;
pub(crate) use engine_diversity::run_diversity_stage;
#[path = "engine_pipeline_dispatch.rs"]
mod dispatch;
pub(super) use dispatch::collect_batches;

pub(super) async fn collect_initial_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    source_filter: Option<&crate::types::CandidateSourceFilter>,
) -> RetrievalResult<(
    Vec<crate::types::CandidateBatch>,
    crate::rewrite::QueryRewriteSession,
    u32,
    SearchExecutionUsage,
)> {
    let session = super::rewrite_session(plan);
    if session
        .records()
        .iter()
        .any(|record| record.stage != crate::rewrite::StageRole::InitialRetrieval)
    {
        return Err(RetrievalError::Internal(
            "retrieval engine cannot dispatch non-initial rewrite stages".to_string(),
        ));
    }
    let mut batches = Vec::new();
    let mut web_requests_used = 0_u32;
    let mut execution_usage = SearchExecutionUsage::default();
    for rewrite in session.records() {
        let rewrite_query = SearchQuery {
            q: rewrite.query.clone(),
            limit: plan.stop_conditions().max_results as usize,
            offset: 0,
            execution_budget: plan.execution_budget()?,
        };
        batches.extend(
            collect_batches(
                retrievers,
                plan,
                &rewrite_query,
                authorization,
                source_filter,
                &mut web_requests_used,
                &mut execution_usage,
            )
            .await?,
        );
    }
    Ok((batches, session, web_requests_used, execution_usage))
}
pub(super) async fn collect_missing_slot_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &str,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    source_filter: Option<&crate::types::CandidateSourceFilter>,
    web_requests_used: &mut u32,
    execution_usage: &mut SearchExecutionUsage,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let query = SearchQuery {
        q: query.to_string(),
        limit: plan.stop_conditions().max_results as usize,
        offset: 0,
        execution_budget: plan.execution_budget()?,
    };
    collect_batches(
        retrievers,
        plan,
        &query,
        authorization,
        source_filter,
        web_requests_used,
        execution_usage,
    )
    .await
}

pub(super) fn trace_lanes(
    batches: &[crate::types::CandidateBatch],
) -> RetrievalResult<Vec<maestria_domain::SearchTraceLane>> {
    batches
        .iter()
        .map(|batch| {
            Ok(maestria_domain::SearchTraceLane {
                retriever_id: batch.descriptor.id.clone(),
                query: batch.query.clone(),
                generation: Some(batch.descriptor.generation),
                status: batch.status.clone(),
                execution: batch.execution,
                candidates: batch
                    .candidates
                    .iter()
                    .enumerate()
                    .map(|(rank, candidate)| {
                        maestria_domain::SearchTraceLaneCandidate::new(
                            SearchTraceLaneCandidateDto {
                                evidence_id: candidate.evidence_id(),
                                artifact_version: candidate.artifact_version(),
                                source_span: candidate.source_span().clone(),
                                lane_rank: (rank + 1) as u32,
                                duplicate_cluster: candidate.duplicate_cluster(),
                                scores: candidate.scores().clone(),
                                reasons: candidate.reasons().to_vec(),
                            },
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            })
        })
        .collect()
}
