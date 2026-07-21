use std::collections::BTreeMap;

use crate::golden::Metric;

use super::{
    LearnedSparseBenchmarkCase, LearnedSparseBenchmarkError, LearnedSparseBenchmarkObservation,
    LearnedSparseQueryClass, LearnedSparseRoute, LearnedSparseRouteMetrics,
};

pub(super) fn aggregate(
    cases: &[&LearnedSparseBenchmarkCase],
    route: LearnedSparseRoute,
    observations: &[LearnedSparseBenchmarkObservation],
) -> Result<LearnedSparseRouteMetrics, LearnedSparseBenchmarkError> {
    let selected = cases
        .iter()
        .map(|case| {
            observations
                .iter()
                .find(|observation| {
                    observation.case_id == case.case_id && observation.route == route
                })
                .ok_or_else(|| LearnedSparseBenchmarkError::MissingObservation {
                    case_id: case.case_id.clone(),
                    route,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = selected.len().max(1) as u64;
    let mean_metric = |values: Vec<u32>| {
        let value = values
            .into_iter()
            .map(u64::from)
            .fold(0_u64, u64::saturating_add)
            / count;
        Metric::new(value.min(u64::from(u32::MAX)) as u32).map_or(Metric::ZERO, |metric| metric)
    };
    let mut latencies = selected
        .iter()
        .map(|observation| observation.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let p95_index = (latencies.len() * 95).div_ceil(100).saturating_sub(1);
    let p95_latency_ms = latencies.get(p95_index).copied().map_or(0, |value| value);
    Ok(LearnedSparseRouteMetrics {
        recall_at_20: mean_metric(
            selected
                .iter()
                .map(|observation| observation.recall_at_20.value())
                .collect(),
        ),
        ndcg_at_10: mean_metric(
            selected
                .iter()
                .map(|observation| observation.ndcg_at_10.value())
                .collect(),
        ),
        mrr_at_10: mean_metric(
            selected
                .iter()
                .map(|observation| observation.mrr_at_10.value())
                .collect(),
        ),
        exact_span_recall: mean_metric(
            selected
                .iter()
                .map(|observation| observation.exact_span_recall.value())
                .collect(),
        ),
        p95_latency_ms,
        peak_memory_bytes: selected
            .iter()
            .map(|observation| observation.memory_bytes)
            .max()
            .map_or(0, |value| value),
        peak_disk_bytes: selected
            .iter()
            .map(|observation| observation.disk_bytes)
            .max()
            .map_or(0, |value| value),
        total_ingest_update_ms: selected
            .iter()
            .map(|observation| observation.ingest_update_ms)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.into_iter().fold(0_u64, u64::saturating_add)),
        total_energy_millijoules: selected
            .iter()
            .map(|observation| observation.energy_millijoules)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.into_iter().fold(0_u64, u64::saturating_add)),
        privacy_violations: selected
            .iter()
            .map(|observation| observation.privacy_violations)
            .fold(0_u32, u32::saturating_add),
        security_violations: selected
            .iter()
            .map(|observation| observation.security_violations)
            .fold(0_u32, u32::saturating_add),
        budget_violations: selected
            .iter()
            .zip(cases.iter())
            .filter(|(observation, case)| exceeds_budget(observation, case))
            .count()
            .min(u32::MAX as usize) as u32,
    })
}

pub(super) fn winning_sparse_route(
    class: LearnedSparseQueryClass,
    routes: &BTreeMap<LearnedSparseRoute, LearnedSparseRouteMetrics>,
) -> Option<LearnedSparseRoute> {
    if matches!(
        class,
        LearnedSparseQueryClass::ExactLiteral
            | LearnedSparseQueryClass::NoEvidence
            | LearnedSparseQueryClass::Security
    ) {
        return None;
    }
    let lexical = routes.get(&LearnedSparseRoute::Lexical)?;
    let hybrid = routes.get(&LearnedSparseRoute::Hybrid)?;
    let candidate = routes.get(&LearnedSparseRoute::SparseFused)?;
    (telemetry_complete(candidate)
        && wins_against(candidate, lexical)
        && wins_against(candidate, hybrid))
    .then_some(LearnedSparseRoute::SparseFused)
}

fn telemetry_complete(metrics: &LearnedSparseRouteMetrics) -> bool {
    metrics.total_ingest_update_ms.is_some()
        && metrics.total_energy_millijoules.is_some()
        && metrics.privacy_violations == 0
        && metrics.security_violations == 0
        && metrics.budget_violations == 0
}

fn exceeds_budget(
    observation: &LearnedSparseBenchmarkObservation,
    case: &&LearnedSparseBenchmarkCase,
) -> bool {
    observation.latency_ms > case.latency_budget_ms
        || observation.memory_bytes > case.memory_budget_bytes
        || observation.disk_bytes > case.disk_budget_bytes
        || observation
            .ingest_update_ms
            .is_some_and(|value| value > case.ingest_update_budget_ms)
        || observation
            .energy_millijoules
            .is_some_and(|value| value > case.energy_budget_millijoules)
}

fn wins_against(
    candidate: &LearnedSparseRouteMetrics,
    baseline: &LearnedSparseRouteMetrics,
) -> bool {
    let qualities = [
        (candidate.recall_at_20, baseline.recall_at_20),
        (candidate.ndcg_at_10, baseline.ndcg_at_10),
        (candidate.mrr_at_10, baseline.mrr_at_10),
        (candidate.exact_span_recall, baseline.exact_span_recall),
    ];
    let no_quality_regression = qualities
        .iter()
        .all(|(candidate, baseline)| candidate >= baseline);
    let material_improvement = qualities.iter().any(|(candidate, baseline)| {
        candidate.value().saturating_sub(baseline.value()) >= Metric::MATERIAL_QUALITY_DELTA.value()
    });
    no_quality_regression
        && material_improvement
        && candidate.p95_latency_ms <= baseline.p95_latency_ms.saturating_mul(2)
        && candidate.peak_memory_bytes <= baseline.peak_memory_bytes.saturating_mul(2)
        && candidate.peak_disk_bytes <= baseline.peak_disk_bytes.saturating_mul(2)
}
