use maestria_domain::{
    EvidenceCandidate, SearchExecution, SearchExecutionBudget, SearchExecutionCompletion,
    SearchExecutionUsage, SearchOutcome, SearchPlan, SearchStatus,
};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::traits::{CandidateRetriever, ContextExpander, RetrievalEvaluator};
use crate::types::{
    CandidateRequest, ExpansionPolicy, RankedCandidate, RetrievalError, RetrievalExperiment,
    RetrievalResult,
};

fn lane_uses_primary_generation(descriptor: &crate::types::RetrieverDescriptor) -> bool {
    !descriptor.modality.eq_ignore_ascii_case("dense")
        && !descriptor.modality.eq_ignore_ascii_case("image")
        && !descriptor.modality.eq_ignore_ascii_case("sparse")
        && !descriptor.modality.eq_ignore_ascii_case("sparse-shadow")
}

fn lane_generation_is_current(
    descriptor: &crate::types::RetrieverDescriptor,
    plan: &SearchPlan,
) -> bool {
    !lane_uses_primary_generation(descriptor) || descriptor.generation == plan.index_generation
}

fn execution_with_budget(
    budget: SearchExecutionBudget,
    completion: SearchExecutionCompletion,
) -> SearchExecution {
    SearchExecution::new(budget, SearchExecutionUsage::default(), completion)
}

fn add_usage(total: &mut SearchExecutionUsage, usage: SearchExecutionUsage) {
    total.results = total.results.saturating_add(usage.results);
    total.candidates = total.candidates.saturating_add(usage.candidates);
    total.work_units = total.work_units.saturating_add(usage.work_units);
    total.bytes_read = total.bytes_read.saturating_add(usage.bytes_read);
}

fn usage_within_budget(usage: SearchExecutionUsage, budget: SearchExecutionBudget) -> bool {
    usage.results <= budget.max_results()
        && usage.candidates <= budget.max_candidates()
        && usage.work_units <= budget.max_work_units()
        && budget
            .max_bytes_read()
            .is_none_or(|limit| usage.bytes_read <= limit.get())
}

fn partition_allowance(total: u64, lanes: usize, lane: usize) -> u64 {
    let lanes = lanes.max(1) as u64;
    let base = total / lanes;
    let remainder = total % lanes;
    base + if (lane as u64) < remainder { 1 } else { 0 }
}

fn lane_budget(
    plan: &SearchPlan,
    remaining: SearchExecutionUsage,
    lanes: usize,
    lane: usize,
) -> Option<SearchExecutionBudget> {
    let global = plan.execution_budget().ok()?;
    let max_results = global.max_results().saturating_sub(remaining.results);
    let max_candidates = global.max_candidates().saturating_sub(remaining.candidates);
    let max_work_units = global.max_work_units().saturating_sub(remaining.work_units);
    if max_results == 0 || max_candidates == 0 || max_work_units == 0 {
        return None;
    }
    let remaining_bytes = global
        .max_bytes_read()
        .map(|limit| limit.get().saturating_sub(remaining.bytes_read));
    if remaining_bytes == Some(0) {
        return None;
    }
    let partitioned_bytes = remaining_bytes.map(|limit| partition_allowance(limit, lanes, lane));
    if partitioned_bytes == Some(0) {
        return None;
    }
    let max_bytes = partitioned_bytes.and_then(std::num::NonZeroU64::new);
    SearchExecutionBudget::with_byte_limit(
        partition_allowance(max_results, lanes, lane),
        partition_allowance(max_candidates, lanes, lane),
        partition_allowance(max_work_units, lanes, lane),
        max_bytes,
    )
    .ok()
}

