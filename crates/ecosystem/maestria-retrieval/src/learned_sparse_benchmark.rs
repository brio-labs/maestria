#[path = "learned_sparse_benchmark_metrics.rs"]
mod metrics;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::golden::Metric;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LearnedSparseQueryClass {
    ExactLiteral,
    VocabularyExpansion,
    DomainTerminology,
    MultiTerm,
    NoEvidence,
    Security,
}

impl LearnedSparseQueryClass {
    pub const fn all() -> [Self; 6] {
        [
            Self::ExactLiteral,
            Self::VocabularyExpansion,
            Self::DomainTerminology,
            Self::MultiTerm,
            Self::NoEvidence,
            Self::Security,
        ]
    }

    pub fn classify(query: &str) -> Self {
        if maestria_governance::contains_prompt_injection_risk(query) {
            return Self::Security;
        }
        match maestria_domain::SearchIntent::classify(query) {
            maestria_domain::SearchIntent::ExactLookup => Self::ExactLiteral,
            maestria_domain::SearchIntent::SemanticDiscovery => Self::VocabularyExpansion,
            maestria_domain::SearchIntent::CompositionalConstraints => Self::MultiTerm,
            _ => Self::DomainTerminology,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LearnedSparseRoute {
    Lexical,
    Hybrid,
    SparseOnly,
    SparseFused,
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
}

impl LearnedSparseBenchmarkCorpus {
    pub fn from_json(input: &str) -> Result<Self, LearnedSparseBenchmarkError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| LearnedSparseBenchmarkError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        if self.schema_version == 0
            || self.corpus_id.trim().is_empty()
            || self.corpus_revision.trim().is_empty()
            || self.judgment_set_id.trim().is_empty()
            || self.source_input_hash.trim().is_empty()
            || self.evaluation_date.trim().is_empty()
        {
            return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                "sparse corpus identity must be complete".to_string(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut classes = BTreeSet::new();
        for case in &self.cases {
            validate_case(case)?;
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

    fn case(&self, case_id: &str) -> Option<&LearnedSparseBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
}

fn validate_case(case: &LearnedSparseBenchmarkCase) -> Result<(), LearnedSparseBenchmarkError> {
    if case.case_id.trim().is_empty() || case.query.trim().is_empty() {
        return Err(LearnedSparseBenchmarkError::InvalidCorpus(
            "sparse case identity and query must be non-empty".to_string(),
        ));
    }
    if case.latency_budget_ms == 0
        || case.memory_budget_bytes == 0
        || case.disk_budget_bytes == 0
        || case.ingest_update_budget_ms == 0
        || case.energy_budget_millijoules == 0
    {
        return Err(LearnedSparseBenchmarkError::InvalidCorpus(format!(
            "sparse case {} must declare positive budgets",
            case.case_id
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseBenchmarkObservation {
    pub corpus_id: String,
    pub corpus_revision: String,
    pub judgment_set_id: String,
    pub case_id: String,
    pub route: LearnedSparseRoute,
    pub model_fingerprint: String,
    pub index_generation: String,
    pub recall_at_20: Metric,
    pub ndcg_at_10: Metric,
    pub mrr_at_10: Metric,
    pub exact_span_recall: Metric,
    pub latency_ms: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub ingest_update_ms: Option<u64>,
    pub energy_millijoules: Option<u64>,
    pub privacy_violations: u32,
    pub security_violations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseRouteMetrics {
    pub recall_at_20: Metric,
    pub ndcg_at_10: Metric,
    pub mrr_at_10: Metric,
    pub exact_span_recall: Metric,
    pub p95_latency_ms: u64,
    pub peak_memory_bytes: u64,
    pub peak_disk_bytes: u64,
    pub total_ingest_update_ms: Option<u64>,
    pub total_energy_millijoules: Option<u64>,
    pub privacy_violations: u32,
    pub security_violations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LearnedSparseClassComparison {
    pub class: LearnedSparseQueryClass,
    pub routes: BTreeMap<LearnedSparseRoute, LearnedSparseRouteMetrics>,
    pub winning_route: Option<LearnedSparseRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LearnedSparseBenchmarkComparison {
    corpus_id: String,
    corpus_revision: String,
    judgment_set_id: String,
    classes: BTreeMap<LearnedSparseQueryClass, LearnedSparseClassComparison>,
}

impl LearnedSparseBenchmarkComparison {
    pub fn evaluate(
        corpus: &LearnedSparseBenchmarkCorpus,
        observations: &[LearnedSparseBenchmarkObservation],
    ) -> Result<Self, LearnedSparseBenchmarkError> {
        corpus.validate()?;
        validate_observations(corpus, observations)?;
        let mut classes = BTreeMap::new();
        for class in LearnedSparseQueryClass::all() {
            let cases = corpus
                .cases
                .iter()
                .filter(|case| case.class == class)
                .collect::<Vec<_>>();
            let mut routes = BTreeMap::new();
            for route in all_routes() {
                routes.insert(route, metrics::aggregate(&cases, route, observations)?);
            }
            let winning_route = metrics::winning_sparse_route(class, &routes);
            classes.insert(
                class,
                LearnedSparseClassComparison {
                    class,
                    routes,
                    winning_route,
                },
            );
        }
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.corpus_revision.clone(),
            judgment_set_id: corpus.judgment_set_id.clone(),
            classes,
        })
    }

    pub fn promotion(
        &self,
        evaluation_id: String,
        evaluation_date: String,
        model_fingerprint: String,
    ) -> Result<LearnedSparsePromotionRecord, LearnedSparseBenchmarkError> {
        if evaluation_id.trim().is_empty()
            || evaluation_date.trim().is_empty()
            || model_fingerprint.trim().is_empty()
        {
            return Err(LearnedSparseBenchmarkError::InvalidPromotion(
                "sparse promotion identity must be complete".to_string(),
            ));
        }
        Ok(LearnedSparsePromotionRecord {
            evaluation_id,
            evaluation_date,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            judgment_set_id: self.judgment_set_id.clone(),
            model_fingerprint,
            winning_routes: self
                .classes
                .values()
                .filter_map(|comparison| {
                    comparison
                        .winning_route
                        .map(|route| (comparison.class, route))
                })
                .collect(),
        })
    }

    pub fn classes(
        &self,
    ) -> &BTreeMap<LearnedSparseQueryClass, LearnedSparseClassComparison> {
        &self.classes
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LearnedSparsePromotionRecord {
    evaluation_id: String,
    evaluation_date: String,
    corpus_id: String,
    corpus_revision: String,
    judgment_set_id: String,
    model_fingerprint: String,
    winning_routes: BTreeMap<LearnedSparseQueryClass, LearnedSparseRoute>,
}

impl LearnedSparsePromotionRecord {
    pub(crate) fn is_valid(&self) -> bool {
        !self.evaluation_id.trim().is_empty()
            && !self.evaluation_date.trim().is_empty()
            && !self.corpus_id.trim().is_empty()
            && !self.corpus_revision.trim().is_empty()
            && !self.judgment_set_id.trim().is_empty()
            && !self.model_fingerprint.trim().is_empty()
    }

    pub fn winning_routes(
        &self,
    ) -> &BTreeMap<LearnedSparseQueryClass, LearnedSparseRoute> {
        &self.winning_routes
    }
}

fn all_routes() -> [LearnedSparseRoute; 4] {
    [
        LearnedSparseRoute::Lexical,
        LearnedSparseRoute::Hybrid,
        LearnedSparseRoute::SparseOnly,
        LearnedSparseRoute::SparseFused,
    ]
}

fn validate_observations(
    corpus: &LearnedSparseBenchmarkCorpus,
    observations: &[LearnedSparseBenchmarkObservation],
) -> Result<(), LearnedSparseBenchmarkError> {
    let mut seen = BTreeSet::new();
    for observation in observations {
        validate_observation(corpus, observation)?;
        if !seen.insert((observation.case_id.clone(), observation.route)) {
            return Err(LearnedSparseBenchmarkError::DuplicateObservation {
                case_id: observation.case_id.clone(),
                route: observation.route,
            });
        }
    }
    for case in &corpus.cases {
        for route in all_routes() {
            if !seen.contains(&(case.case_id.clone(), route)) {
                return Err(LearnedSparseBenchmarkError::MissingObservation {
                    case_id: case.case_id.clone(),
                    route,
                });
            }
        }
    }
    Ok(())
}

fn validate_observation(
    corpus: &LearnedSparseBenchmarkCorpus,
    observation: &LearnedSparseBenchmarkObservation,
) -> Result<(), LearnedSparseBenchmarkError> {
    if observation.corpus_id != corpus.corpus_id
        || observation.corpus_revision != corpus.corpus_revision
        || observation.judgment_set_id != corpus.judgment_set_id
    {
        return Err(LearnedSparseBenchmarkError::InvalidCorpus(
            "sparse observation identity does not match corpus".to_string(),
        ));
    }
    let case = corpus.case(&observation.case_id).ok_or_else(|| {
        LearnedSparseBenchmarkError::UnknownCase(observation.case_id.clone())
    })?;
    if observation.model_fingerprint.trim().is_empty()
        || observation.index_generation.trim().is_empty()
        || observation.latency_ms > case.latency_budget_ms
        || observation.memory_bytes > case.memory_budget_bytes
        || observation.disk_bytes > case.disk_budget_bytes
        || observation
            .ingest_update_ms
            .is_some_and(|value| value > case.ingest_update_budget_ms)
        || observation
            .energy_millijoules
            .is_some_and(|value| value > case.energy_budget_millijoules)
    {
        return Err(LearnedSparseBenchmarkError::InvalidObservation {
            case_id: observation.case_id.clone(),
            route: observation.route,
        });
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LearnedSparseBenchmarkError {
    #[error("invalid learned-sparse benchmark JSON: {0}")]
    InvalidJson(String),
    #[error("invalid learned-sparse benchmark corpus: {0}")]
    InvalidCorpus(String),
    #[error("learned-sparse benchmark is missing class {0:?}")]
    MissingClass(LearnedSparseQueryClass),
    #[error("learned-sparse benchmark contains duplicate case {0}")]
    DuplicateCase(String),
    #[error("learned-sparse benchmark references unknown case {0}")]
    UnknownCase(String),
    #[error("invalid observation for case {case_id} on route {route:?}")]
    InvalidObservation {
        case_id: String,
        route: LearnedSparseRoute,
    },
    #[error("duplicate observation for case {case_id} on route {route:?}")]
    DuplicateObservation {
        case_id: String,
        route: LearnedSparseRoute,
    },
    #[error("missing observation for case {case_id} on route {route:?}")]
    MissingObservation {
        case_id: String,
        route: LearnedSparseRoute,
    },
    #[error("invalid learned-sparse promotion: {0}")]
    InvalidPromotion(String),
}
