//! RAPL energy accounting for the four-profile benchmark.
//!
//! Energy is read from the package energy counter
//! (`/sys/class/powercap/intel-rapl:0/energy_uj`). The counter wraps at
//! `max_energy_range_uj`, so deltas are taken modulo that range. When the
//! counter is unreadable (no privileges, no RAPL), every energy measurement
//! is recorded `Unavailable` instead of being inferred.

use std::path::Path;

const RAPL_ENERGY_PATH: &str = "/sys/class/powercap/intel-rapl:0/energy_uj";
const RAPL_MAX_PATH: &str = "/sys/class/powercap/intel-rapl:0/max_energy_range_uj";

fn read_uj(path: &str) -> Option<u64> {
    let contents = std::fs::read_to_string(Path::new(path)).ok()?;
    contents.trim().parse::<u64>().ok()
}

/// One RAPL energy sample with wraparound handling.
#[derive(Debug, Clone, Copy)]
pub(super) struct EnergySample {
    counter: u64,
    range: u64,
}

impl EnergySample {
    /// Captures the current package energy counter, or `None` when RAPL is
    /// unreadable on this host.
    pub(super) fn capture() -> Option<Self> {
        let counter = read_uj(RAPL_ENERGY_PATH)?;
        let range = read_uj(RAPL_MAX_PATH)?;
        Some(Self { counter, range })
    }

    /// Microjoules consumed between this sample and `later`.
    pub(super) fn delta_uj(self, later: Self) -> u64 {
        if self.range == 0 {
            return 0;
        }
        later
            .counter
            .wrapping_sub(self.counter)
            .rem_euclid(self.range)
    }

    /// Converts an optional before/after pair into a millijoule measurement.
    pub(super) fn delta_uj_pair(
        before: Option<Self>,
        after: Option<Self>,
    ) -> maestria_retrieval::Measurement<u64> {
        match (before, after) {
            (Some(before), Some(after)) => maestria_retrieval::Measurement::measured(
                before.delta_uj(after).saturating_div(1_000),
            ),
            _ => maestria_retrieval::Measurement::unavailable(
                "RAPL energy_uj is not readable without privileges on this host",
            ),
        }
    }
}
