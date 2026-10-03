//! Shared benchmark report and platform-measurement helpers.

use std::time::{SystemTime, UNIX_EPOCH};

const RAPL_ENERGY_PATH: &str = "/sys/class/powercap/intel-rapl:0/energy_uj";
const RAPL_MAX_PATH: &str = "/sys/class/powercap/intel-rapl:0/max_energy_range_uj";

fn read_counter(path: &str) -> Option<u64> {
    let contents = std::fs::read_to_string(path).ok()?;
    contents.trim().parse().ok()
}

/// One package-level RAPL sample.
///
/// The counter wraps at the hardware-reported range. A missing or unreadable
/// counter returns `None`; callers must preserve that unavailable status.
#[derive(Debug, Clone, Copy)]
pub struct EnergySample {
    counter_uj: u64,
    range_uj: u64,
}

impl EnergySample {
    /// Captures the package counter, or `None` when the host denies access.
    pub fn capture() -> Option<Self> {
        let counter_uj = read_counter(RAPL_ENERGY_PATH)?;
        let range_uj = read_counter(RAPL_MAX_PATH)?;
        if range_uj == 0 {
            return None;
        }
        Some(Self {
            counter_uj,
            range_uj,
        })
    }

    /// Returns consumed energy in millijoules between two samples.
    pub fn delta_millijoules(self, later: Self) -> u64 {
        if self.range_uj == 0 {
            return 0;
        }
        let delta_uj = if later.counter_uj >= self.counter_uj {
            later.counter_uj - self.counter_uj
        } else {
            self.range_uj
                .saturating_sub(self.counter_uj)
                .saturating_add(later.counter_uj)
        };
        delta_uj.saturating_div(1_000)
    }
}

/// Current process resident-set size in bytes, when `/proc` exposes it.
pub fn current_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kilobytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.trim().strip_suffix(" kB"))
        .and_then(|value| value.trim().parse::<u64>().ok())?;
    Some(kilobytes.saturating_mul(1024))
}

/// Unix epoch seconds, matching the benchmark reports' convention.
pub fn evaluation_date() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{EnergySample, current_rss_bytes};

    #[test]
    fn energy_delta_uses_rapl_range_for_wraparound() {
        let before = EnergySample {
            counter_uj: 1_950_000,
            range_uj: 2_000_000,
        };
        let after = EnergySample {
            counter_uj: 50_000,
            range_uj: 2_000_000,
        };

        assert_eq!(before.delta_millijoules(after), 100);
    }

    #[test]
    fn current_rss_reads_the_process_resident_set() {
        let rss = current_rss_bytes();
        assert!(rss.is_some_and(|bytes| bytes > 0));
    }
}
