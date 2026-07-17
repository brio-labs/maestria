use maestria_domain::{
    SearchOutcome, SearchPlan, SearchStatus, SearchStopReason, SearchTrace, SearchTraceExpansion,
};

pub(crate) struct EnsureTraceOptions {
    pub(crate) fusion_enabled: bool,
    pub(crate) expansion_enabled: bool,
    pub(crate) rerank_trace: Option<maestria_domain::SearchTraceRerank>,
    pub(crate) diversity_trace: Option<maestria_domain::SearchTraceDiversity>,
    pub(crate) rewrites: Vec<maestria_domain::SearchTraceRewrite>,
    pub(crate) explicit_stop_reason: Option<SearchStopReason>,
}

pub(crate) fn ensure_trace(
    plan: &SearchPlan,
    mut outcome: SearchOutcome,
    lanes: Vec<maestria_domain::SearchTraceLane>,
    options: EnsureTraceOptions,
) -> SearchOutcome {
    let EnsureTraceOptions {
        fusion_enabled,
        expansion_enabled,
        rerank_trace,
        diversity_trace,
        rewrites,
        explicit_stop_reason,
    } = options;
    let expected_stop_reason = match explicit_stop_reason.clone() {
        Some(stop_reason) => stop_reason,
        None => match &outcome.status {
            SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
                SearchStopReason::PolicyDenied
            }
            SearchStatus::Abstained => SearchStopReason::Abstained,
            SearchStatus::NoEvidenceFound => SearchStopReason::NoEvidence,
            SearchStatus::SourcesConflict
            | SearchStatus::EvidenceIncomplete
            | SearchStatus::StaleEvidenceOnly => SearchStopReason::RequirementsUnmet,
            _ => diversity_trace.as_ref().map_or_else(
                || {
                    if outcome.evidence.len() >= plan.stop_conditions.max_results as usize {
                        SearchStopReason::ResultsLimit
                    } else {
                        SearchStopReason::EvidenceComplete
                    }
                },
                |trace| trace.stop_reason.clone(),
            ),
        },
    };
    let expected_fusion = fusion_enabled.then_some("configured".to_string());
    let expected_expansions = expansion_enabled
        .then_some(SearchTraceExpansion {
            strategy: "configured".to_string(),
            added_candidates: None,
        })
        .into_iter()
        .collect::<Vec<_>>();

    let trace_is_valid = outcome.trace_data.as_ref().is_some_and(|trace| {
        outcome.trace == trace.deterministic_id()
            && trace.matches_plan(plan)
            && trace.retrievers
                == lanes
                    .iter()
                    .map(|lane| lane.retriever_id.clone())
                    .collect::<Vec<_>>()
            && trace.lanes == lanes
            && trace.fusion == expected_fusion
            && trace.rerank == rerank_trace
            && trace.diversity == diversity_trace
            && trace.expansions == expected_expansions
            && trace.rewrites == rewrites
            && trace.stop_reason == expected_stop_reason
            && trace.matches_evidence(&outcome.evidence)
    });
    if trace_is_valid {
        return outcome;
    }
    let stop_reason = expected_stop_reason;
    let expansions = expected_expansions;
    let mut trace = SearchTrace::from_plan(
        plan,
        lanes.iter().map(|lane| lane.retriever_id.clone()).collect(),
        &outcome.evidence,
        Vec::new(),
        expected_fusion,
        expansions,
        stop_reason,
    )
    .with_lanes(lanes)
    .with_gaps_and_conflicts(
        outcome.coverage.gaps_identified.clone(),
        outcome
            .conflicts
            .iter()
            .map(|conflict| conflict.id)
            .collect(),
    );
    trace.rewrites = rewrites;
    trace.rerank = rerank_trace;
    trace.diversity = diversity_trace;
    outcome.trace = trace.deterministic_id();
    outcome.trace_data = Some(Box::new(trace));
    outcome
}