fn normalize_batch(
    mut batch: crate::types::CandidateBatch,
    descriptor: crate::types::RetrieverDescriptor,
    query: &SearchQuery,
    plan: &SearchPlan,
    allocation: SearchExecutionBudget,
    usage: &mut SearchExecutionUsage,
) -> crate::types::CandidateBatch {
    let expected_generation = descriptor.generation;
    let batch_descriptor_generation = batch.descriptor.generation;
    let generation_matches = lane_generation_is_current(&descriptor, plan)
        && batch.descriptor == descriptor
        && batch.generation == Some(expected_generation);
    let candidate_count = maestria_domain::saturating_u64(batch.candidates.len());
    let candidate_usage_matches = candidate_count <= batch.execution.usage.results
        && candidate_count <= batch.execution.usage.candidates
        && candidate_count <= batch.execution.usage.work_units;
    let metadata_matches = batch.execution.budget == allocation
        && usage_within_budget(batch.execution.usage, allocation)
        && candidate_usage_matches;
    if !generation_matches {
        batch.candidates.clear();
        batch.status = maestria_domain::SearchLaneStatus::Failed {
            error: format!(
                "stale lane generation: expected lane {}, plan primary {}, retriever {}, batch descriptor {}, batch {}",
                expected_generation,
                plan.index_generation,
                descriptor.generation,
                batch_descriptor_generation,
                batch.generation.map_or_else(
                    || "missing".to_string(),
                    |generation| generation.to_string()
                ),
            ),
        };
    } else if !metadata_matches {
        batch.candidates.clear();
        batch.status = maestria_domain::SearchLaneStatus::Failed {
            error: format!(
                "invalid execution metadata for {} lane",
                descriptor.modality
            ),
        };
    } else {
        add_usage(usage, batch.execution.usage);
    }
    batch.candidates.truncate(allocation.max_results() as usize);
    batch.descriptor = descriptor;
    batch.query = query.q.clone();
    if !matches!(
        batch.status,
        maestria_domain::SearchLaneStatus::Failed { .. }
    ) {
        batch.status = if batch.candidates.is_empty() {
            maestria_domain::SearchLaneStatus::Empty
        } else {
            maestria_domain::SearchLaneStatus::Succeeded
        };
    }
    batch
}

type CompletedLane = Option<(
    crate::types::RetrieverDescriptor,
    SearchExecutionBudget,
    crate::types::CandidateBatch,
)>;
type RetrieverTask = (
    usize,
    crate::types::RetrieverDescriptor,
    SearchExecutionBudget,
    RetrievalResult<crate::types::CandidateBatch>,
);

async fn dispatch_eligible_lanes(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &SearchQuery,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    web_requests_used: &mut u32,
    execution_usage: SearchExecutionUsage,
) -> RetrievalResult<(Vec<CompletedLane>, JoinSet<RetrieverTask>, usize)> {
    let eligible = retrievers
        .iter()
        .enumerate()
        .filter_map(|(index, retriever)| {
            let descriptor = retriever.descriptor();
            let web_blocked = descriptor.modality.eq_ignore_ascii_case("web")
                && *web_requests_used >= plan.budgets.max_web_requests();
            (lane_generation_is_current(&descriptor, plan) && !web_blocked)
                .then_some((index, descriptor))
        })
        .collect::<Vec<_>>();
    let lane_count = eligible.len();
    let mut completed = std::iter::repeat_with(|| None)
        .take(retrievers.len())
        .collect::<Vec<CompletedLane>>();
    let mut tasks = JoinSet::new();
    let concurrency =
        maestria_domain::saturating_usize(u64::from(plan.budgets.max_concurrency())).max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    for (lane, (index, descriptor)) in eligible.into_iter().enumerate() {
        let Some(allocation) = lane_budget(plan, execution_usage, lane_count, lane) else {
            let exhausted_budget = plan.execution_budget()?;
            completed[index] = Some((
                descriptor.clone(),
                exhausted_budget,
                crate::types::CandidateBatch {
                    descriptor: descriptor.clone(),
                    query: query.q.clone(),
                    candidates: Vec::new(),
                    status: maestria_domain::SearchLaneStatus::Failed {
                        error: "execution budget exhausted before lane dispatch".to_string(),
                    },
                    generation: Some(descriptor.generation),
                    execution: execution_with_budget(
                        exhausted_budget,
                        SearchExecutionCompletion::Exhausted(
                            maestria_domain::SearchExecutionResource::Candidates,
                        ),
                    ),
                },
            ));
            continue;
        };
        let generation = descriptor.generation;
        if descriptor.modality.eq_ignore_ascii_case("web") {
            *web_requests_used = web_requests_used.saturating_add(1);
        }
        let retriever = Arc::clone(&retrievers[index]);
        let mut request_query = query.clone();
        request_query.execution_budget = allocation;
        request_query.limit = request_query
            .limit
            .min(maestria_domain::saturating_usize(allocation.max_results()));
        let request = CandidateRequest {
            plan: plan.clone(),
            query: request_query,
            execution_budget: allocation,
            expected_generation: generation,
            authorization: authorization.clone(),
        };
        let semaphore = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let result = match semaphore.acquire_owned().await {
                Ok(permit) => {
                    let result = retriever.retrieve(request).await;
                    drop(permit);
                    result
                }
                Err(error) => Err(RetrievalError::Internal(error.to_string())),
            };
            (index, descriptor, allocation, result)
        });
    }
    Ok((completed, tasks, lane_count))
}

