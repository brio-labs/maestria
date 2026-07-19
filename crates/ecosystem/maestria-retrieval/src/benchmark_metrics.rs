use super::{
    RepositoryBenchmarkCase, RepositoryBenchmarkError, RepositoryBenchmarkObservation,
    RepositoryExpectedOutcome, RepositoryQueryClass, RepositoryRoute, RepositoryRouteMetrics,
};
use crate::golden::Metric;

pub(super) fn metrics_for(
    class: RepositoryQueryClass,
    route: RepositoryRoute,
    cases: &[&RepositoryBenchmarkCase],
    observations: &[RepositoryBenchmarkObservation],
) -> Result<RepositoryRouteMetrics, RepositoryBenchmarkError> {
    let mut exact_hits = 0usize;
    let mut exact_expected = 0usize;
    let mut chain_hits = 0usize;
    let mut chain_expected = 0usize;
    let mut outcomes = 0usize;
    let mut abstentions = 0usize;
    let mut abstention_expected = 0usize;
    let mut latencies = Vec::with_capacity(cases.len());
    let mut freshness_errors = 0u32;
    let mut peak_memory_bytes = 0_u64;
    let mut privacy_violations = 0_u32;
    let mut security_violations = 0_u32;
    let mut energy_milliwatt_seconds = 0_u64;
    for case in cases {
        let observation = observations
            .iter()
            .find(|observation| observation.case_id == case.case_id && observation.route == route)
            .ok_or_else(|| RepositoryBenchmarkError::MissingObservation {
                case_id: case.case_id.clone(),
                route,
            })?;
        exact_hits += observation
            .exact_span_hits
            .min(case.expected.exact_span_count());
        exact_expected += case.expected.exact_span_count();
        chain_hits += observation
            .evidence_chain_length
            .min(case.expected.evidence_chain_length());
        chain_expected += case.expected.evidence_chain_length();
        outcomes += usize::from(observation.outcome_correct);
        let expected_abstain = matches!(&case.expected, RepositoryExpectedOutcome::Abstain);
        abstention_expected += 1;
        abstentions += usize::from(observation.abstained == expected_abstain);
        latencies.push(observation.latency_ms);
        freshness_errors += u32::from(observation.freshness_error);
        peak_memory_bytes = peak_memory_bytes.max(observation.memory_bytes);
        privacy_violations += u32::from(observation.privacy_violation);
        security_violations += u32::from(observation.security_violation);
        energy_milliwatt_seconds =
            energy_milliwatt_seconds.saturating_add(observation.energy_milliwatt_seconds);
    }
    if cases.is_empty() {
        return Err(RepositoryBenchmarkError::MissingClass(class));
    }
    latencies.sort_unstable();
    let p95_index = ((latencies.len() * 95).div_ceil(100)).saturating_sub(1);
    Ok(RepositoryRouteMetrics {
        exact_span_recall: Metric::from_ratio(exact_hits, exact_expected),
        peak_memory_bytes,
        privacy_violations,
        security_violations,
        energy_milliwatt_seconds,
        evidence_chain_accuracy: Metric::from_ratio(chain_hits, chain_expected),
        outcome_accuracy: Metric::from_ratio(outcomes, cases.len()),
        abstention_accuracy: Metric::from_ratio(abstentions, abstention_expected),
        p95_latency_ms: latencies[p95_index],
        freshness_errors,
    })
}

pub(super) fn wins(
    cases: &[&RepositoryBenchmarkCase],
    phase_c: &RepositoryRouteMetrics,
    specialized: &RepositoryRouteMetrics,
) -> bool {
    let quality_gain = specialized.exact_span_recall.value()
        >= phase_c
            .exact_span_recall
            .value()
            .saturating_add(Metric::MATERIAL_QUALITY_DELTA.value())
        || specialized.evidence_chain_accuracy.value()
            >= phase_c
                .evidence_chain_accuracy
                .value()
                .saturating_add(Metric::MATERIAL_QUALITY_DELTA.value())
        || specialized.outcome_accuracy.value()
            >= phase_c
                .outcome_accuracy
                .value()
                .saturating_add(Metric::MATERIAL_QUALITY_DELTA.value());
    let latency_not_slower = specialized.p95_latency_ms <= phase_c.p95_latency_ms;
    let latency_within_budget = cases
        .iter()
        .all(|case| specialized.p95_latency_ms <= case.latency_budget_ms);
    let abstention_safe =
        specialized.abstention_accuracy.value() >= phase_c.abstention_accuracy.value();
    let resource_safe = specialized.peak_memory_bytes <= phase_c.peak_memory_bytes
        && specialized.privacy_violations <= phase_c.privacy_violations
        && specialized.security_violations <= phase_c.security_violations
        && specialized.energy_milliwatt_seconds <= phase_c.energy_milliwatt_seconds;
    quality_gain
        && specialized.freshness_errors <= phase_c.freshness_errors
        && latency_not_slower
        && latency_within_budget
        && abstention_safe
        && resource_safe
}
