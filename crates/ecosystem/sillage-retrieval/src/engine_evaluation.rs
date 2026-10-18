use sillage_domain::{SearchOutcome, SearchPlan};
use sillage_ports::SearchQuery;

use super::{RetrievalEngine, engine_pipeline, reconcile_status};
use crate::types::{RankedCandidate, RerankRequest, RetrievalResult};

pub(super) struct EvaluationRequest<'a> {
    pub(super) engine: &'a RetrievalEngine,
    pub(super) plan: &'a SearchPlan,
    pub(super) query: &'a SearchQuery,
    pub(super) batches: &'a [crate::types::CandidateBatch],
    pub(super) started: crate::MonotonicInstant,
    pub(super) execution_usage: &'a mut sillage_domain::SearchExecutionUsage,
    pub(super) authorization: &'a sillage_governance::RetrievalAuthorizationContext,
    pub(super) source_filter: Option<&'a crate::types::CandidateSourceFilter>,
}

pub(super) fn evaluate_batches(
    request: EvaluationRequest<'_>,
) -> RetrievalResult<(
    SearchOutcome,
    Vec<sillage_domain::SearchTraceLane>,
    Option<sillage_domain::SearchTraceRerank>,
    sillage_domain::SearchTraceDiversity,
)> {
    let EvaluationRequest {
        engine,
        plan,
        query,
        batches,
        started,
        execution_usage,
        authorization,
        source_filter,
    } = request;
    let original_query = plan.original_query();
    let lanes = engine_pipeline::trace_lanes(batches)?;
    let repository_specialized = engine
        .repository_execution_policy
        .allows_specialized(&query.q);
    let visual_enabled = engine.visual_execution_policy.allows_visual(&query.q);
    let sparse_enabled = engine
        .learned_sparse_execution_policy
        .allows_sparse(&query.q);
    let (fusion_batches, stale_code_only) = prepare_fusion_batches(
        engine,
        batches,
        visual_enabled,
        sparse_enabled,
        repository_specialized,
        original_query,
    );
    let (ranked, protected_lexical_head) = if let Some(fusion) = &engine.fusion {
        let output = fusion.fuse_with_protected_head(query, &fusion_batches, None)?;
        let ranked = output
            .candidates
            .into_iter()
            .enumerate()
            .map(|(rank, fused)| RankedCandidate {
                candidate: fused.candidate,
                rank,
            })
            .collect();
        (ranked, output.protected_lexical_head)
    } else {
        let ranked = fusion_batches
            .iter()
            .filter(|batch| matches!(batch.status, sillage_domain::SearchLaneStatus::Succeeded))
            .flat_map(|batch| batch.candidates.iter().cloned())
            .enumerate()
            .map(|(rank, candidate)| RankedCandidate { candidate, rank })
            .collect();
        (ranked, None)
    };
    let (ranked, rerank_trace) = apply_reranking(
        engine,
        plan,
        visual_enabled,
        started,
        protected_lexical_head,
        ranked,
    )?;
    let initial_diversity = crate::diversity::select_candidates(&ranked, plan)?;
    let expansion_enabled = plan
        .stages()
        .contains(&sillage_domain::SearchStage::Filtering);
    let configured_expander = expansion_enabled.then(|| engine.expander.clone()).flatten();
    let (mut raw_outcome, final_diversity) =
        engine_pipeline::run_diversity_stage(engine_pipeline::DiversityStageRequest {
            plan,
            initial: initial_diversity,
            expander: &configured_expander,
            evaluator: &engine.evaluator,
            execution_usage,
            authorization,
            source_filter,
            protected_lexical_head,
        })?;
    raw_outcome.status = reconcile_status(&raw_outcome.status, &final_diversity.status);
    if stale_code_only
        && raw_outcome.evidence.is_empty()
        && matches!(
            raw_outcome.status,
            sillage_domain::SearchStatus::NoEvidenceFound
        )
    {
        raw_outcome.status = sillage_domain::SearchStatus::StaleEvidenceOnly;
    }
    raw_outcome.coverage = final_diversity.coverage.clone();
    Ok((raw_outcome, lanes, rerank_trace, final_diversity.trace))
}

