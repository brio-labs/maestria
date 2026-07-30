use std::collections::BTreeMap;

use crate::golden::Metric;

use super::super::measurements::CheckStatus;
use super::super::{
    LearnedSparseQualityMetrics, LearnedSparseQueryClass, LearnedSparseResourceMetrics,
    LearnedSparseRoute, LearnedSparseRouteMetrics, LearnedSparseSafetyMetrics,
};
pub(crate) fn winning_sparse_route(
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
    metrics.budget_violations == 0
        && measured_quality(&metrics.quality)
        && measured_resources(&metrics.resources)
        && metrics.safety.provider.is_measured()
        && metrics.safety.energy.is_measured()
        && metrics.safety.acl_leakage.measured_value() == Some(&0)
        && metrics.safety.fail_open_count.measured_value() == Some(&0)
        && all_checks_pass(&metrics.safety)
}

fn measured_quality(quality: &LearnedSparseQualityMetrics) -> bool {
    [
        &quality.recall_at_5,
        &quality.recall_at_20,
        &quality.recall_at_50,
        &quality.recall_at_100,
        &quality.ndcg_at_10,
        &quality.ndcg_at_20,
        &quality.mrr_at_10,
        &quality.mean_average_precision,
        &quality.exact_span_recall,
        &quality.evidence_chain_coverage,
        &quality.source_diversity,
        &quality.source_redundancy,
        &quality.citation_precision,
        &quality.citation_recall,
        &quality.abstention_precision,
        &quality.abstention_recall,
    ]
    .iter()
    .all(|metric| metric.is_measured())
        && quality.unsupported_claim_status.is_measured()
        && quality.conflict_detection_status.is_measured()
}

fn measured_resources(resources: &LearnedSparseResourceMetrics) -> bool {
    let direct = [
        &resources.p50_latency_ms,
        &resources.p95_latency_ms,
        &resources.p99_latency_ms,
        &resources.peak_ram_bytes,
        &resources.index_disk_bytes,
    ]
    .iter()
    .all(|measurement| measurement.is_measured());
    let operations = [
        &resources.initial_indexing,
        &resources.incremental_update,
        &resources.deletion,
        &resources.rebuild,
        &resources.activation,
        &resources.rollback,
    ]
    .iter()
    .all(|operation| {
        operation.elapsed_ms.is_measured()
            && operation.throughput_items_per_second.is_measured()
            && operation.cost_micros.is_measured()
            && operation.energy_millijoules.is_measured()
    });
    direct && operations
}

fn all_checks_pass(safety: &LearnedSparseSafetyMetrics) -> bool {
    [
        &safety.namespace_isolation,
        &safety.attack_outcome,
        &safety.poisoning_outcome,
        &safety.secret_exposure,
        &safety.quarantine_outcome,
        &safety.prompt_injection_outcome,
    ]
    .iter()
    .all(|status| {
        matches!(
            status.measured_value(),
            Some(CheckStatus::Passed | CheckStatus::NotDetected)
        )
    })
}

fn wins_against(
    candidate: &LearnedSparseRouteMetrics,
    baseline: &LearnedSparseRouteMetrics,
) -> bool {
    if !measured_quality(&candidate.quality)
        || !measured_quality(&baseline.quality)
        || !measured_resources(&candidate.resources)
        || !measured_resources(&baseline.resources)
    {
        return false;
    }
    let candidate_quality = quality_values(&candidate.quality);
    let baseline_quality = quality_values(&baseline.quality);
    let no_quality_regression = candidate_quality
        .iter()
        .zip(baseline_quality.iter())
        .enumerate()
        .all(|(index, (candidate, baseline))| {
            if index == 11 {
                candidate.value() <= baseline.value()
            } else {
                candidate.value() >= baseline.value()
            }
        });
    let material_improvement = candidate_quality
        .iter()
        .zip(baseline_quality.iter())
        .enumerate()
        .any(|(index, (candidate, baseline))| {
            let delta = if index == 11 {
                baseline.value().saturating_sub(candidate.value())
            } else {
                candidate.value().saturating_sub(baseline.value())
            };
            delta >= Metric::MATERIAL_QUALITY_DELTA.value()
        });
    no_quality_regression
        && material_improvement
        && bounded_pair(
            &candidate.resources.p95_latency_ms,
            &baseline.resources.p95_latency_ms,
        )
        && bounded_pair(
            &candidate.resources.p99_latency_ms,
            &baseline.resources.p99_latency_ms,
        )
        && bounded_pair(
            &candidate.resources.peak_ram_bytes,
            &baseline.resources.peak_ram_bytes,
        )
        && bounded_pair(
            &candidate.resources.index_disk_bytes,
            &baseline.resources.index_disk_bytes,
        )
        && lifecycle_within_factor(&candidate.resources, &baseline.resources)
}

fn bounded_pair(candidate: &super::Measurement<u64>, baseline: &super::Measurement<u64>) -> bool {
    candidate
        .measured_value()
        .zip(baseline.measured_value())
        .is_some_and(|(candidate, baseline)| *candidate <= baseline.saturating_mul(2))
}

fn lifecycle_within_factor(
    candidate: &LearnedSparseResourceMetrics,
    baseline: &LearnedSparseResourceMetrics,
) -> bool {
    let operations = [
        (&candidate.initial_indexing, &baseline.initial_indexing),
        (&candidate.incremental_update, &baseline.incremental_update),
        (&candidate.deletion, &baseline.deletion),
        (&candidate.rebuild, &baseline.rebuild),
        (&candidate.activation, &baseline.activation),
        (&candidate.rollback, &baseline.rollback),
    ];
    operations.iter().all(|(candidate, baseline)| {
        let elapsed_ok = bounded_pair(&candidate.elapsed_ms, &baseline.elapsed_ms);
        let cost_ok = bounded_pair(&candidate.cost_micros, &baseline.cost_micros);
        let energy_ok = bounded_pair(&candidate.energy_millijoules, &baseline.energy_millijoules);
        let throughput_ok = candidate
            .throughput_items_per_second
            .measured_value()
            .zip(baseline.throughput_items_per_second.measured_value())
            .is_some_and(|(candidate, baseline)| candidate.saturating_mul(2) >= *baseline);
        elapsed_ok && cost_ok && energy_ok && throughput_ok
    })
}

fn quality_values(quality: &LearnedSparseQualityMetrics) -> Vec<Metric> {
    [
        &quality.recall_at_5,
        &quality.recall_at_20,
        &quality.recall_at_50,
        &quality.recall_at_100,
        &quality.ndcg_at_10,
        &quality.ndcg_at_20,
        &quality.mrr_at_10,
        &quality.mean_average_precision,
        &quality.exact_span_recall,
        &quality.evidence_chain_coverage,
        &quality.source_diversity,
        &quality.source_redundancy,
        &quality.citation_precision,
        &quality.citation_recall,
        &quality.abstention_precision,
        &quality.abstention_recall,
    ]
    .iter()
    .filter_map(|metric| metric.measured_value().copied())
    .collect()
}
