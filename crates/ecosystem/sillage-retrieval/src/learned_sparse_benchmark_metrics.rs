#[path = "learned_sparse_benchmark_gate.rs"]
mod gate;
pub(super) use gate::{hybrid_serving_eligible, winning_sparse_route};

use super::measurements::{CheckStatus, LearnedSparseOperationMeasurement, Measurement};
use super::{
    LearnedSparseBenchmarkCase, LearnedSparseBenchmarkError, LearnedSparseBenchmarkObservation,
    LearnedSparseResourceMetrics, LearnedSparseRoute, LearnedSparseRouteMetrics,
};
use crate::golden::Metric;

fn select_observations<'a>(
    cases: &[&'a LearnedSparseBenchmarkCase],
    route: LearnedSparseRoute,
    observations: &'a [LearnedSparseBenchmarkObservation],
) -> Result<Vec<&'a LearnedSparseBenchmarkObservation>, LearnedSparseBenchmarkError> {
    cases
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
        .collect()
}

pub(super) fn aggregate_metric(values: Vec<&Measurement<Metric>>) -> Measurement<Metric> {
    if values.iter().all(|value| value.is_measured()) {
        let sum = values
            .iter()
            .filter_map(|value| value.measured_value())
            .map(|value| u64::from(value.value()))
            .fold(0_u64, u64::saturating_add);
        let mean = sum / values.len().max(1) as u64;
        return Metric::new(mean.min(u64::from(u32::MAX)) as u32).map_or_else(
            || Measurement::unavailable("metric mean is out of range"),
            Measurement::measured,
        );
    }
    state_from_measurements(&values)
}

pub(super) fn aggregate_u32(values: Vec<&Measurement<u32>>) -> Measurement<u32> {
    if values.iter().all(|value| value.is_measured()) {
        return Measurement::measured(
            values
                .iter()
                .filter_map(|value| value.measured_value())
                .copied()
                .fold(0_u32, u32::saturating_add),
        );
    }
    state_from_measurements(&values)
}
fn aggregate_u64(values: Vec<&Measurement<u64>>) -> Measurement<u64> {
    if values.iter().all(|value| value.is_measured()) {
        return Measurement::measured(
            values
                .iter()
                .filter_map(|value| value.measured_value())
                .copied()
                .fold(0_u64, u64::saturating_add)
                / values.len().max(1) as u64,
        );
    }
    state_from_measurements(&values)
}

pub(super) fn aggregate_percentile(
    values: Vec<&Measurement<u64>>,
    percentile: usize,
) -> Measurement<u64> {
    if values.iter().all(|value| value.is_measured()) {
        let mut measured = values
            .iter()
            .filter_map(|value| value.measured_value())
            .copied()
            .collect::<Vec<_>>();
        measured.sort_unstable();
        let index = ((measured.len() * percentile).div_ceil(100)).saturating_sub(1);
        return measured.get(index).copied().map_or_else(
            || Measurement::unavailable("percentile has no samples"),
            Measurement::measured,
        );
    }
    state_from_measurements(&values)
}

pub(super) fn aggregate_max(values: Vec<&Measurement<u64>>) -> Measurement<u64> {
    if values.iter().all(|value| value.is_measured()) {
        return values
            .iter()
            .filter_map(|value| value.measured_value())
            .copied()
            .max()
            .map_or_else(
                || Measurement::unavailable("no measured values"),
                Measurement::measured,
            );
    }
    state_from_measurements(&values)
}

pub(super) fn aggregate_sum(values: Vec<&Measurement<u64>>) -> Measurement<u64> {
    if values.iter().all(|value| value.is_measured()) {
        return Measurement::measured(
            values
                .iter()
                .filter_map(|value| value.measured_value())
                .copied()
                .fold(0_u64, u64::saturating_add),
        );
    }
    state_from_measurements(&values)
}

fn state_from_measurements<T>(values: &[&Measurement<T>]) -> Measurement<T>
where
    T: Clone,
{
    if values
        .iter()
        .all(|value| matches!(value, Measurement::NotApplicable { .. }))
    {
        return Measurement::not_applicable("all observations marked not applicable");
    }
    Measurement::unavailable("one or more observations lack this measurement")
}

