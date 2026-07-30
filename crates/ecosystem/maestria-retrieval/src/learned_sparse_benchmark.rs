#[path = "learned_sparse_benchmark_comparison.rs"]
mod comparison;
#[path = "learned_sparse_benchmark_contract.rs"]
mod contract;
#[path = "learned_sparse_benchmark_errors.rs"]
mod errors;
#[path = "learned_sparse_benchmark_measurements.rs"]
mod measurements;
#[path = "learned_sparse_benchmark_metrics.rs"]
mod metrics;
#[path = "learned_sparse_benchmark_quality_resources.rs"]
mod quality_resources;
#[path = "learned_sparse_benchmark_safety.rs"]
mod safety;
pub use errors::LearnedSparseBenchmarkError;
use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{ContentHash, CorpusSnapshotId, IndexGenerationId};
use serde::{Deserialize, Serialize};

pub use comparison::{
    LearnedSparseBenchmarkComparison, LearnedSparseClassComparison, LearnedSparseClassDecision,
    LearnedSparsePromotionRecord, LearnedSparseRollbackTarget, LearnedSparseRouteMetrics,
};
pub use contract::{
    LearnedSparseAcceptedSpan, LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkIdentity,
    LearnedSparseDataFidelity, LearnedSparseDataSplit, LearnedSparseEnvironment,
    LearnedSparseExpectedOutcome, LearnedSparseQualityMetrics, LearnedSparseResourceMetrics,
    LearnedSparseRouteConfiguration, LearnedSparseSafetyMetrics,
};
pub use maestria_ports::LearnedSparseQueryClass;
pub use measurements::{
    CheckStatus, LearnedSparseOperationMeasurement, LearnedSparseProviderDisclosure,
    LearnedSparseRetentionPolicy, MAX_MEASUREMENT_REASON_CHARS, Measurement,
};

pub const LEARNED_SPARSE_BENCHMARK_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LearnedSparseRoute {
    Lexical,
    Hybrid,
    SparseOnly,
    SparseFused,
}
impl LearnedSparseRoute {
    pub const fn all() -> [Self; 4] {
        [
            Self::Lexical,
            Self::Hybrid,
            Self::SparseOnly,
            Self::SparseFused,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseBenchmarkCase {
    pub case_id: String,
    pub class: LearnedSparseQueryClass,
    pub query: String,
    pub latency_budget_ms: u64,
    pub memory_budget_bytes: u64,
    pub disk_budget_bytes: u64,
    pub ingest_update_budget_ms: u64,
    pub energy_budget_millijoules: u64,
    #[serde(default)]
    pub split: LearnedSparseDataSplit,
    #[serde(default)]
    pub fidelity: LearnedSparseDataFidelity,
    #[serde(default)]
    pub expected: Option<LearnedSparseExpectedOutcome>,
}

impl LearnedSparseBenchmarkCase {
    fn budget(&self) -> LearnedSparseBenchmarkBudget {
        LearnedSparseBenchmarkBudget {
            latency_ms: self.latency_budget_ms,
            memory_bytes: self.memory_budget_bytes,
            disk_bytes: self.disk_budget_bytes,
            indexing_cost_micros: self.ingest_update_budget_ms.saturating_mul(1_000),
            incremental_update_cost_micros: self.ingest_update_budget_ms.saturating_mul(1_000),
            energy_millijoules: self.energy_budget_millijoules,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseBenchmarkCorpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub judgment_set_id: String,
    pub source_input_hash: String,
    pub evaluation_date: String,
    pub cases: Vec<LearnedSparseBenchmarkCase>,
    #[serde(default)]
    pub judgment_set_hash: Option<ContentHash>,
    #[serde(default)]
    pub environment: LearnedSparseEnvironment,
    #[serde(default)]
    pub data_fidelity: LearnedSparseDataFidelity,
    #[serde(default)]
    pub corpus_snapshot: Option<CorpusSnapshotId>,
    #[serde(default)]
    pub index_generation: Option<IndexGenerationId>,
    #[serde(default)]
    pub namespace: Option<maestria_domain::SparseNamespace>,
    #[serde(default)]
    pub route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
}

impl LearnedSparseBenchmarkCorpus {
    pub fn from_json(input: &str) -> Result<Self, LearnedSparseBenchmarkError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| LearnedSparseBenchmarkError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        if self.schema_version == 0 || self.schema_version > LEARNED_SPARSE_BENCHMARK_SCHEMA_VERSION
        {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "unsupported learned-sparse benchmark schema version".to_string(),
            ));
        }
        if self.corpus_id.trim().is_empty()
            || self.corpus_revision.trim().is_empty()
            || self.judgment_set_id.trim().is_empty()
            || self.source_input_hash.trim().is_empty()
            || self.evaluation_date.trim().is_empty()
        {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "sparse corpus identity must be complete".to_string(),
            ));
        }
        if self.schema_version >= LEARNED_SPARSE_BENCHMARK_SCHEMA_VERSION {
            self.validate_complete_metadata()?;
        }
        let mut ids = BTreeSet::new();
        let mut classes = BTreeSet::new();
        for case in &self.cases {
            validate_case(case, self.schema_version)?;
            if !ids.insert(case.case_id.clone()) {
                return Err(LearnedSparseBenchmarkError::DuplicateCase(
                    case.case_id.clone(),
                ));
            }
            classes.insert(case.class);
        }
        for class in LearnedSparseQueryClass::all() {
            if !classes.contains(&class) {
                return Err(LearnedSparseBenchmarkError::MissingClass(class));
            }
        }
        Ok(())
    }

