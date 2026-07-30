use super::{LearnedSparseBenchmarkObservation, LearnedSparseSafetyMetrics};

pub(super) fn aggregate_safety(
    selected: &[&LearnedSparseBenchmarkObservation],
) -> LearnedSparseSafetyMetrics {
    LearnedSparseSafetyMetrics {
        provider: super::metrics::aggregate_provider(selected),
        namespace_isolation: super::metrics::aggregate_check(
            selected
                .iter()
                .map(|o| &o.safety.namespace_isolation)
                .collect(),
        ),
        acl_leakage: super::metrics::aggregate_u32(
            selected.iter().map(|o| &o.safety.acl_leakage).collect(),
        ),
        attack_outcome: super::metrics::aggregate_check(
            selected.iter().map(|o| &o.safety.attack_outcome).collect(),
        ),
        poisoning_outcome: super::metrics::aggregate_check(
            selected
                .iter()
                .map(|o| &o.safety.poisoning_outcome)
                .collect(),
        ),
        secret_exposure: super::metrics::aggregate_check(
            selected.iter().map(|o| &o.safety.secret_exposure).collect(),
        ),
        quarantine_outcome: super::metrics::aggregate_check(
            selected
                .iter()
                .map(|o| &o.safety.quarantine_outcome)
                .collect(),
        ),
        prompt_injection_outcome: super::metrics::aggregate_check(
            selected
                .iter()
                .map(|o| &o.safety.prompt_injection_outcome)
                .collect(),
        ),
        fail_open_count: super::metrics::aggregate_u32(
            selected.iter().map(|o| &o.safety.fail_open_count).collect(),
        ),
        energy: super::metrics::aggregate_sum(selected.iter().map(|o| &o.safety.energy).collect()),
    }
}
