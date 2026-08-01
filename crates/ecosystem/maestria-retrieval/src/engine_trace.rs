use maestria_domain::{
    CorpusScope, FreshnessRequirement, SearchOutcome, SearchPlan, SearchStatus, SearchStopReason,
    SearchTrace, SearchTraceExpansion, SearchTraceFilter,
};

/// Serializes the engine-owned retrieval security policy into the provenance format
/// consumed by the retrieval security validator.
pub fn security_policy_fingerprint(
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> String {
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
    if matches!(plan.scope, CorpusScope::Restricted(_)) || policy.required_scope_id.is_some() {
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
    if !matches!(plan.freshness, FreshnessRequirement::Any) {
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
    /// Human-readable degradation message when the visual provider is unavailable.
    degradation: Option<String>,
    /// Name of the unavailable capability (e.g. `"visual provider"`).
    unavailable_capability: Option<String>,
}

/// Computes the expected trace state from the plan, outcome, lanes, and options.
///
/// Stop reason is resolved in priority order:
/// 1. Explicit override in `options.explicit_stop_reason`.
/// 2. Terminal outcome status (`DeniedByPolicy`, `Abstained`, `NoEvidenceFound`, etc.).
/// 3. Diversity trace stop reason when present.
/// 4. Evidence count against `plan.stop_conditions.max_results`.
///
/// Fusion and expansions are toggled by `options`. Degradation fields are set
/// when the visual provider is unreachable or the visual lane failed.
fn compute_expected_trace_state(
    plan: &SearchPlan,
    outcome: &SearchOutcome,
    lanes: &[maestria_domain::SearchTraceLane],
    options: &EnsureTraceOptions,
) -> ExpectedTraceState {
    let expected_policy_fingerprint = match plan.authorization.as_ref() {
        Some(authorization) => authorization.canonical_fingerprint(),
        None => security_policy_fingerprint(&options.security_policy),
    };
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
    let expected_fusion = options.fusion_enabled.then_some("configured".to_string());
    let expected_expansions = options
        .expansion_enabled
        .then_some(SearchTraceExpansion {
            strategy: "hierarchy+graph".to_string(),
            added_candidates: None,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let visual_plan_fallback = plan.intent == maestria_domain::SearchIntent::FactualLocal
        && maestria_domain::SearchIntent::classify(&plan.original_query)
            == maestria_domain::SearchIntent::VisualDocument;
    let visual_lane_failed = lanes.iter().any(|lane| {
        lane.retriever_id == "visual_page_regions"
            && matches!(
                lane.status,
                maestria_domain::SearchLaneStatus::Failed { .. }
            )
    });
    let expected_degradation = (visual_plan_fallback || visual_lane_failed)
        .then(|| "visual provider unavailable; using text/layout retrieval".to_string());
    let expected_unavailable_capability =
        (visual_plan_fallback || visual_lane_failed).then(|| "visual provider".to_string());

    ExpectedTraceState {
        policy_fingerprint: expected_policy_fingerprint,
        filters: expected_filters,
        stop_reason: expected_stop_reason,
        fusion: expected_fusion,
        expansions: expected_expansions,
        degradation: expected_degradation,
        unavailable_capability: expected_unavailable_capability,
    }
}

/// Checks whether an existing `SearchTrace` matches the computed expectations.
///
/// Performs a 13-condition equality check covering:
/// deterministic ID, plan match, degradation, unavailable capability, retrievers,
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
        && trace.unavailable_capability == expected.unavailable_capability
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
) -> SearchTrace {
    SearchTrace::from_plan(
        plan,
        lanes.iter().map(|lane| lane.retriever_id.clone()).collect(),
        &outcome.evidence,
        expected.filters.clone(),
        expected.fusion.clone(),
        expected.expansions.clone(),
        expected.stop_reason.clone(),
    )
    .with_policy_fingerprint(expected.policy_fingerprint.clone())
    .with_lanes(lanes)
    .with_gaps_and_conflicts(
        outcome.coverage.gaps_identified.clone(),
        outcome
            .conflicts
            .iter()
            .map(|conflict| conflict.id)
            .collect(),
    )
}

/// Rebuilds the outcome trace so it matches the governed search context.
pub fn ensure_trace(
    plan: &SearchPlan,
    mut outcome: SearchOutcome,
    lanes: Vec<maestria_domain::SearchTraceLane>,
    options: EnsureTraceOptions,
) -> SearchOutcome {
    let expected = compute_expected_trace_state(plan, &outcome, &lanes, &options);
    let trace_is_valid = outcome.trace_data.as_ref().is_some_and(|trace| {
        trace_matches_expected(trace, plan, &outcome, &lanes, &expected, &options)
    });
    if trace_is_valid {
        return outcome;
    }
    let mut trace = assemble_trace(plan, &outcome, lanes, &expected);
    trace = apply_degradation(trace, expected.degradation, expected.unavailable_capability);
    trace.rewrites = options.rewrites;
    trace.rerank = options.rerank_trace;
    trace.diversity = options.diversity_trace;
    outcome.trace = trace.deterministic_id();
    outcome.trace_data = Some(Box::new(trace));
    outcome
}

fn apply_degradation(
    trace: SearchTrace,
    degradation: Option<String>,
    unavailable_capability: Option<String>,
) -> SearchTrace {
    let trace = match degradation {
        Some(value) => trace.with_degradation(value),
        None => trace,
    };
    match unavailable_capability {
        Some(value) => trace.with_unavailable_capability(value),
        None => trace,
    }
}
