use maestria_domain::{EvidenceCandidate, SearchOutcome, SearchPlan, SearchStatus};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use tokio::task::JoinSet;

use crate::traits::{CandidateRetriever, ContextExpander, RetrievalEvaluator};
use crate::types::{
    CandidateRequest, ExpansionPolicy, RankedCandidate, RetrievalError, RetrievalExperiment,
    RetrievalResult,
};

pub(super) async fn collect_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &SearchQuery,
    web_requests_used: &mut u32,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let mut completed = std::iter::repeat_with(|| None)
        .take(retrievers.len())
        .collect::<Vec<_>>();
    let mut tasks = JoinSet::new();
    for (index, retriever) in retrievers.iter().enumerate() {
        let descriptor = retriever.descriptor();
        if descriptor.modality.eq_ignore_ascii_case("web")
            && *web_requests_used >= plan.budgets.max_web_requests()
        {
            completed[index] = Some(crate::types::CandidateBatch {
                descriptor,
                query: query.q.clone(),
                candidates: Vec::new(),
                status: maestria_domain::SearchLaneStatus::Failed {
                    error: "web request budget exhausted".to_string(),
                },
                generation: Some(plan.index_generation),
            });
            continue;
        }
        if descriptor.modality.eq_ignore_ascii_case("web") {
            *web_requests_used = web_requests_used.saturating_add(1);
        }
        let retriever = Arc::clone(retriever);
        let request = CandidateRequest {
            plan: plan.clone(),
            query: query.clone(),
            expected_generation: plan.index_generation,
        };
        tasks.spawn(async move { (index, descriptor, retriever.retrieve(request).await) });
    }

    while let Some(result) = tasks.join_next().await {
        let (index, descriptor, result) = result
            .map_err(|error| RetrievalError::Internal(format!("retriever task failed: {error}")))?;
        let batch = match result {
            Ok(mut batch) => {
                if batch.generation != Some(plan.index_generation) {
                    batch.candidates.clear();
                    batch.status = maestria_domain::SearchLaneStatus::Failed {
                        error: format!(
                            "stale lane generation: expected {}, got {}",
                            plan.index_generation,
                            batch.generation.map_or_else(
                                || "missing".to_string(),
                                |generation| generation.to_string()
                            ),
                        ),
                    };
                }
                batch
                    .candidates
                    .truncate(plan.stop_conditions.max_results as usize);
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
            Err(RetrievalError::Cancelled) => return Err(RetrievalError::Cancelled),
            Err(error) => crate::types::CandidateBatch {
                descriptor,
                query: query.q.clone(),
                candidates: Vec::new(),
                status: maestria_domain::SearchLaneStatus::Failed {
                    error: error.to_string(),
                },
                generation: Some(plan.index_generation),
            },
        };
        completed[index] = Some(batch);
    }

    completed
        .into_iter()
        .map(|batch| {
            batch.ok_or_else(|| {
                RetrievalError::Internal("retriever task produced no result".to_string())
            })
        })
        .collect()
}
pub(super) async fn collect_initial_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
) -> RetrievalResult<(
    Vec<crate::types::CandidateBatch>,
    crate::rewrite::QueryRewriteSession,
    u32,
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
    for rewrite in session.records() {
        let rewrite_query = SearchQuery {
            q: rewrite.query.clone(),
            limit: plan.stop_conditions.max_results as usize,
            offset: 0,
        };
        batches.extend(
            collect_batches(retrievers, plan, &rewrite_query, &mut web_requests_used).await?,
        );
    }
    Ok((batches, session, web_requests_used))
}

pub(super) async fn collect_missing_slot_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &str,
    web_requests_used: &mut u32,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let query = SearchQuery {
        q: query.to_string(),
        limit: plan.stop_conditions.max_results as usize,
        offset: 0,
    };
    collect_batches(retrievers, plan, &query, web_requests_used).await
}

pub(super) fn trace_lanes(
    batches: &[crate::types::CandidateBatch],
) -> Vec<maestria_domain::SearchTraceLane> {
    batches
        .iter()
        .map(|batch| maestria_domain::SearchTraceLane {
            retriever_id: batch.descriptor.id.clone(),
            query: batch.query.clone(),
            status: batch.status.clone(),
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
) -> RetrievalResult<(SearchOutcome, crate::diversity::DiversitySelection)> {
    let selected_candidates = initial.candidates.clone();
    let expansion_policy = ExpansionPolicy {
        max_results: plan.stop_conditions.max_results as usize,
        max_depth: plan.stages.len(),
        selected_seeds: selected_candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect(),
        required_claims: initial.coverage.required_claims.clone(),
        required_subquestions: initial.coverage.required_subquestions.clone(),
    };
    let expanded = if let Some(expander) = expander {
        expander.expand(&selected_candidates, &expansion_policy)?
    } else {
        selected_candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect()
    };
    let expanded_ranked = expanded
        .into_iter()
        .enumerate()
        .map(|(rank, candidate)| RankedCandidate { candidate, rank })
        .collect::<Vec<_>>();
    let final_diversity = crate::diversity::select_candidates(&expanded_ranked, plan);
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
    ensure_exact_lineage_from_evidence(&report.outcome.evidence, &final_diversity.candidates)?;
    Ok((report.outcome, final_diversity))
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
    if evidence.len() != seeds.len()
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
