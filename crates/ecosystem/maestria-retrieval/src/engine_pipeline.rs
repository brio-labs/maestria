use maestria_domain::{EvidenceCandidate, SearchOutcome, SearchPlan, SearchStatus};
use maestria_ports::SearchQuery;
use std::sync::Arc;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::traits::{CandidateRetriever, ContextExpander, RetrievalEvaluator};
use crate::types::{
    CandidateRequest, ExpansionPolicy, RankedCandidate, RetrievalError, RetrievalExperiment,
    RetrievalResult,
};

fn normalize_batch(
    mut batch: crate::types::CandidateBatch,
    descriptor: crate::types::RetrieverDescriptor,
    query: &SearchQuery,
    plan: &SearchPlan,
    web_bytes_read: &mut u64,
) -> crate::types::CandidateBatch {
    if batch.generation != Some(descriptor.generation) {
        batch.candidates.clear();
        batch.status = maestria_domain::SearchLaneStatus::Failed {
            error: format!(
                "stale lane generation: expected {}, got {}",
                descriptor.generation,
                batch.generation.map_or_else(
                    || "missing".to_string(),
                    |generation| generation.to_string()
                ),
            ),
        };
    }
    if descriptor.modality.eq_ignore_ascii_case("web") {
        let remaining_bytes = plan
            .budgets
            .max_bytes_read()
            .saturating_sub(*web_bytes_read);
        if batch.bytes_read > remaining_bytes {
            batch.candidates.clear();
            batch.status = maestria_domain::SearchLaneStatus::Failed {
                error: "web byte budget exhausted".to_string(),
            };
        } else {
            *web_bytes_read = (*web_bytes_read).saturating_add(batch.bytes_read);
        }
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

pub(super) async fn collect_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &SearchQuery,
    web_requests_used: &mut u32,
    web_bytes_read: &mut u64,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let mut completed = std::iter::repeat_with(|| None)
        .take(retrievers.len())
        .collect::<Vec<_>>();
    let mut tasks = JoinSet::new();
    let concurrency = usize::try_from(plan.budgets.max_concurrency())
        .map_or(1, |value| value)
        .max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    for (index, retriever) in retrievers.iter().enumerate() {
        let descriptor = retriever.descriptor();
        let generation = descriptor.generation;
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
                generation: Some(generation),
                bytes_read: 0,
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
            expected_generation: descriptor.generation,
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
            (index, descriptor, result)
        });
    }

    while let Some(result) = tasks.join_next().await {
        let (index, descriptor, result) = result
            .map_err(|error| RetrievalError::Internal(format!("retriever task failed: {error}")))?;
        let generation = descriptor.generation;
        let batch = match result {
            Ok(batch) => normalize_batch(batch, descriptor, query, plan, web_bytes_read),
            Err(RetrievalError::Cancelled) => return Err(RetrievalError::Cancelled),
            Err(error) => crate::types::CandidateBatch {
                descriptor,
                query: query.q.clone(),
                candidates: Vec::new(),
                status: maestria_domain::SearchLaneStatus::Failed {
                    error: error.to_string(),
                },
                generation: Some(generation),
                bytes_read: 0,
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
    u64,
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
    let mut web_bytes_read = 0_u64;
    for rewrite in session.records() {
        let rewrite_query = SearchQuery {
            q: rewrite.query.clone(),
            limit: plan.stop_conditions.max_results as usize,
            offset: 0,
        };
        batches.extend(
            collect_batches(
                retrievers,
                plan,
                &rewrite_query,
                &mut web_requests_used,
                &mut web_bytes_read,
            )
            .await?,
        );
    }
    Ok((batches, session, web_requests_used, web_bytes_read))
}

pub(super) async fn collect_missing_slot_batches(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    query: &str,
    web_requests_used: &mut u32,
    web_bytes_read: &mut u64,
) -> RetrievalResult<Vec<crate::types::CandidateBatch>> {
    let query = SearchQuery {
        q: query.to_string(),
        limit: plan.stop_conditions.max_results as usize,
        offset: 0,
    };
    collect_batches(retrievers, plan, &query, web_requests_used, web_bytes_read).await
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
