use maestria_domain::{
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTraceDiversity,
    SearchTraceLane, SearchTraceRerank,
};
use maestria_ports::SearchQuery;

use super::RetrievalEngine;
use super::engine_pipeline;
use crate::rewrite::{RewriteAccounting, RewriteOrigin, StageRole};
use crate::types::{CandidateBatch, RetrievalError, RetrievalResult};

pub(super) struct AdaptiveSearchState {
    pub(super) batches: Vec<CandidateBatch>,
    pub(super) rewrites: crate::rewrite::QueryRewriteSession,
    pub(super) web_requests_used: u32,
    pub(super) outcome: SearchOutcome,
    pub(super) lanes: Vec<SearchTraceLane>,
    pub(super) rerank_trace: Option<SearchTraceRerank>,
    pub(super) diversity_trace: SearchTraceDiversity,
}

pub(super) async fn iterate_until_stop(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
    query: &SearchQuery,
    state: &mut AdaptiveSearchState,
    started: tokio::time::Instant,
) -> RetrievalResult<Option<SearchStopReason>> {
    use std::collections::BTreeSet;

    let mut attempted_slots = BTreeSet::new();
    let mut iteration_count = 0_usize;
    let max_iterations = (plan.budgets.max_stages() as usize).saturating_sub(plan.stages.len());
    let mut previous_evidence = evidence_ids(&state.outcome);
    loop {
        let missing_slots = missing_required_slots(plan, &state.outcome);
        if missing_slots.is_empty() {
            return Ok(None);
        }
        if let Some(stop_reason) = terminal_stop_reason(&state.outcome.status) {
            return Ok(Some(stop_reason));
        }
        if iteration_count >= max_iterations
            || state.rewrites.records().len() >= plan.budgets.max_queries() as usize
            || started.elapsed().as_millis() >= u128::from(plan.budgets.max_latency_ms())
        {
            return Ok(Some(SearchStopReason::BudgetExhausted));
        }
        let Some(slot) = missing_slots
            .into_iter()
            .find(|slot| attempted_slots.insert(slot.clone()))
        else {
            return Ok(Some(SearchStopReason::RequirementsUnmet));
        };
        if !retrieve_missing_slot(engine, plan, query, state, slot, started).await? {
            return Ok(Some(SearchStopReason::BudgetExhausted));
        }
        iteration_count = iteration_count.saturating_add(1);
        let current_evidence = evidence_ids(&state.outcome);
        let no_new_evidence = current_evidence == previous_evidence;
        previous_evidence = current_evidence;
        let missing_slots = missing_required_slots(plan, &state.outcome);
        let has_unattempted_slot = missing_slots
            .iter()
            .any(|slot| !attempted_slots.contains(slot));
        if no_new_evidence && !has_unattempted_slot {
            if missing_slots.is_empty() {
                return Ok(None);
            }
            return Ok(Some(if state.outcome.evidence.is_empty() {
                SearchStopReason::NoEvidence
            } else {
                SearchStopReason::LowMarginalGain
            }));
        }
    }
}

async fn retrieve_missing_slot(
    engine: &RetrievalEngine,
    plan: &SearchPlan,
    query: &SearchQuery,
    state: &mut AdaptiveSearchState,
    slot: String,
    started: tokio::time::Instant,
) -> RetrievalResult<bool> {
    state.rewrites.set_missing_slots([slot.clone()]);
    let token_estimate = slot.split_whitespace().count().max(1);
    if !state.rewrites.add_missing_slot_rewrite(
        slot.clone(),
        slot.clone(),
        RewriteAccounting {
            token_estimate,
            latency_budget_units: 1,
            is_proposal: false,
        },
    ) {
        return Ok(false);
    }
    let query_text = state
        .rewrites
        .records()
        .iter()
        .find(|record| {
            record.origin == RewriteOrigin::MissingSlot
                && record.stage == StageRole::IterativeRetrieval
                && record.missing_slot.as_deref() == Some(slot.as_str())
        })
        .map(|record| record.query.clone())
        .ok_or_else(|| {
            RetrievalError::Internal("accepted missing-slot rewrite was not retained".to_string())
        })?;
    state.batches.extend(
        engine_pipeline::collect_missing_slot_batches(
            &engine.retrievers,
            plan,
            &query_text,
            &mut state.web_requests_used,
        )
        .await?,
    );
    (
        state.outcome,
        state.lanes,
        state.rerank_trace,
        state.diversity_trace,
    ) = engine
        .evaluate_batches(plan, query, &state.batches, started)
        .await?;
    Ok(true)
}

fn terminal_stop_reason(status: &SearchStatus) -> Option<SearchStopReason> {
    match status {
        SearchStatus::SourcesConflict => Some(SearchStopReason::RequirementsUnmet),
        SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
            Some(SearchStopReason::PolicyDenied)
        }
        SearchStatus::Abstained => Some(SearchStopReason::Abstained),
        _ => None,
    }
}

fn missing_required_slots(plan: &SearchPlan, outcome: &SearchOutcome) -> Vec<String> {
    use std::collections::BTreeSet;

    let required = plan
        .evidence_requirements
        .required_claims
        .iter()
        .chain(plan.evidence_requirements.required_subquestions.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    outcome
        .coverage
        .gaps_identified
        .iter()
        .filter(|gap| required.contains(*gap))
        .cloned()
        .collect()
}

fn evidence_ids(
    outcome: &SearchOutcome,
) -> std::collections::BTreeSet<maestria_domain::EvidenceId> {
    outcome
        .evidence
        .iter()
        .map(|candidate| candidate.evidence_id)
        .collect()
}
