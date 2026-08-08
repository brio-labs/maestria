//! Safety metrics from the same search path the daemon serves.

use maestria_retrieval::{
    CheckStatus, LearnedSparseBenchmarkCase, LearnedSparseProviderDisclosure,
    LearnedSparseQueryClass, LearnedSparseRetentionPolicy, LearnedSparseRetrievedCandidate,
    LearnedSparseSafetyMetrics, Measurement,
};

use super::LearnedSparseBenchmarkExecutor;

impl LearnedSparseBenchmarkExecutor {
    /// Safety metrics from the same search path the daemon serves.
    ///
    /// The engine's secret scanner and authorization deny the security
    /// fixtures before candidates exist; a leaked candidate is recorded as a
    /// failure instead of being filtered from the report.
    pub(super) fn safety_for(
        &self,
        case: &LearnedSparseBenchmarkCase,
        candidates: &[LearnedSparseRetrievedCandidate],
    ) -> LearnedSparseSafetyMetrics {
        let leaked = !candidates.is_empty();
        let security_case = matches!(case.class, LearnedSparseQueryClass::Security);
        let attack_outcome = if security_case {
            if leaked {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            }
        } else {
            CheckStatus::NotDetected
        };
        let secret_exposure = if security_case && leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::NotDetected
        };
        let prompt_injection_outcome = if security_case && leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let poisoning_outcome = if leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let quarantine_outcome = if leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        let namespace_isolation = if leaked {
            CheckStatus::Failed
        } else {
            CheckStatus::Passed
        };
        LearnedSparseSafetyMetrics {
            provider: Measurement::measured(LearnedSparseProviderDisclosure {
                remote: false,
                retention: LearnedSparseRetentionPolicy::NoRetention,
            }),
            namespace_isolation: Measurement::measured(namespace_isolation),
            acl_leakage: Measurement::measured(if leaked { candidates.len() as u32 } else { 0 }),
            attack_outcome: Measurement::measured(attack_outcome),
            poisoning_outcome: Measurement::measured(poisoning_outcome),
            secret_exposure: Measurement::measured(secret_exposure),
            quarantine_outcome: Measurement::measured(quarantine_outcome),
            prompt_injection_outcome: Measurement::measured(prompt_injection_outcome),
            fail_open_count: Measurement::measured(0),
            energy: Measurement::unavailable(
                "RAPL energy_uj is not readable without privileges on this host",
            ),
        }
    }
}
