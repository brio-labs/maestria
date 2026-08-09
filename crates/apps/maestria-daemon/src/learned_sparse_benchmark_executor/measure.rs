//! Measurement helpers for the lifecycle operations.

use maestria_retrieval::{
    LearnedSparseOperationMeasurement, LearnedSparseRoute, Measurement, MonotonicInstant,
};

use super::LearnedSparseBenchmarkExecutor;
use super::energy::EnergySample;

impl LearnedSparseBenchmarkExecutor {
    pub(super) fn unavailable_operation(
        route: LearnedSparseRoute,
        operation: &str,
        reason: String,
    ) -> LearnedSparseOperationMeasurement {
        LearnedSparseOperationMeasurement {
            elapsed_ms: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
            throughput_items_per_second: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
            cost_micros: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
            energy_millijoules: EnergySample::delta_uj_pair(
                EnergySample::capture(),
                EnergySample::capture(),
            ),
        }
    }

    pub(super) fn finish_measurement(
        route: LearnedSparseRoute,
        operation: &str,
        items: usize,
        started: MonotonicInstant,
        energy_before: Option<EnergySample>,
        result: Result<(), anyhow::Error>,
    ) -> LearnedSparseOperationMeasurement {
        let elapsed = started.elapsed();
        let elapsed_us = elapsed.as_micros() as u64;
        let energy_after = EnergySample::capture();
        match result {
            Ok(()) => LearnedSparseOperationMeasurement {
                elapsed_ms: Measurement::measured(elapsed_us.saturating_div(1_000)),
                throughput_items_per_second: Measurement::measured(
                    match (items as u128)
                        .saturating_mul(1_000_000)
                        .checked_div(elapsed.as_micros().max(1))
                    {
                        Some(value) => value as u64,
                        None => 0,
                    },
                ),
                cost_micros: Measurement::measured(elapsed_us),
                energy_millijoules: EnergySample::delta_uj_pair(energy_before, energy_after),
            },
            Err(error) => {
                let reason = format!("{operation} on the {route:?} projection failed: {error}");
                LearnedSparseOperationMeasurement {
                    elapsed_ms: Measurement::unavailable(reason.clone()),
                    throughput_items_per_second: Measurement::unavailable(reason.clone()),
                    cost_micros: Measurement::unavailable(reason.clone()),
                    energy_millijoules: EnergySample::delta_uj_pair(energy_before, energy_after),
                }
            }
        }
    }

    /// Measures one lifecycle operation on the route's projection.
    pub(super) fn measure_operation(
        &self,
        route: LearnedSparseRoute,
        operation: &str,
        items: usize,
        op: impl Fn() -> Result<(), anyhow::Error>,
    ) -> LearnedSparseOperationMeasurement {
        let started = MonotonicInstant::now();
        let energy_before = EnergySample::capture();
        let result = op();
        Self::finish_measurement(route, operation, items, started, energy_before, result)
    }

    /// The zero-chunk op set: an empty projection's lifecycle measurements.
    pub(super) fn empty_chunk_ops(
        &self,
        route: LearnedSparseRoute,
    ) -> (
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
    ) {
        let reason = "the projection has no chunks in this instance".to_string();
        (
            Self::unavailable_operation(route, "initial indexing", reason.clone()),
            Self::unavailable_operation(route, "incremental update", reason.clone()),
            Self::unavailable_operation(route, "deletion", reason.clone()),
            Self::unavailable_operation(route, "rebuild", reason.clone()),
            Self::unavailable_operation(route, "activation", reason.clone()),
            Self::unavailable_operation(route, "rollback", reason),
        )
    }
}
