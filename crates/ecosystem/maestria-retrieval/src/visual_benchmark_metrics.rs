use super::{
    VisualBenchmarkCase, VisualBenchmarkError, VisualBenchmarkObservation, VisualRoute,
    VisualRouteMetrics,
};
use crate::golden::Metric;

fn average(metrics: impl Iterator<Item = Metric>, count: usize) -> Metric {
    let total = metrics.fold(0usize, |total, metric| {
        total.saturating_add(metric.value() as usize)
    });
    Metric::from_ratio(total, count.saturating_mul(Metric::ONE.value() as usize))
}

pub(super) fn metrics_for(
    class: super::VisualQueryClass,
    route: VisualRoute,
    cases: &[&VisualBenchmarkCase],
    observations: &[VisualBenchmarkObservation],
) -> Result<VisualRouteMetrics, VisualBenchmarkError> {
    let mut selected = Vec::with_capacity(cases.len());
    for case in cases {
        let observation = observations
            .iter()
            .find(|observation| observation.case_id == case.case_id && observation.route == route)
            .ok_or_else(|| VisualBenchmarkError::MissingObservation {
                case_id: case.case_id.clone(),
                route,
            })?;
        selected.push(observation);
    }
    if selected.is_empty() {
        return Err(VisualBenchmarkError::MissingClass(class));
    }
    let mut latencies = selected
        .iter()
        .map(|observation| observation.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let p95_index = ((latencies.len() * 95).div_ceil(100)).saturating_sub(1);
    Ok(VisualRouteMetrics {
        page_region_recall: average(
            selected
                .iter()
                .map(|observation| observation.page_region_recall),
            selected.len(),
        ),
        ndcg_at_10: average(
            selected.iter().map(|observation| observation.ndcg_at_10),
            selected.len(),
        ),
        citation_alignment: average(
            selected
                .iter()
                .map(|observation| observation.citation_alignment),
            selected.len(),
        ),
        p95_latency_ms: latencies[p95_index],
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
        energy_millijoules: selected.iter().fold(0_u64, |total, observation| {
            total.saturating_add(observation.energy_millijoules)
        }),
        privacy_violations: selected.iter().fold(0_u32, |total, observation| {
            total.saturating_add(observation.privacy_violations)
        }),
        security_violations: selected.iter().fold(0_u32, |total, observation| {
            total.saturating_add(observation.security_violations)
        }),
    })
}

pub(super) fn wins(
    cases: &[&VisualBenchmarkCase],
    text_layout: &VisualRouteMetrics,
    visual: &VisualRouteMetrics,
    observations: &[VisualBenchmarkObservation],
) -> bool {
    let quality_gain = visual.page_region_recall.value()
        >= text_layout
            .page_region_recall
            .value()
            .saturating_add(Metric::MATERIAL_QUALITY_DELTA.value())
        || visual.ndcg_at_10.value()
            >= text_layout
                .ndcg_at_10
                .value()
                .saturating_add(Metric::MATERIAL_QUALITY_DELTA.value());
    let citation_safe = visual.citation_alignment.value() >= text_layout.citation_alignment.value();
    let resource_safe = cases.iter().all(|case| {
        observations
            .iter()
            .find(|observation| {
                observation.case_id == case.case_id && observation.route == VisualRoute::Visual
            })
            .is_some_and(|observation| {
                observation.provider_status.is_available()
                    && observation.latency_ms <= case.latency_budget_ms
                    && observation.memory_bytes <= case.memory_budget_bytes
                    && observation.disk_bytes <= case.disk_budget_bytes
                    && observation.energy_millijoules <= case.energy_budget_millijoules
                    && observation.privacy_violations == 0
                    && observation.security_violations == 0
            })
    });
    quality_gain && citation_safe && resource_safe
}
