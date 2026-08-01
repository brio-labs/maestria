use maestria_domain::{
    EvidenceCandidate, SearchExecution, SearchExecutionBudget, SearchExecutionCompletion,
    SearchExecutionUsage, SearchOutcome, SearchPlan, SearchStatus,
};
use std::sync::Arc;

use crate::traits::{ContextExpander, RetrievalEvaluator};
use crate::types::{
    ContextExpansion, ExpansionPolicy, RankedCandidate, RetrievalError, RetrievalExperiment,
    RetrievalResult,
};

fn seed_expansion(
    selected_candidates: &[RankedCandidate],
    budget: SearchExecutionBudget,
    completion: SearchExecutionCompletion,
) -> ContextExpansion {
    ContextExpansion {
        candidates: selected_candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect(),
        execution: SearchExecution::new(budget, SearchExecutionUsage::default(), completion),
    }
}

fn expansion_policy(
    plan: &SearchPlan,
    initial: &crate::diversity::DiversitySelection,
    selected_candidates: &[RankedCandidate],
    remaining_candidates: u64,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    expansion_budget: Option<SearchExecutionBudget>,
    fallback_budget: SearchExecutionBudget,
) -> ExpansionPolicy {
    ExpansionPolicy {
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
        execution_budget: match expansion_budget {
            Some(budget) => budget,
            None => fallback_budget,
        },
    }
}

fn expand_context(
    selected_candidates: &[RankedCandidate],
    expander: &Option<Arc<dyn ContextExpander>>,
    policy: &ExpansionPolicy,
    expansion_budget: Option<SearchExecutionBudget>,
    fallback_budget: SearchExecutionBudget,
) -> RetrievalResult<ContextExpansion> {
    match (expander, expansion_budget) {
        (Some(expander), Some(_)) => expander.expand(selected_candidates, policy),
        (Some(_), None) => Ok(seed_expansion(
            selected_candidates,
            fallback_budget,
            SearchExecutionCompletion::Exhausted(
                maestria_domain::SearchExecutionResource::Candidates,
            ),
        )),
        (None, _) => Ok(seed_expansion(
            selected_candidates,
            match expansion_budget {
                Some(budget) => budget,
                None => fallback_budget,
            },
            SearchExecutionCompletion::Complete,
        )),
    }
}

pub(crate) async fn run_diversity_stage(
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
    let expansion_budget = super::remaining_budget(plan, *execution_usage);
    let policy = expansion_policy(
        plan,
        &initial,
        &selected_candidates,
        remaining_candidates,
        authorization,
        expansion_budget,
        budget,
    );
    let ContextExpansion {
        candidates: expanded,
        execution,
    } = expand_context(
        &selected_candidates,
        expander,
        &policy,
        expansion_budget,
        budget,
    )?;
    super::add_usage(execution_usage, execution.usage);
    let mut expansion_budget_exhausted =
        !matches!(execution.completion, SearchExecutionCompletion::Complete);
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
            if !super::usage_within_budget(candidate_usage, budget) {
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

/// Reconciles an evaluator status with the diversity selector status.
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