pub(super) fn aggregate_check(values: Vec<&Measurement<CheckStatus>>) -> Measurement<CheckStatus> {
    if values.iter().all(|value| value.is_measured()) {
        if values
            .iter()
            .any(|value| matches!(value.measured_value(), Some(CheckStatus::Failed)))
        {
            Measurement::measured(CheckStatus::Failed)
        } else if values
            .iter()
            .all(|value| matches!(value.measured_value(), Some(CheckStatus::NotDetected)))
        {
            Measurement::measured(CheckStatus::NotDetected)
        } else {
            Measurement::measured(CheckStatus::Passed)
        }
    } else {
        state_from_measurements(&values)
    }
}

pub(super) fn aggregate_operation(
    observations: &[&LearnedSparseBenchmarkObservation],
    selector: impl Fn(&LearnedSparseResourceMetrics) -> &LearnedSparseOperationMeasurement,
) -> LearnedSparseOperationMeasurement {
    let selected = observations
        .iter()
        .map(|observation| selector(&observation.resources));
    let selected = selected.collect::<Vec<_>>();
    LearnedSparseOperationMeasurement {
        elapsed_ms: aggregate_sum(selected.iter().map(|value| &value.elapsed_ms).collect()),
        throughput_items_per_second: aggregate_u64(
            selected
                .iter()
                .map(|value| &value.throughput_items_per_second)
                .collect(),
        ),
        cost_micros: aggregate_sum(selected.iter().map(|value| &value.cost_micros).collect()),
        energy_millijoules: aggregate_sum(
            selected
                .iter()
                .map(|value| &value.energy_millijoules)
                .collect(),
        ),
    }
}

pub(super) fn aggregate(
    cases: &[&LearnedSparseBenchmarkCase],
    route: LearnedSparseRoute,
    observations: &[LearnedSparseBenchmarkObservation],
) -> Result<LearnedSparseRouteMetrics, LearnedSparseBenchmarkError> {
    let selected = select_observations(cases, route, observations)?;
    let quality = super::quality_resources::aggregate_quality(&selected);
    let resources = super::quality_resources::aggregate_resources(&selected);
    let safety = super::safety::aggregate_safety(&selected);
    let budget_violations = selected
        .iter()
        .zip(cases.iter())
        .filter(|(observation, case)| exceeds_budget(observation, case))
        .count()
        .min(u32::MAX as usize) as u32;
    Ok(LearnedSparseRouteMetrics {
        quality,
        resources,
        safety,
        budget_violations,
    })
}

pub(super) fn aggregate_provider(
    observations: &[&LearnedSparseBenchmarkObservation],
) -> Measurement<super::measurements::LearnedSparseProviderDisclosure> {
    let providers = observations
        .iter()
        .map(|observation| &observation.safety.provider)
        .collect::<Vec<_>>();
    if providers.iter().all(|provider| provider.is_measured()) {
        let first = providers
            .first()
            .and_then(|provider| provider.measured_value());
        if providers
            .iter()
            .all(|provider| provider.measured_value() == first)
        {
            return first.cloned().map_or_else(
                || Measurement::unavailable("provider disclosure is empty"),
                Measurement::measured,
            );
        }
        return Measurement::unavailable("provider disclosure differs across observations");
    }
    state_from_measurements(&providers)
}

fn exceeds_budget(
    observation: &LearnedSparseBenchmarkObservation,
    case: &&LearnedSparseBenchmarkCase,
) -> bool {
    let exceeds = |measurement: &Measurement<u64>, budget: u64| {
        measurement
            .measured_value()
            .is_some_and(|value| *value > budget)
    };
    let operation_exceeds = |operation: &LearnedSparseOperationMeasurement| {
        exceeds(&operation.elapsed_ms, case.ingest_update_budget_ms)
            || exceeds(
                &operation.cost_micros,
                case.ingest_update_budget_ms.saturating_mul(1_000),
            )
            || exceeds(
                &operation.energy_millijoules,
                case.energy_budget_millijoules,
            )
    };
    exceeds(
        &observation.resources.p95_latency_ms,
        case.latency_budget_ms,
    ) || exceeds(
        &observation.resources.peak_ram_bytes,
        case.memory_budget_bytes,
    ) || exceeds(
        &observation.resources.index_disk_bytes,
        case.disk_budget_bytes,
    ) || [
        &observation.resources.initial_indexing,
        &observation.resources.incremental_update,
        &observation.resources.deletion,
        &observation.resources.rebuild,
        &observation.resources.activation,
        &observation.resources.rollback,
    ]
    .iter()
    .any(|operation| operation_exceeds(operation))
        || exceeds(&observation.safety.energy, case.energy_budget_millijoules)
}
