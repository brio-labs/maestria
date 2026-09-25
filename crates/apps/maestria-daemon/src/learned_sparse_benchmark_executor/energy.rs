//! RAPL energy accounting for the four-profile benchmark.
//!
//! The counter reader and wraparound handling live in the shared retrieval
//! benchmark helpers so all benchmark routes use the same platform contract.

pub(super) use maestria_retrieval::benchmark_common::EnergySample;

pub(super) fn delta_millijoules_pair(
    before: Option<EnergySample>,
    after: Option<EnergySample>,
) -> maestria_retrieval::Measurement<u64> {
    match (before, after) {
        (Some(before), Some(after)) => {
            maestria_retrieval::Measurement::measured(before.delta_millijoules(after))
        }
        _ => maestria_retrieval::Measurement::unavailable(
            "RAPL energy_uj is not readable without privileges on this host",
        ),
    }
}
