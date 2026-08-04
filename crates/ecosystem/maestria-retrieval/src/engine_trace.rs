use maestria_domain::{
    CorpusScope, Modality, SearchExpansionStrategy, SearchOutcome, SearchPlan, SearchStatus,
    SearchStopReason, SearchTrace, SearchTraceExpansion, SearchTraceFilter,
};

use crate::types::RetrievalError;

/// Serializes the engine-owned retrieval security policy into the provenance
/// format consumed by the retrieval security validator.
pub fn security_policy_fingerprint(
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> Result<String, maestria_governance::RetrievalAuthorizationError> {
    policy.canonical_fingerprint()
}

/// Lists every security filter enabled for a governed search trace.
pub fn applied_security_filters(
    plan: &SearchPlan,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> Vec<SearchTraceFilter> {
    let mut filters = vec![
        SearchTraceFilter::Quarantine,
        SearchTraceFilter::PromptInjection,
    ];
    if matches!(plan.scope(), CorpusScope::Restricted(_)) || policy.required_scope_id.is_some() {
        filters.push(SearchTraceFilter::Scope);
    }
    if policy.require_read_allowed {
        filters.push(SearchTraceFilter::Acl);
    }
    if policy.require_trust_zone.is_some() {
        filters.push(SearchTraceFilter::Trust);
    }
    if policy.max_sensitivity.is_some() {
        filters.push(SearchTraceFilter::Sensitivity);
    }
    // The freshness filter is enforced on repository-code lanes: stale-code
    // candidates are dropped before fusion, and the code-intel adapter
    // rejects stale indexes before scoring. Code plans are the only plans
    // that carry `FreshnessRequirement::Any` while still running this filter;
    // web plans declare `Realtime` but no web lane is wired, so claiming
    // `Freshness` for them would record a filter that never executed (R46).
    if plan.modalities().values().contains(&Modality::Code) {
        filters.push(SearchTraceFilter::Freshness);
    }
    filters
}

/// Options controlling governed search-trace construction.
pub struct EnsureTraceOptions {
    pub(crate) security_policy: maestria_governance::RetrievalSecurityPolicy,
    pub(crate) fusion_enabled: bool,
    pub(crate) expansion_enabled: bool,
    pub(crate) rerank_trace: Option<maestria_domain::SearchTraceRerank>,
    pub(crate) diversity_trace: Option<maestria_domain::SearchTraceDiversity>,
    pub(crate) rewrites: Vec<maestria_domain::SearchTraceRewrite>,
    pub(crate) explicit_stop_reason: Option<SearchStopReason>,
}

/// Snapshot of what a `SearchTrace` should contain for the current search context.
///
/// Derived from the plan, outcome, lanes, and `EnsureTraceOptions`. Every field
/// is mirrored by a corresponding check in [`trace_matches_expected`].
struct ExpectedTraceState {
    /// Canonical fingerprint of the engine-owned retrieval security policy.
    policy_fingerprint: String,
    /// Security filters applied by the engine-owned retrieval policy.
    filters: Vec<SearchTraceFilter>,
    /// Stop reason derived from the outcome status, explicit override, or result count.
    stop_reason: SearchStopReason,
    /// Fusion marker when fusion is enabled (`"configured"`).
    fusion: Option<String>,
    /// Expansions when expansion is enabled.
    expansions: Vec<SearchTraceExpansion>,
    /// Named capability degradation when the visual provider is unavailable.
    degradation: Option<maestria_domain::SearchDegradation>,
}

/// Computes the expected trace state from the plan, outcome, lanes, and options.
///
/// Stop reason is resolved in priority order:
/// 1. Explicit override in `options.explicit_stop_reason`.
/// 2. Terminal outcome status (`DeniedByPolicy`, `Abstained`, `NoEvidenceFound`, etc.).
/// 3. Diversity trace stop reason when present.
/// 4. Evidence count against `plan.stop_conditions().max_results`.
///
/// Fusion and expansions are toggled by `options`. Degradation fields are set
/// when the visual provider is unreachable or the visual lane failed.
fn compute_expected_trace_state(
    plan: &SearchPlan,
    outcome: &SearchOutcome,
    lanes: &[maestria_domain::SearchTraceLane],
    options: &EnsureTraceOptions,
) -> ExpectedTraceState {
    let expected_policy_fingerprint = plan.authorization().canonical_fingerprint();
    let expected_filters = applied_security_filters(plan, &options.security_policy);
    let expected_stop_reason = match options.explicit_stop_reason.clone() {
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
            _ => options.diversity_trace.as_ref().map_or_else(
                || {
                    if outcome.evidence.len() >= plan.stop_conditions().max_results as usize {
                        SearchStopReason::ResultsLimit
                    } else {
                        SearchStopReason::EvidenceComplete
                    }
                },
                |trace| trace.stop_reason.clone(),
            ),
        },
    };
    let expected_fusion = options.fusion_enabled.then_some("configured".to_string());
    let expected_expansions = options
        .expansion_enabled
        .then_some(SearchTraceExpansion::new(
            SearchExpansionStrategy::HierarchyGraph,
            None,
        ))
        .into_iter()
        .collect::<Vec<_>>();
    let visual_plan_fallback = plan.intent() == maestria_domain::SearchIntent::FactualLocal
        && maestria_domain::SearchIntent::classify(plan.original_query())
            == maestria_domain::SearchIntent::VisualDocument;
    let visual_lane_failed = lanes.iter().any(|lane| {
        lane.retriever_id == "visual_page_regions"
            && matches!(
                lane.status,
                maestria_domain::SearchLaneStatus::Failed { .. }
            )
    });
    let expected_degradation =
        (visual_plan_fallback || visual_lane_failed).then(|| maestria_domain::SearchDegradation {
            capability: "visual provider".to_string(),
            reason: "visual provider unavailable; using text/layout retrieval".to_string(),
        });

    ExpectedTraceState {
        policy_fingerprint: expected_policy_fingerprint,
        filters: expected_filters,
        stop_reason: expected_stop_reason,
        fusion: expected_fusion,
        expansions: expected_expansions,
        degradation: expected_degradation,
    }
}

/// Checks whether an existing `SearchTrace` matches the computed expectations.
///
/// Performs a 13-condition equality check covering:
/// deterministic ID, plan match, degradation, retrievers,
/// lanes, fusion, rerank, diversity, expansions, rewrites, stop reason, and
/// evidence alignment.
fn trace_matches_expected(
    trace: &SearchTrace,
    plan: &SearchPlan,
    outcome: &SearchOutcome,
    lanes: &[maestria_domain::SearchTraceLane],
    expected: &ExpectedTraceState,
    options: &EnsureTraceOptions,
) -> bool {
    outcome.trace == trace.deterministic_id()
        && trace.matches_plan(plan)
        && trace.policy_fingerprint.as_deref() == Some(expected.policy_fingerprint.as_str())
        && trace.filters == expected.filters
        && trace.degradation == expected.degradation
        && trace.retrievers
            == lanes
                .iter()
                .map(|lane| lane.retriever_id.clone())
                .collect::<Vec<_>>()
        && trace.lanes == lanes
        && trace.fusion == expected.fusion
        && trace.rerank == options.rerank_trace
        && trace.diversity == options.diversity_trace
        && trace.expansions == expected.expansions
        && trace.rewrites == options.rewrites
        && trace.stop_reason == expected.stop_reason
        && trace.matches_evidence(&outcome.evidence)
}

/// Builds a fresh `SearchTrace` when the existing trace is stale or absent.
///
/// Called by [`ensure_trace`] only when [`trace_matches_expected`] returns
/// `false`. When the trace is already valid the existing trace is preserved
/// unchanged.
fn assemble_trace(
    plan: &SearchPlan,
    outcome: &SearchOutcome,
    lanes: Vec<maestria_domain::SearchTraceLane>,
    expected: &ExpectedTraceState,
) -> Result<SearchTrace, RetrievalError> {
    Ok(SearchTrace::from_plan(
        plan,
        lanes.iter().map(|lane| lane.retriever_id.clone()).collect(),
        &outcome.evidence,
        expected.filters.clone(),
        expected.fusion.clone(),
        expected.expansions.clone(),
        expected.stop_reason.clone(),
    )?
    .with_policy_fingerprint(expected.policy_fingerprint.clone())
    .with_lanes(lanes)
    .with_gaps_and_conflicts(
        outcome.coverage.gaps_identified().to_vec(),
        outcome
            .conflicts
            .iter()
            .map(|conflict| conflict.id)
            .collect(),
    ))
}

/// Rebuilds the outcome trace so it matches the governed search context.
pub fn ensure_trace(
    plan: &SearchPlan,
    mut outcome: SearchOutcome,
    lanes: Vec<maestria_domain::SearchTraceLane>,
    options: EnsureTraceOptions,
) -> Result<SearchOutcome, RetrievalError> {
    let expected = compute_expected_trace_state(plan, &outcome, &lanes, &options);
    let trace_is_valid = outcome.trace_data.as_ref().is_some_and(|trace| {
        trace_matches_expected(trace, plan, &outcome, &lanes, &expected, &options)
    });
    if trace_is_valid {
        return Ok(outcome);
    }
    let mut trace = assemble_trace(plan, &outcome, lanes, &expected)?;
    trace = apply_degradation(trace, expected.degradation);
    trace.rewrites = options.rewrites;
    trace.rerank = options.rerank_trace;
    trace.diversity = options.diversity_trace;
    outcome.trace = trace.deterministic_id();
    outcome.trace_data = Some(Box::new(trace));
    Ok(outcome)
}

fn apply_degradation(
    trace: SearchTrace,
    degradation: Option<maestria_domain::SearchDegradation>,
) -> SearchTrace {
    match degradation {
        Some(value) => trace.with_degradation(value),
        None => trace,
    }
}
