#[path = "repository_benchmark_runner.rs"]
mod runner;
pub use runner::{
    RepositoryBenchmarkExecutor, RepositoryCodeIndexExecutor, run_repository_benchmark,
};
#[path = "benchmark_metrics.rs"]
mod benchmark_metrics;
use crate::golden::Metric;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[path = "repository_benchmark_comparison.rs"]
mod repository_benchmark_comparison;
pub use repository_benchmark_comparison::*;

/// Frozen query classes used to promote repository-code retrieval safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RepositoryQueryClass {
    ExactSymbol,
    DefinitionReference,
    IssueToFile,
    MultiHopDependency,
    TestAssociation,
    StaleWorktree,
    CorrectAbstention,
}

impl RepositoryQueryClass {
    /// Return a conservative class guess for an incoming repository question.
    pub fn classify(query: &str) -> Option<Self> {
        let normalized = query.to_ascii_lowercase();
        let class = if normalized.contains("stale")
            || normalized.contains("worktree")
            || normalized.contains("out of date")
        {
            Self::StaleWorktree
        } else if normalized.contains("don't know")
            || normalized.contains("unknown")
            || normalized.contains("not in the repository")
            || normalized.contains("abstain")
        {
            Self::CorrectAbstention
        } else if normalized.contains("issue #") || normalized.contains("issue ") {
            Self::IssueToFile
        } else if normalized.contains("test") || normalized.contains("tested by") {
            Self::TestAssociation
        } else if normalized.contains("depends on")
            || normalized.contains("dependency")
            || normalized.contains("call chain")
        {
            Self::MultiHopDependency
        } else if normalized.contains("definition")
            || normalized.contains("references")
            || normalized.contains("reference")
        {
            Self::DefinitionReference
        } else if normalized.contains("symbol")
            || normalized.contains("struct ")
            || normalized.contains("function ")
            || normalized.contains("fn ")
            || normalized.contains("::")
        {
            Self::ExactSymbol
        } else {
            return None;
        };
        Some(class)
    }

    /// Return all classes required by the frozen benchmark contract.
    pub const fn all() -> [Self; 7] {
        [
            Self::ExactSymbol,
            Self::DefinitionReference,
            Self::IssueToFile,
            Self::MultiHopDependency,
            Self::TestAssociation,
            Self::StaleWorktree,
            Self::CorrectAbstention,
        ]
    }
}

/// Route compared by the repository benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RepositoryRoute {
    PhaseC,
    CodeSpecialized,
}

/// Expected observable result for a frozen repository question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryExpectedOutcome {
    Evidence {
        exact_span_count: usize,
        evidence_chain_length: usize,
    },
    Stale,
    Abstain,
}

impl RepositoryExpectedOutcome {
    fn exact_span_count(&self) -> usize {
        match self {
            Self::Evidence {
                exact_span_count, ..
            } => *exact_span_count,
            Self::Stale | Self::Abstain => 0,
        }
    }

    fn evidence_chain_length(&self) -> usize {
        match self {
            Self::Evidence {
                evidence_chain_length,
                ..
            } => *evidence_chain_length,
            Self::Stale | Self::Abstain => 0,
        }
    }
}

/// Explicit status for platform-level measurements that cannot be collected
/// on the current hardware or operating system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasurementStatus {
    /// Counters were available and values reflect real resource usage.
    Measured,
    /// Counters are absent for the current platform or execution context.
    Unavailable {
        /// Human-readable explanation of why the measurement is absent.
        reason: String,
    },
}

impl Default for MeasurementStatus {
    fn default() -> Self {
        Self::Unavailable {
            reason: "platform counters not available".into(),
        }
    }
}

/// One frozen question and its source-grounded acceptance shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryBenchmarkCase {
    pub case_id: String,
    pub class: RepositoryQueryClass,
    pub query: String,
    pub expected: RepositoryExpectedOutcome,
    pub latency_budget_ms: u64,
}

/// Versioned, frozen Rust repository benchmark corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryBenchmarkCorpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub repository_revision: String,
    /// ISO‑8601 date of the original corpus freeze.
    #[serde(default)]
    pub evaluation_date: String,
    /// Code‑parser generation identifier used to build the reference index.
    #[serde(default)]
    pub index_generation: String,
    pub cases: Vec<RepositoryBenchmarkCase>,
}

