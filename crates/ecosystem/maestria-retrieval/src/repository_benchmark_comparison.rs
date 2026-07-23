use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use super::benchmark_metrics;
use super::{
    RepositoryBenchmarkCorpus, RepositoryBenchmarkError, RepositoryBenchmarkObservation,
    RepositoryQueryClass, RepositoryRoute,
};
use crate::golden::Metric;

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
    pub peak_disk_bytes: u64,
    pub privacy_violations: u32,
    pub security_violations: u32,
    pub energy_milliwatt_seconds: u64,
    pub citation_alignment: Metric,
}

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

    /// Compare both routes and derive a class-scoped promotion result.
    pub fn evaluate(
        corpus: &RepositoryBenchmarkCorpus,
        observations: &[RepositoryBenchmarkObservation],
    ) -> Result<Self, RepositoryBenchmarkError> {
        corpus.validate()?;
        let mut by_case_route = BTreeSet::new();
        for observation in observations {
            if observation.corpus_id != corpus.corpus_id {
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
