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
            if case.case_id.trim().is_empty() || case.query.trim().is_empty() {
                return Err(LearnedSparseBenchmarkError::InvalidCorpus(
                    "sparse case identity and query must be non-empty".to_string(),
                ));
            }
            if !ids.insert(case.case_id.clone()) {
                return Err(LearnedSparseBenchmarkError::DuplicateCase(
                    case.case_id.clone(),
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
            classes.insert(case.class);
        }
        for class in Self::required_classes() {
            if !classes.contains(&class) {
                return Err(LearnedSparseBenchmarkError::MissingClass(class));
            }
        }
        Ok(())
    }

    fn required_classes() -> [LearnedSparseQueryClass; 6] {
        LearnedSparseQueryClass::all()
    }

    fn case(&self, case_id: &str) -> Option<&LearnedSparseBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
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
            for route in [
                LearnedSparseRoute::Lexical,
                LearnedSparseRoute::Hybrid,
                LearnedSparseRoute::SparseOnly,
                LearnedSparseRoute::SparseFused,
            ] {
                routes.insert(route, aggregate(&cases, route, observations)?);
            }
            let winning_route = winning_sparse_route(class, &routes);
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
        let winning_routes = self
            .classes
            .values()
            .filter_map(|comparison| {
                comparison
                    .winning_route
                    .map(|route| (comparison.class, route))
            })
            .collect();
        Ok(LearnedSparsePromotionRecord {
            evaluation_id,
            evaluation_date,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            judgment_set_id: self.judgment_set_id.clone(),
            model_fingerprint,
            winning_routes,
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
    fn is_valid(&self) -> bool {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LearnedSparseExecutionPolicy {
    #[default]
    Shadow,
    Active(LearnedSparsePromotionRecord),
}

impl LearnedSparseExecutionPolicy {
    pub fn route_for(&self, query: &str) -> LearnedSparseRoute {
        let class = LearnedSparseQueryClass::classify(query);
        match self {
            Self::Active(record) if record.is_valid() => record
                .winning_routes
                .get(&class)
                .copied()
                .unwrap_or(LearnedSparseRoute::Hybrid),
            Self::Shadow | Self::Active(_) => LearnedSparseRoute::Hybrid,
        }
    }

    pub fn allows_sparse(&self, query: &str) -> bool {
        matches!(
            self.route_for(query),
            LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused
        )
    }
}

pub(crate) fn sparse_lane_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    sparse_enabled: bool,
) -> bool {
    let id = descriptor.id.to_ascii_lowercase();
    let is_sparse = descriptor.modality.eq_ignore_ascii_case("sparse")
        || id.contains("learned_sparse")
        || descriptor.representation.0 == maestria_ports::SPARSE_REPRESENTATION_V1;
    sparse_enabled || !is_sparse
}

fn validate_observations(
    corpus: &LearnedSparseBenchmarkCorpus,
    observations: &[LearnedSparseBenchmarkObservation],
) -> Result<(), LearnedSparseBenchmarkError> {
    let mut seen = BTreeSet::new();
    for observation in observations {
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
        if !seen.insert((observation.case_id.clone(), observation.route)) {
            return Err(LearnedSparseBenchmarkError::DuplicateObservation {
                case_id: observation.case_id.clone(),
                route: observation.route,
            });
        }
    }
    for case in &corpus.cases {
        for route in [
            LearnedSparseRoute::Lexical,
            LearnedSparseRoute::Hybrid,
            LearnedSparseRoute::SparseOnly,
            LearnedSparseRoute::SparseFused,
        ] {
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

fn aggregate(
    cases: &[&LearnedSparseBenchmarkCase],
    route: LearnedSparseRoute,
    observations: &[LearnedSparseBenchmarkObservation],
) -> Result<LearnedSparseRouteMetrics, LearnedSparseBenchmarkError> {
    let selected = cases
        .iter()
        .map(|case| {
            observations
                .iter()
                .find(|observation| observation.case_id == case.case_id && observation.route == route)
                .ok_or_else(|| LearnedSparseBenchmarkError::MissingObservation {
                    case_id: case.case_id.clone(),
                    route,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let count = selected.len().max(1) as u64;
    let mean_metric = |values: Vec<u32>| {
        let value = values
            .into_iter()
            .map(u64::from)
            .fold(0_u64, u64::saturating_add)
            / count;
        Metric::new(value.min(u64::from(u32::MAX)) as u32).unwrap_or(Metric::ZERO)
    };
    let mut latencies = selected
        .iter()
        .map(|observation| observation.latency_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let p95_index = (latencies.len() * 95).div_ceil(100).saturating_sub(1);
    let p95_latency_ms = latencies.get(p95_index).copied().unwrap_or(0);
    Ok(LearnedSparseRouteMetrics {
        recall_at_20: mean_metric(
            selected
                .iter()
                .map(|observation| observation.recall_at_20.value())
                .collect(),
        ),
        ndcg_at_10: mean_metric(
            selected
                .iter()
                .map(|observation| observation.ndcg_at_10.value())
                .collect(),
        ),
        mrr_at_10: mean_metric(
            selected
                .iter()
                .map(|observation| observation.mrr_at_10.value())
                .collect(),
        ),
        exact_span_recall: mean_metric(
            selected
                .iter()
                .map(|observation| observation.exact_span_recall.value())
                .collect(),
        ),
        p95_latency_ms,
        peak_memory_bytes: selected
            .iter()
            .map(|observation| observation.memory_bytes)
            .max()
            .unwrap_or(0),
        peak_disk_bytes: selected
            .iter()
            .map(|observation| observation.disk_bytes)
            .max()
            .unwrap_or(0),
        total_ingest_update_ms: selected
            .iter()
            .map(|observation| observation.ingest_update_ms)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.into_iter().fold(0_u64, u64::saturating_add)),
        total_energy_millijoules: selected
            .iter()
            .map(|observation| observation.energy_millijoules)
            .collect::<Option<Vec<_>>>()
            .map(|values| values.into_iter().fold(0_u64, u64::saturating_add)),
        privacy_violations: selected
            .iter()
            .map(|observation| observation.privacy_violations)
            .fold(0_u32, u32::saturating_add),
        security_violations: selected
            .iter()
            .map(|observation| observation.security_violations)
            .fold(0_u32, u32::saturating_add),
    })
}

fn winning_sparse_route(
    class: LearnedSparseQueryClass,
    routes: &BTreeMap<LearnedSparseRoute, LearnedSparseRouteMetrics>,
) -> Option<LearnedSparseRoute> {
    if matches!(
        class,
        LearnedSparseQueryClass::ExactLiteral
            | LearnedSparseQueryClass::NoEvidence
            | LearnedSparseQueryClass::Security
    ) {
        return None;
    }
    let lexical = routes.get(&LearnedSparseRoute::Lexical)?;
    let hybrid = routes.get(&LearnedSparseRoute::Hybrid)?;
    [
        LearnedSparseRoute::SparseFused,
        LearnedSparseRoute::SparseOnly,
    ]
    .into_iter()
    .find(|route| {
        routes.get(route).is_some_and(|candidate| {
            telemetry_complete(candidate)
                && wins_against(candidate, lexical)
                && wins_against(candidate, hybrid)
        })
    })
}

fn telemetry_complete(metrics: &LearnedSparseRouteMetrics) -> bool {
    metrics.total_ingest_update_ms.is_some()
        && metrics.total_energy_millijoules.is_some()
        && metrics.privacy_violations == 0
        && metrics.security_violations == 0
}

fn wins_against(
    candidate: &LearnedSparseRouteMetrics,
    baseline: &LearnedSparseRouteMetrics,
) -> bool {
    let qualities = [
        (candidate.recall_at_20, baseline.recall_at_20),
        (candidate.ndcg_at_10, baseline.ndcg_at_10),
        (candidate.mrr_at_10, baseline.mrr_at_10),
        (candidate.exact_span_recall, baseline.exact_span_recall),
    ];
    let no_quality_regression = qualities
        .iter()
        .all(|(candidate, baseline)| candidate >= baseline);
    let material_improvement = qualities.iter().any(|(candidate, baseline)| {
        candidate
            .value()
            .saturating_sub(baseline.value())
            >= Metric::MATERIAL_QUALITY_DELTA.value()
    });
    no_quality_regression
        && material_improvement
        && candidate.p95_latency_ms <= baseline.p95_latency_ms.saturating_mul(2)
        && candidate.peak_memory_bytes <= baseline.peak_memory_bytes.saturating_mul(2)
        && candidate.peak_disk_bytes <= baseline.peak_disk_bytes.saturating_mul(2)
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