impl RepositoryBenchmarkCorpus {
    /// Parse and validate a frozen corpus.
    pub fn from_json(input: &str) -> Result<Self, RepositoryBenchmarkError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| RepositoryBenchmarkError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    /// Validate uniqueness, coverage, and bounded acceptance criteria.
    pub fn validate(&self) -> Result<(), RepositoryBenchmarkError> {
        if self.schema_version == 0 {
            return Err(RepositoryBenchmarkError::InvalidCorpus(
                "schema_version must be non-zero".to_string(),
            ));
        }
        if self.corpus_id.trim().is_empty() || self.repository_revision.trim().is_empty() {
            return Err(RepositoryBenchmarkError::InvalidCorpus(
                "corpus_id and repository_revision must be non-empty".to_string(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut classes = BTreeSet::new();
        for case in &self.cases {
            if case.case_id.trim().is_empty() || case.query.trim().is_empty() {
                return Err(RepositoryBenchmarkError::InvalidCorpus(
                    "case_id and query must be non-empty".to_string(),
                ));
            }
            if case.latency_budget_ms == 0 {
                return Err(RepositoryBenchmarkError::InvalidCorpus(format!(
                    "case {} has no latency budget",
                    case.case_id
                )));
            }
            if !ids.insert(case.case_id.clone()) {
                return Err(RepositoryBenchmarkError::DuplicateCase(
                    case.case_id.clone(),
                ));
            }
            if !classes.insert(case.class) {
                return Err(RepositoryBenchmarkError::DuplicateClass(case.class));
            }
            match &case.expected {
                RepositoryExpectedOutcome::Evidence {
                    exact_span_count,
                    evidence_chain_length,
                } if *exact_span_count == 0 || *evidence_chain_length == 0 => {
                    return Err(RepositoryBenchmarkError::InvalidCorpus(format!(
                        "case {} has an empty evidence acceptance shape",
                        case.case_id
                    )));
                }
                RepositoryExpectedOutcome::Evidence { .. }
                | RepositoryExpectedOutcome::Stale
                | RepositoryExpectedOutcome::Abstain => {}
            }
        }
        for class in RepositoryQueryClass::all() {
            if !classes.contains(&class) {
                return Err(RepositoryBenchmarkError::MissingClass(class));
            }
        }
        Ok(())
    }

    fn case(&self, case_id: &str) -> Option<&RepositoryBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
}

/// Measurements produced by executing one frozen case on one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryBenchmarkObservation {
    pub corpus_id: String,
    pub repository_revision: String,
    /// ISO‑8601 timestamp of when the measurement was taken.
    #[serde(default)]
    pub evaluation_date: String,
    /// Parser generation identifier for the code index used.
    #[serde(default)]
    pub index_generation: String,
    /// Fingerprint of the model or provider that produced the measurements.
    #[serde(default)]
    pub model_fingerprint: String,
    /// Serialised route‑configuration snapshot at measurement time.
    #[serde(default)]
    pub route_config: serde_json::Value,
    pub case_id: String,
    pub route: RepositoryRoute,
    pub exact_span_hits: usize,
    pub evidence_chain_length: usize,
    pub latency_ms: u64,
    pub freshness_error: bool,
    pub abstained: bool,
    pub outcome_correct: bool,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub privacy_violation: bool,
    pub security_violation: bool,
    pub energy_milliwatt_seconds: u64,
    /// Quality of citation alignment (0–10 000 fixed point).
    #[serde(default)]
    pub citation_alignment: Metric,
    #[serde(default)]
    pub measurement_status: MeasurementStatus,
}

/// Errors raised while loading or evaluating the frozen benchmark.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RepositoryBenchmarkError {
    #[error("invalid benchmark JSON: {0}")]
    InvalidJson(String),
    #[error("invalid benchmark corpus: {0}")]
    InvalidCorpus(String),
    #[error("benchmark is missing query class {0:?}")]
    MissingClass(RepositoryQueryClass),
    #[error("benchmark contains duplicate case {0}")]
    DuplicateCase(String),
    #[error("benchmark contains duplicate class {0:?}")]
    DuplicateClass(RepositoryQueryClass),
    #[error("benchmark observation references unknown case {0}")]
    UnknownCase(String),
    #[error("benchmark is missing observation for case {case_id} on route {route:?}")]
    MissingObservation {
        case_id: String,
        route: RepositoryRoute,
    },
    #[error("benchmark has duplicate observation for case {case_id} on route {route:?}")]
    DuplicateObservation {
        case_id: String,
        route: RepositoryRoute,
    },
}