    fn validate_complete_metadata(&self) -> Result<(), LearnedSparseBenchmarkError> {
        if self.judgment_set_hash.is_none()
            || self.corpus_snapshot.is_none()
            || self.index_generation.is_none()
            || self.namespace.is_none()
        {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "schema v2 requires judgment, corpus, generation, and namespace identities"
                    .to_string(),
            ));
        }
        ContentHash::new(self.source_input_hash.clone()).map_err(|error| {
            LearnedSparseBenchmarkError::InvalidCorpus(format!(
                "source_input_hash must be a SHA-256 content hash: {error}"
            ))
        })?;
        self.environment.validate()?;
        self.namespace
            .as_ref()
            .ok_or_else(|| LearnedSparseBenchmarkError::InvalidCorpus("namespace missing".into()))?
            .validate()
            .map_err(|error| LearnedSparseBenchmarkError::InvalidCorpus(error.to_string()))?;
        for route in LearnedSparseRoute::all() {
            self.route_configurations
                .get(&route)
                .ok_or_else(|| {
                    LearnedSparseBenchmarkError::InvalidCorpus(format!(
                        "route {route:?} configuration missing"
                    ))
                })?
                .validate()?;
        }
        Ok(())
    }

    fn case(&self, case_id: &str) -> Option<&LearnedSparseBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
}

fn validate_case(
    case: &LearnedSparseBenchmarkCase,
    schema_version: u32,
) -> Result<(), LearnedSparseBenchmarkError> {
    if case.case_id.trim().is_empty() || case.query.trim().is_empty() {
        return Err(LearnedSparseBenchmarkError::InvalidCorpus(
            "sparse case identity and query must be non-empty".to_string(),
        ));
    }
    case.budget().validate(&case.case_id)?;
    if schema_version >= LEARNED_SPARSE_BENCHMARK_SCHEMA_VERSION {
        case.expected
            .as_ref()
            .ok_or_else(|| {
                LearnedSparseBenchmarkError::InvalidCorpus(
                    "schema v2 requires explicit case judgments".to_string(),
                )
            })?
            .validate()?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedSparseBenchmarkObservation {
    pub schema_version: u32,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub judgment_set_id: String,
    pub evaluation_date: String,
    pub case_id: String,
    pub route: LearnedSparseRoute,
    pub identity: LearnedSparseBenchmarkIdentity,
    pub route_configuration: LearnedSparseRouteConfiguration,
    pub quality: LearnedSparseQualityMetrics,
    pub resources: LearnedSparseResourceMetrics,
    pub safety: LearnedSparseSafetyMetrics,
}

impl LearnedSparseBenchmarkObservation {
    fn validate(
        &self,
        corpus: &LearnedSparseBenchmarkCorpus,
    ) -> Result<(), LearnedSparseBenchmarkError> {
        if self.schema_version != LEARNED_SPARSE_BENCHMARK_SCHEMA_VERSION
            || self.corpus_id != corpus.corpus_id
            || self.corpus_revision != corpus.corpus_revision
            || self.judgment_set_id != corpus.judgment_set_id
            || self.evaluation_date != corpus.evaluation_date
            || self.evaluation_date.trim().is_empty()
        {
            return Err(LearnedSparseBenchmarkError::InvalidObservation {
                case_id: self.case_id.clone(),
                route: self.route,
            });
        }
        let case = corpus
            .case(&self.case_id)
            .ok_or_else(|| LearnedSparseBenchmarkError::UnknownCase(self.case_id.clone()))?;
        self.identity.validate()?;
        if Some(self.identity.corpus_snapshot) != corpus.corpus_snapshot
            || Some(self.identity.index_generation) != corpus.index_generation
            || Some(self.identity.namespace.clone()) != corpus.namespace
        {
            return Err(LearnedSparseBenchmarkError::InvalidIdentity(
                "observation identity does not match corpus identity".to_string(),
            ));
        }
        self.route_configuration.validate()?;
        if self.route_configuration.route != self.route
            || corpus.route_configurations.get(&self.route) != Some(&self.route_configuration)
        {
            return Err(LearnedSparseBenchmarkError::InvalidObservation {
                case_id: self.case_id.clone(),
                route: self.route,
            });
        }
        self.quality.validate()?;
        self.resources.validate()?;
        self.safety.validate()?;
        let _ = case;
        Ok(())
    }
}