pub(super) async fn collect_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &SearchQuery,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    web_requests_used: &mut u32,
    execution_usage: &mut SearchExecutionUsage,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let (mut completed, mut tasks, lane_count) = dispatch_eligible_lanes(
        retrievers,
        plan,
        query,
        authorization,
        web_requests_used,
        *execution_usage,
    )
    .await?;
    for (index, retriever) in retrievers.iter().enumerate() {
        if completed[index].is_some() {
            continue;
        }
        let descriptor = retriever.descriptor();
        if lane_generation_is_current(&descriptor, plan)
            && !(descriptor.modality.eq_ignore_ascii_case("web")
                && *web_requests_used >= plan.budgets.max_web_requests())
        {
            continue;
        }
        let allocation = lane_budget(plan, *execution_usage, lane_count.max(1), 0)
            .or_else(|| plan.execution_budget().ok())
            .ok_or_else(|| RetrievalError::Internal("invalid execution budget".to_string()))?;
        let error = if !lane_generation_is_current(&descriptor, plan) {
            format!(
                "stale retriever generation: expected primary {}, got {}",
                plan.index_generation, descriptor.generation
            )
        } else {
            "web request budget exhausted".to_string()
        };
        completed[index] = Some((
            descriptor.clone(),
            allocation,
            crate::types::CandidateBatch {
                descriptor,
                query: query.q.clone(),
                candidates: Vec::new(),
                status: maestria_domain::SearchLaneStatus::Failed { error },
                generation: Some(retriever.descriptor().generation),
                execution: execution_with_budget(allocation, SearchExecutionCompletion::Complete),
            },
        ));
    }
    while let Some(result) = tasks.join_next().await {
        let (index, descriptor, allocation, result) = result
            .map_err(|error| RetrievalError::Internal(format!("retriever task failed: {error}")))?;
        let batch = match result {
            Ok(batch) => normalize_batch(
                batch,
                descriptor.clone(),
                query,
                plan,
                allocation,
                execution_usage,
            ),
            Err(error) => crate::types::CandidateBatch {
                descriptor: descriptor.clone(),
                query: query.q.clone(),
                candidates: Vec::new(),
                status: maestria_domain::SearchLaneStatus::Failed {
                    error: error.to_string(),
                },
                generation: Some(descriptor.generation),
                execution: execution_with_budget(allocation, SearchExecutionCompletion::Complete),
            },
        };
        completed[index] = Some((descriptor, allocation, batch));
    }
    let mut batches = Vec::with_capacity(completed.len());
    for (descriptor, allocation, batch) in completed.into_iter().flatten() {
        let batch = if batch.execution.budget == allocation {
            batch
        } else {
            normalize_batch(batch, descriptor, query, plan, allocation, execution_usage)
        };
        batches.push(batch);
    }
    Ok(batches)
}
pub(super) async fn collect_initial_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
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
            limit: plan.stop_conditions.max_results as usize,
            offset: 0,
            execution_budget: plan.execution_budget()?,
        };
        batches.extend(
            collect_batches(
                retrievers,
                plan,
                &rewrite_query,
                authorization,
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
    web_requests_used: &mut u32,
    execution_usage: &mut SearchExecutionUsage,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let query = SearchQuery {
        q: query.to_string(),
        limit: plan.stop_conditions.max_results as usize,
        offset: 0,
        execution_budget: plan.execution_budget()?,
    };
    collect_batches(
        retrievers,
        plan,
        &query,
        authorization,
        web_requests_used,
        execution_usage,
    )
    .await
}

pub(super) fn trace_lanes(
    batches: &[crate::types::CandidateBatch],
) -> Vec<maestria_domain::SearchTraceLane> {
    batches
        .iter()
        .map(|batch| maestria_domain::SearchTraceLane {
            retriever_id: batch.descriptor.id.clone(),
            query: batch.query.clone(),
            generation: Some(batch.descriptor.generation),
            status: batch.status.clone(),
            execution: batch.execution,
            candidates: batch
                .candidates
                .iter()
                .enumerate()
                .map(
                    |(rank, candidate)| maestria_domain::SearchTraceLaneCandidate {
                        evidence_id: candidate.evidence_id,
                        artifact_version: candidate.artifact_version,
                        source_span: candidate.source_span.clone(),
                        lane_rank: (rank + 1) as u32,
                        duplicate_cluster: candidate.duplicate_cluster,
                        scores: candidate.scores.clone(),
                        reasons: candidate.reasons.clone(),
                    },
                )
                .collect(),
        })
        .collect()
}

pub(super) async fn run_diversity_stage(
    plan: &SearchPlan,
    initial: crate::diversity::DiversitySelection,
    expander: &Option<Arc<dyn ContextExpander>>,
    evaluator: &Arc<dyn RetrievalEvaluator>,
    execution_usage: &mut SearchExecutionUsage,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
) -> RetrievalResult<(SearchOutcome, crate::diversity::DiversitySelection)> {
    let selected_candidates = initial.candidates.clone();
    let budget = plan.execution_budget()?;
    let remaining_candidates = budget
        .max_candidates()
        .saturating_sub(execution_usage.candidates);
    let expansion_policy = ExpansionPolicy {
        max_results: (plan.stop_conditions.max_results as u64)
            .min(remaining_candidates)
            .min(usize::MAX as u64) as usize,
        max_depth: plan.stages.len(),
        selected_seeds: selected_candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect(),
        required_claims: initial.coverage.required_claims.clone(),
        required_subquestions: initial.coverage.required_subquestions.clone(),
        authorization: authorization.clone(),
    };
    let expanded = if let Some(expander) = expander {
        expander.expand(&selected_candidates, &expansion_policy)?
    } else {
        selected_candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect()
    };
    let mut expansion_budget_exhausted = false;
    let mut bounded_expanded = Vec::with_capacity(expanded.len());
    for candidate in expanded {
        let is_seed = selected_candidates
            .iter()
            .any(|seed| seed.candidate.evidence_id == candidate.evidence_id);
        if !is_seed {
            let range = candidate.source_span.range();
            let candidate_bytes = range.end.saturating_sub(range.start) as u64;
            let mut candidate_usage = *execution_usage;
            candidate_usage.candidates = candidate_usage.candidates.saturating_add(1);
            candidate_usage.work_units = candidate_usage.work_units.saturating_add(1);
            candidate_usage.bytes_read = candidate_usage.bytes_read.saturating_add(candidate_bytes);
            if !usage_within_budget(candidate_usage, budget) {
                expansion_budget_exhausted = true;
                continue;
            }
            *execution_usage = candidate_usage;
        }
        bounded_expanded.push(candidate);
    }
    let expanded_ranked = bounded_expanded
        .into_iter()
        .enumerate()
        .map(|(rank, candidate)| RankedCandidate { candidate, rank })
        .collect::<Vec<_>>();
    let mut final_diversity = crate::diversity::select_candidates(&expanded_ranked, plan);
    if expansion_budget_exhausted {
        final_diversity.trace.stop_reason = maestria_domain::SearchStopReason::BudgetExhausted;
    }
    ensure_exact_lineage(&final_diversity.candidates, &selected_candidates)?;
    let candidates = final_diversity
        .candidates
        .iter()
        .map(|candidate| candidate.candidate.clone())
        .collect();
    let report = evaluator
        .evaluate(RetrievalExperiment {
            plan: plan.clone(),
            candidates,
        })
        .await?;
    let mut outcome = report.outcome;
    if expansion_budget_exhausted
        && matches!(
            outcome.status,
            maestria_domain::SearchStatus::Answerable
                | maestria_domain::SearchStatus::AnswerableWithWarnings
        )
    {
        outcome.status = maestria_domain::SearchStatus::EvidenceIncomplete;
    }
    ensure_exact_lineage_from_evidence(&outcome.evidence, &final_diversity.candidates)?;
    Ok((outcome, final_diversity))
}

fn ensure_exact_lineage(
    candidates: &[RankedCandidate],
    seeds: &[RankedCandidate],
) -> RetrievalResult<()> {
    let evidence = candidates
        .iter()
        .map(|candidate| candidate.candidate.clone())
        .collect::<Vec<_>>();
    ensure_exact_lineage_from_evidence(&evidence, seeds)
}

fn ensure_exact_lineage_from_evidence(
    evidence: &[EvidenceCandidate],
    seeds: &[RankedCandidate],
) -> RetrievalResult<()> {
    if evidence.len() < seeds.len()
        || seeds.iter().any(|seed| {
            !evidence
                .iter()
                .any(|candidate| candidate == &seed.candidate)
        })
    {
        return Err(RetrievalError::Internal(
            "evidence stage changed selected candidate lineage".to_string(),
        ));
    }
    Ok(())
}

pub fn reconcile_status(
    evaluator_status: &SearchStatus,
    selector_status: &SearchStatus,
) -> SearchStatus {
    match evaluator_status {
        SearchStatus::SourcesConflict
        | SearchStatus::DeniedByPolicy
        | SearchStatus::QuarantinedForReview
        | SearchStatus::Abstained => evaluator_status.clone(),
        _ => match selector_status {
            SearchStatus::NoEvidenceFound
            | SearchStatus::AnswerableWithWarnings
            | SearchStatus::EvidenceIncomplete
            | SearchStatus::StaleEvidenceOnly => selector_status.clone(),
            _ => evaluator_status.clone(),
        },
    }
}
