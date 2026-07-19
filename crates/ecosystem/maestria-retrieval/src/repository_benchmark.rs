#[path = "repository_benchmark_runner.rs"]
mod runner;
pub use runner::{RepositoryBenchmarkExecutor, run_repository_benchmark};
#[path = "benchmark_metrics.rs"]
mod benchmark_metrics;
use crate::golden::Metric;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

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
    pub case_id: String,
    pub route: RepositoryRoute,
    pub exact_span_hits: usize,
    pub evidence_chain_length: usize,
    pub latency_ms: u64,
    pub freshness_error: bool,
    pub abstained: bool,
    pub outcome_correct: bool,
    pub memory_bytes: u64,
    pub privacy_violation: bool,
    pub security_violation: bool,
    pub energy_milliwatt_seconds: u64,
}

/// Deterministic route metrics for one query class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRouteMetrics {
    pub exact_span_recall: Metric,
    pub evidence_chain_accuracy: Metric,
    pub outcome_accuracy: Metric,
    pub abstention_accuracy: Metric,
    pub p95_latency_ms: u64,
    pub freshness_errors: u32,
    pub peak_memory_bytes: u64,
    pub privacy_violations: u32,
    pub security_violations: u32,
    pub energy_milliwatt_seconds: u64,
}

/// Baseline and specialized results for one query class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryClassComparison {
    pub class: RepositoryQueryClass,
    pub phase_c: RepositoryRouteMetrics,
    pub code_specialized: RepositoryRouteMetrics,
    pub specialized_wins: bool,
}

/// Complete deterministic comparison and promotion decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryBenchmarkComparison {
    corpus_id: String,
    classes: BTreeMap<RepositoryQueryClass, RepositoryClassComparison>,
}

impl RepositoryBenchmarkComparison {
    /// Return the corpus identifier used for this comparison.
    pub fn corpus_id(&self) -> &str {
        &self.corpus_id
    }

    /// Return per-class route comparisons.
    pub fn classes(&self) -> &BTreeMap<RepositoryQueryClass, RepositoryClassComparison> {
        &self.classes
    }
}

/// Evidence that specialized routing was promoted for selected classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryPromotionRecord {
    evaluation_id: String,
    corpus_id: String,
    winning_classes: BTreeSet<RepositoryQueryClass>,
}

impl RepositoryPromotionRecord {
    /// Return the benchmark evaluation identifier.
    pub fn evaluation_id(&self) -> &str {
        &self.evaluation_id
    }

    /// Return the frozen corpus identifier used for promotion.
    pub fn corpus_id(&self) -> &str {
        &self.corpus_id
    }

    /// Return the query classes proven to benefit from specialization.
    pub fn winning_classes(&self) -> &BTreeSet<RepositoryQueryClass> {
        &self.winning_classes
    }

    fn is_valid(&self) -> bool {
        !self.evaluation_id.trim().is_empty() && !self.corpus_id.trim().is_empty()
    }
}

/// Runtime policy for repository-code routing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RepositoryExecutionPolicy {
    #[default]
    Shadow,
    Active(RepositoryPromotionRecord),
}

impl RepositoryExecutionPolicy {
    /// Return the route allowed to affect the answer for a query.
    pub fn route_for(&self, query: &str) -> RepositoryRoute {
        match self {
            Self::Shadow => RepositoryRoute::PhaseC,
            Self::Active(record) if record.is_valid() => RepositoryQueryClass::classify(query)
                .filter(|class| record.winning_classes.contains(class))
                .map_or(RepositoryRoute::PhaseC, |_| {
                    RepositoryRoute::CodeSpecialized
                }),
            Self::Active(_) => RepositoryRoute::PhaseC,
        }
    }

    /// Return whether repository code lanes may affect this query.
    pub fn allows_specialized(&self, query: &str) -> bool {
        self.route_for(query) == RepositoryRoute::CodeSpecialized
    }
}

impl RepositoryBenchmarkComparison {
    /// Compare both routes and derive a class-scoped promotion result.
    pub fn evaluate(
        corpus: &RepositoryBenchmarkCorpus,
        observations: &[RepositoryBenchmarkObservation],
    ) -> Result<Self, RepositoryBenchmarkError> {
        corpus.validate()?;
        let mut by_case_route = BTreeSet::new();
        for observation in observations {
            if observation.corpus_id != corpus.corpus_id
                || observation.repository_revision != corpus.repository_revision
            {
                return Err(RepositoryBenchmarkError::InvalidCorpus(
                    "benchmark observation identity does not match corpus".to_string(),
                ));
            }
            if corpus.case(&observation.case_id).is_none() {
                return Err(RepositoryBenchmarkError::UnknownCase(
                    observation.case_id.clone(),
                ));
            }
            if !by_case_route.insert((observation.case_id.clone(), observation.route)) {
                return Err(RepositoryBenchmarkError::DuplicateObservation {
                    case_id: observation.case_id.clone(),
                    route: observation.route,
                });
            }
        }
        let mut classes = BTreeMap::new();
        for class in RepositoryQueryClass::all() {
            let cases: Vec<_> = corpus
                .cases
                .iter()
                .filter(|case| case.class == class)
                .collect();
            let phase_c = benchmark_metrics::metrics_for(
                class,
                RepositoryRoute::PhaseC,
                &cases,
                observations,
            )?;
            let code_specialized = benchmark_metrics::metrics_for(
                class,
                RepositoryRoute::CodeSpecialized,
                &cases,
                observations,
            )?;
            let specialized_wins = benchmark_metrics::wins(&cases, &phase_c, &code_specialized);
            classes.insert(
                class,
                RepositoryClassComparison {
                    class,
                    phase_c,
                    code_specialized,
                    specialized_wins,
                },
            );
        }
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            classes,
        })
    }

    /// Create a promotion record containing only proven query classes.
    pub fn promotion(
        &self,
        evaluation_id: String,
    ) -> Result<RepositoryPromotionRecord, RepositoryBenchmarkError> {
        if evaluation_id.trim().is_empty() {
            return Err(RepositoryBenchmarkError::InvalidCorpus(
                "evaluation_id must be non-empty".to_string(),
            ));
        }
        Ok(RepositoryPromotionRecord {
            evaluation_id,
            corpus_id: self.corpus_id.clone(),
            winning_classes: self
                .classes
                .values()
                .filter(|comparison| comparison.specialized_wins)
                .map(|comparison| comparison.class)
                .collect(),
        })
    }
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
