use maestria_domain::{
    CorpusSnapshotId, IndexGenerationId, RepresentationName, SearchExecutionBudget, SparseNamespace,
};
use maestria_ports::{SparseFingerprint, SparseIdentity};
use serde::{Deserialize, Serialize};

use crate::golden::Metric;

use super::measurements::{
    CheckStatus, LearnedSparseOperationMeasurement, LearnedSparseProviderDisclosure, Measurement,
};
use super::{LearnedSparseBenchmarkError, LearnedSparseRoute};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum LearnedSparseDataSplit {
    #[default]
    Development,
    FinalEvaluation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LearnedSparseDataFidelity {
    RealMaestriaTask,
    SyntheticAdversarial,
    SyntheticLifecycle,
    #[default]
    SyntheticContractFixture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LearnedSparseEnvironment {
    pub operating_system: String,
    pub architecture: String,
    pub cpu_model: String,
    pub software_revision: String,
    pub warmup_policy: String,
    pub sample_count: u32,
}

impl LearnedSparseEnvironment {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        let fields = [
            ("operating_system", self.operating_system.as_str()),
            ("architecture", self.architecture.as_str()),
            ("cpu_model", self.cpu_model.as_str()),
            ("software_revision", self.software_revision.as_str()),
            ("warmup_policy", self.warmup_policy.as_str()),
        ];
        if let Some((field, _)) = fields.iter().find(|(_, value)| value.trim().is_empty()) {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(format!(
                "benchmark environment field {field} must be non-empty"
            )));
        }
        if self.sample_count == 0 {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "benchmark environment sample_count must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseRouteConfiguration {
    pub route: LearnedSparseRoute,
    pub result_limit: u32,
    pub candidate_limit: u32,
    pub budget: SearchExecutionBudget,
}

impl Default for LearnedSparseRouteConfiguration {
    fn default() -> Self {
        Self {
            route: LearnedSparseRoute::Lexical,
            result_limit: 0,
            candidate_limit: 0,
            budget: SearchExecutionBudget::default(),
        }
    }
}

impl LearnedSparseRouteConfiguration {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        if self.result_limit == 0 || self.candidate_limit == 0 {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "route configuration limits must be positive".to_string(),
            ));
        }
        if u64::from(self.result_limit) > self.budget.max_results()
            || u64::from(self.candidate_limit) > self.budget.max_candidates()
        {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "route configuration exceeds declared search budget".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseBenchmarkBudget {
    pub latency_ms: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub indexing_cost_micros: u64,
    pub incremental_update_cost_micros: u64,
    pub energy_millijoules: u64,
}

impl LearnedSparseBenchmarkBudget {
    pub fn validate(&self, case_id: &str) -> Result<(), LearnedSparseBenchmarkError> {
        if self.latency_ms == 0
            || self.memory_bytes == 0
            || self.disk_bytes == 0
            || self.indexing_cost_micros == 0
            || self.incremental_update_cost_micros == 0
            || self.energy_millijoules == 0
        {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(format!(
                "sparse case {case_id} must declare positive budgets"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseAcceptedSpan {
    pub source_id: String,
    pub start: u32,
    pub end: u32,
}

impl LearnedSparseAcceptedSpan {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        if self.source_id.trim().is_empty() || self.start >= self.end {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "accepted evidence spans must have a source and increasing bounds".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseExpectedOutcome {
    Evidence {
        accepted_spans: Vec<LearnedSparseAcceptedSpan>,
        evidence_chain: Vec<String>,
        minimum_source_diversity: u32,
    },
    Abstain,
    UnsupportedClaim,
    Conflict,
}

impl LearnedSparseExpectedOutcome {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        match self {
            Self::Evidence {
                accepted_spans,
                evidence_chain,
                minimum_source_diversity,
            } => {
                if accepted_spans.is_empty()
                    || evidence_chain.is_empty()
                    || *minimum_source_diversity == 0
                {
                    return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                        "evidence judgments must contain spans, a chain, and source diversity"
                            .to_string(),
                    ));
                }
                for span in accepted_spans {
                    span.validate()?;
                }
                if evidence_chain.iter().any(|source| source.trim().is_empty()) {
                    return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                        "evidence-chain identities must be non-empty".to_string(),
                    ));
                }
            }
            Self::Abstain | Self::UnsupportedClaim | Self::Conflict => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseQualityMetrics {
    pub recall_at_5: Measurement<Metric>,
    pub recall_at_20: Measurement<Metric>,
    pub recall_at_50: Measurement<Metric>,
    pub recall_at_100: Measurement<Metric>,
    pub ndcg_at_10: Measurement<Metric>,
    pub ndcg_at_20: Measurement<Metric>,
    pub mrr_at_10: Measurement<Metric>,
    pub mean_average_precision: Measurement<Metric>,
    pub exact_span_recall: Measurement<Metric>,
    pub evidence_chain_coverage: Measurement<Metric>,
    pub source_diversity: Measurement<Metric>,
    pub source_redundancy: Measurement<Metric>,
    pub citation_precision: Measurement<Metric>,
    pub citation_recall: Measurement<Metric>,
    pub abstention_precision: Measurement<Metric>,
    pub abstention_recall: Measurement<Metric>,
    pub unsupported_claim_status: Measurement<CheckStatus>,
    pub conflict_detection_status: Measurement<CheckStatus>,
}

impl LearnedSparseQualityMetrics {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        self.validate_metrics([
            &self.recall_at_5,
            &self.recall_at_20,
            &self.recall_at_50,
            &self.recall_at_100,
            &self.ndcg_at_10,
            &self.ndcg_at_20,
            &self.mrr_at_10,
            &self.mean_average_precision,
            &self.exact_span_recall,
            &self.evidence_chain_coverage,
            &self.source_diversity,
            &self.source_redundancy,
            &self.citation_precision,
            &self.citation_recall,
            &self.abstention_precision,
            &self.abstention_recall,
        ])?;
        self.unsupported_claim_status
            .validate()
            .map_err(invalid_measurement)?;
        self.conflict_detection_status
            .validate()
            .map_err(invalid_measurement)
    }

    fn validate_metrics<const N: usize>(
        &self,
        metrics: [&Measurement<Metric>; N],
    ) -> Result<(), LearnedSparseBenchmarkError> {
        for metric in metrics {
            metric.validate().map_err(invalid_measurement)?;
            if metric
                .measured_value()
                .is_some_and(|value| value.value() > Metric::ONE.value())
            {
                return Err(LearnedSparseBenchmarkError::InvalidMeasurement(
                    "quality metric exceeds the fixed-point range".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseResourceMetrics {
    pub p50_latency_ms: Measurement<u64>,
    pub p95_latency_ms: Measurement<u64>,
    pub p99_latency_ms: Measurement<u64>,
    pub peak_ram_bytes: Measurement<u64>,
    pub index_disk_bytes: Measurement<u64>,
    pub initial_indexing: LearnedSparseOperationMeasurement,
    pub incremental_update: LearnedSparseOperationMeasurement,
    pub deletion: LearnedSparseOperationMeasurement,
    pub rebuild: LearnedSparseOperationMeasurement,
    pub activation: LearnedSparseOperationMeasurement,
    pub rollback: LearnedSparseOperationMeasurement,
}

impl LearnedSparseResourceMetrics {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        for measurement in [
            &self.p50_latency_ms,
            &self.p95_latency_ms,
            &self.p99_latency_ms,
            &self.peak_ram_bytes,
            &self.index_disk_bytes,
        ] {
            measurement.validate().map_err(invalid_measurement)?;
        }
        for operation in [
            &self.initial_indexing,
            &self.incremental_update,
            &self.deletion,
            &self.rebuild,
            &self.activation,
            &self.rollback,
        ] {
            operation.validate().map_err(invalid_measurement)?;
        }
        if let (Some(p50), Some(p95), Some(p99)) = (
            self.p50_latency_ms.measured_value(),
            self.p95_latency_ms.measured_value(),
            self.p99_latency_ms.measured_value(),
        ) && !(p50 <= p95 && p95 <= p99)
        {
            return Err(LearnedSparseBenchmarkError::InvalidMeasurement(
                "latency percentiles must be ordered p50 <= p95 <= p99".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseSafetyMetrics {
    pub provider: Measurement<LearnedSparseProviderDisclosure>,
    pub namespace_isolation: Measurement<CheckStatus>,
    pub acl_leakage: Measurement<u32>,
    pub attack_outcome: Measurement<CheckStatus>,
    pub poisoning_outcome: Measurement<CheckStatus>,
    pub secret_exposure: Measurement<CheckStatus>,
    pub quarantine_outcome: Measurement<CheckStatus>,
    pub prompt_injection_outcome: Measurement<CheckStatus>,
    pub fail_open_count: Measurement<u32>,
    pub energy: Measurement<u64>,
}

impl LearnedSparseSafetyMetrics {
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        self.provider.validate().map_err(invalid_measurement)?;
        self.namespace_isolation
            .validate()
            .map_err(invalid_measurement)?;
        self.acl_leakage.validate().map_err(invalid_measurement)?;
        self.attack_outcome
            .validate()
            .map_err(invalid_measurement)?;
        self.poisoning_outcome
            .validate()
            .map_err(invalid_measurement)?;
        self.secret_exposure
            .validate()
            .map_err(invalid_measurement)?;
        self.quarantine_outcome
            .validate()
            .map_err(invalid_measurement)?;
        self.prompt_injection_outcome
            .validate()
            .map_err(invalid_measurement)?;
        self.fail_open_count
            .validate()
            .map_err(invalid_measurement)?;
        self.energy.validate().map_err(invalid_measurement)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedSparseBenchmarkIdentity {
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub representation: RepresentationName,
    pub namespace: SparseNamespace,
    pub fingerprint: SparseFingerprint,
    pub backend_fingerprint: String,
}

impl LearnedSparseBenchmarkIdentity {
    pub fn from_sparse_identity(
        identity: &SparseIdentity,
        backend_fingerprint: impl Into<String>,
    ) -> Result<Self, LearnedSparseBenchmarkError> {
        let result = Self {
            corpus_snapshot: identity.corpus_snapshot,
            index_generation: identity.generation_id,
            representation: identity.representation.clone(),
            namespace: identity.namespace.clone(),
            fingerprint: identity.fingerprint.clone(),
            backend_fingerprint: backend_fingerprint.into(),
        };
        result.validate()?;
        Ok(result)
    }

    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        self.fingerprint
            .validate()
            .map_err(|error| LearnedSparseBenchmarkError::InvalidIdentity(error.to_string()))?;
        if self.representation.0 != maestria_ports::SPARSE_REPRESENTATION_V1
            || self.namespace.validate().is_err()
            || self.namespace.projection() != self.representation.0
            || self.backend_fingerprint.trim().is_empty()
        {
            return Err(LearnedSparseBenchmarkError::InvalidIdentity(
                "sparse benchmark identity is incomplete or incompatible".to_string(),
            ));
        }
        Ok(())
    }
}

fn invalid_measurement(message: &'static str) -> LearnedSparseBenchmarkError {
    LearnedSparseBenchmarkError::InvalidMeasurement(message.to_string())
}