fn prepare_fusion_batches(
    engine: &RetrievalEngine,
    batches: &[crate::types::CandidateBatch],
    visual_enabled: bool,
    sparse_enabled: bool,
    repository_specialized: bool,
    original_query: &str,
) -> (Vec<crate::types::CandidateBatch>, bool) {
    let has_non_code_evidence = batches
        .iter()
        .any(|batch| !batch.descriptor.is_code() && !batch.candidates.is_empty());
    let has_fresh_code_evidence = batches.iter().any(|batch| {
        batch.descriptor.is_code()
            && batch.candidates.iter().any(|candidate| {
                !matches!(
                    candidate.freshness(),
                    sillage_domain::FreshnessStatus::Stale
                )
            })
    });
    let stale_code_only = !has_non_code_evidence
        && !has_fresh_code_evidence
        && batches.iter().any(|batch| {
            batch.descriptor.is_code()
                && batch.candidates.iter().any(|candidate| {
                    matches!(
                        candidate.freshness(),
                        sillage_domain::FreshnessStatus::Stale
                    )
                })
        });
    let fusion_batches: Vec<_> = batches
        .iter()
        .filter(|batch| {
            crate::visual_benchmark::visual_lane_is_eligible(&batch.descriptor, visual_enabled)
        })
        .filter(|batch| {
            crate::learned_sparse_policy::sparse_lane_is_eligible(&batch.descriptor, sparse_enabled)
        })
        .filter(|batch| {
            super::batch_is_eligible(
                &batch.descriptor,
                &engine.hybrid_policy,
                repository_specialized,
                original_query,
            )
        })
        .filter_map(|batch| {
            let mut batch = batch.clone();
            if batch.descriptor.is_code() {
                batch.candidates.retain(|candidate| {
                    !matches!(
                        candidate.freshness(),
                        sillage_domain::FreshnessStatus::Stale
                    )
                });
                if batch.candidates.is_empty() {
                    return None;
                }
            }
            Some(batch)
        })
        .collect();
    (fusion_batches, stale_code_only)
}

fn apply_reranking(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
    visual_enabled: bool,
    started: crate::MonotonicInstant,
    protected_lexical_head: Option<sillage_domain::EvidenceId>,
    ranked: Vec<RankedCandidate>,
) -> RetrievalResult<(
    Vec<RankedCandidate>,
    Option<sillage_domain::SearchTraceRerank>,
)> {
    if plan
        .stages()
        .contains(&sillage_domain::SearchStage::Reranking)
        && (!engine.visual_reranker || visual_enabled)
        && let Some(reranker) = &engine.reranker
    {
        let Some(protected_head) = protected_lexical_head else {
            return rerank_all(plan, started, reranker.as_ref(), ranked);
        };
        let mut ranked = ranked;
        let head_position = ranked
            .iter()
            .position(|candidate| candidate.candidate.evidence_id() == protected_head)
            .ok_or_else(|| {
                crate::types::RetrievalError::Internal(
                    "protected lexical result missing before reranking".to_string(),
                )
            })?;
        ranked[..=head_position].rotate_right(1);
        let head = ranked.remove(0);
        if ranked.is_empty() {
            return Ok((vec![head], None));
        }
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let remaining_ms = u64::from(plan.budgets().max_latency_ms())
            .saturating_sub(elapsed_ms)
            .min(u64::from(u32::MAX)) as u32;
        let rerank_res = reranker.rerank(RerankRequest {
            plan: std::sync::Arc::new(plan.clone()),
            candidates: ranked,
            max_latency_ms: remaining_ms,
        })?;
        let mut trace = rerank_res.trace;
        for candidate in &mut trace.candidates {
            if let sillage_domain::RerankPosition::Reranked(rank) = &mut candidate.position {
                *rank = rank.saturating_add(1);
            }
        }
        trace
            .candidates
            .push(sillage_domain::SearchTraceRerankCandidate {
                candidate_id: protected_head,
                original_rank: 0,
                position: sillage_domain::RerankPosition::SkippedNotApplicable,
                relevance_score: None,
                constraint_scores: Vec::new(),
            });
        let mut candidates = Vec::with_capacity(rerank_res.candidates.len() + 1);
        candidates.push(head);
        candidates.extend(rerank_res.candidates);
        for (rank, candidate) in candidates.iter_mut().enumerate() {
            candidate.rank = rank;
        }
        return Ok((candidates, Some(trace)));
    }
    Ok((ranked, None))
}

fn rerank_all(
    plan: &SearchPlan,
    started: crate::MonotonicInstant,
    reranker: &dyn crate::traits::CandidateReranker,
    ranked: Vec<RankedCandidate>,
) -> RetrievalResult<(
    Vec<RankedCandidate>,
    Option<sillage_domain::SearchTraceRerank>,
)> {
    let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let remaining_ms = u64::from(plan.budgets().max_latency_ms())
        .saturating_sub(elapsed_ms)
        .min(u64::from(u32::MAX)) as u32;
    let rerank_res = reranker.rerank(RerankRequest {
        plan: std::sync::Arc::new(plan.clone()),
        candidates: ranked,
        max_latency_ms: remaining_ms,
    })?;
    Ok((rerank_res.candidates, Some(rerank_res.trace)))
}
