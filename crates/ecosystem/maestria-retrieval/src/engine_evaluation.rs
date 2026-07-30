use maestria_domain::{SearchOutcome, SearchPlan};
use maestria_ports::SearchQuery;

use super::{RetrievalEngine, engine_pipeline, reconcile_status};
use crate::types::{RankedCandidate, RerankRequest, RetrievalResult};
fn batch_is_code(batch: &crate::types::CandidateBatch) -> bool {
    batch.descriptor.modality.eq_ignore_ascii_case("code")
        || batch.descriptor.modality.eq_ignore_ascii_case("rust")
        || batch
            .descriptor
            .id
            .to_ascii_lowercase()
            .contains("code_intel")
}

pub(super) async fn evaluate_batches(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
    query: &SearchQuery,
    batches: &[crate::types::CandidateBatch],
    started: tokio::time::Instant,
    execution_usage: &mut maestria_domain::SearchExecutionUsage,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
) -> RetrievalResult<(
    SearchOutcome,
    Vec<maestria_domain::SearchTraceLane>,
    Option<maestria_domain::SearchTraceRerank>,
    maestria_domain::SearchTraceDiversity,
)> {
    let lanes = engine_pipeline::trace_lanes(batches);
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
    );
    let ranked = if let Some(fusion) = &engine.fusion {
        fusion
            .fuse(query, &fusion_batches)?
            .into_iter()
            .enumerate()
            .map(|(rank, fused)| RankedCandidate {
                candidate: fused.candidate,
                rank,
            })
            .collect()
    } else {
        fusion_batches
            .iter()
            .filter(|batch| matches!(batch.status, maestria_domain::SearchLaneStatus::Succeeded))
            .flat_map(|batch| batch.candidates.iter().cloned())
            .enumerate()
            .map(|(rank, candidate)| RankedCandidate { candidate, rank })
            .collect()
    };
    let (ranked, rerank_trace) =
        apply_reranking(engine, plan, visual_enabled, started, ranked).await?;
    let initial_diversity = crate::diversity::select_candidates(&ranked, plan);
    let expansion_enabled = plan
        .stages
        .contains(&maestria_domain::SearchStage::Filtering);
    let configured_expander = expansion_enabled.then(|| engine.expander.clone()).flatten();
    let (mut raw_outcome, final_diversity) = engine_pipeline::run_diversity_stage(
        plan,
        initial_diversity,
        &configured_expander,
        &engine.evaluator,
        execution_usage,
        authorization,
    )
    .await?;
    raw_outcome.status = reconcile_status(&raw_outcome.status, &final_diversity.status);
    if stale_code_only
        && raw_outcome.evidence.is_empty()
        && matches!(
            raw_outcome.status,
            maestria_domain::SearchStatus::NoEvidenceFound
        )
    {
        raw_outcome.status = maestria_domain::SearchStatus::StaleEvidenceOnly;
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
) -> (Vec<crate::types::CandidateBatch>, bool) {
    let has_non_code_evidence = batches
        .iter()
        .any(|batch| !batch_is_code(batch) && !batch.candidates.is_empty());
    let has_fresh_code_evidence = batches.iter().any(|batch| {
        batch_is_code(batch)
            && batch.candidates.iter().any(|candidate| {
                !matches!(candidate.freshness, maestria_domain::FreshnessStatus::Stale)
            })
    });
    let stale_code_only = !has_non_code_evidence
        && !has_fresh_code_evidence
        && batches.iter().any(|batch| {
            batch_is_code(batch)
                && batch.candidates.iter().any(|candidate| {
                    matches!(candidate.freshness, maestria_domain::FreshnessStatus::Stale)
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
            )
        })
        .filter_map(|batch| {
            let mut batch = batch.clone();
            if batch_is_code(&batch) {
                batch.candidates.retain(|candidate| {
                    !matches!(candidate.freshness, maestria_domain::FreshnessStatus::Stale)
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

async fn apply_reranking(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
    visual_enabled: bool,
    started: tokio::time::Instant,
    ranked: Vec<RankedCandidate>,
) -> RetrievalResult<(
    Vec<RankedCandidate>,
    Option<maestria_domain::SearchTraceRerank>,
)> {
    if plan
        .stages
        .contains(&maestria_domain::SearchStage::Reranking)
        && (!engine.visual_reranker || visual_enabled)
        && let Some(reranker) = &engine.reranker
    {
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let remaining_ms = u64::from(plan.budgets.max_latency_ms())
            .saturating_sub(elapsed_ms)
            .min(u64::from(u32::MAX)) as u32;
        let rerank_res = reranker
            .rerank(RerankRequest {
                plan: plan.clone(),
                candidates: ranked,
                max_latency_ms: remaining_ms,
            })
            .await?;
        return Ok((rerank_res.candidates, Some(rerank_res.trace)));
    }
    Ok((ranked, None))
}
