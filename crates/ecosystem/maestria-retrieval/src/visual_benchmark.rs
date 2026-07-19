#[path = "visual_benchmark_metrics.rs"]
mod metrics;
#[path = "visual_benchmark_runner.rs"]
mod runner;

pub use runner::{VisualBenchmarkExecutor, run_visual_benchmark};

use crate::golden::Metric;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
pub(crate) fn visual_lane_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    visual_enabled: bool,
) -> bool {
    let descriptor_id = descriptor.id.to_ascii_lowercase();
    let is_visual =
        descriptor.modality.eq_ignore_ascii_case("image") || descriptor_id == "visual_page_regions";
    visual_enabled || !is_visual
}

/// Frozen visual query classes covered by the benchmark gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualQueryClass {
    Text,
    Table,
    Chart,
    Figure,
    Formula,
    ScannedPage,
}

impl VisualQueryClass {
    pub const fn all() -> [Self; 6] {
        [
            Self::Text,
            Self::Table,
            Self::Chart,
            Self::Figure,
            Self::Formula,
            Self::ScannedPage,
        ]
    }

    /// Classify only explicit visual-document query vocabulary.
    pub fn classify(query: &str) -> Option<Self> {
        let normalized = query.to_ascii_lowercase();
        let tokens = normalized
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .collect::<BTreeSet<_>>();
        let visual_document =
            tokens.contains("pdf") || tokens.contains("page") || tokens.contains("document");
        if tokens.contains("scanned") || tokens.contains("scan") || tokens.contains("ocr") {
            Some(Self::ScannedPage)
        } else if tokens.contains("formula")
            || tokens.contains("equation")
            || tokens.contains("mathematical")
        {
            Some(Self::Formula)
        } else if tokens.contains("table") {
            Some(Self::Table)
        } else if tokens.contains("chart") || tokens.contains("graph") {
            Some(Self::Chart)
        } else if tokens.contains("figure") || tokens.contains("diagram") {
            Some(Self::Figure)
        } else if visual_document && (tokens.contains("text") || tokens.contains("paragraph")) {
            Some(Self::Text)
        } else {
            None
        }
    }
}

/// Route compared by the frozen visual benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualRoute {
    TextLayout,
    Visual,
}

/// Page or region shape expected by a visual judgment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisualEvidenceKind {
    Page,
    Region,
}

/// One frozen page/region judgment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualJudgment {
    pub kind: VisualEvidenceKind,
    pub relevance: u8,
}

/// One frozen visual query and its resource budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualBenchmarkCase {
    pub case_id: String,
    pub class: VisualQueryClass,
    pub query: String,
    pub judgments: Vec<VisualJudgment>,
    pub latency_budget_ms: u64,
    pub memory_budget_bytes: u64,
    pub disk_budget_bytes: u64,
    pub energy_budget_millijoules: u64,
}

/// Versioned, frozen visual retrieval benchmark corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualBenchmarkCorpus {
    pub schema_version: u32,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub cases: Vec<VisualBenchmarkCase>,
}

impl VisualBenchmarkCorpus {
    pub fn from_json(input: &str) -> Result<Self, VisualBenchmarkError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| VisualBenchmarkError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), VisualBenchmarkError> {
        if self.schema_version == 0
            || self.corpus_id.trim().is_empty()
            || self.corpus_revision.trim().is_empty()
        {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "schema and corpus identity must be non-empty".to_string(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut classes = BTreeSet::new();
        let mut has_page = false;
        let mut has_region = false;
        for case in &self.cases {
            if case.case_id.trim().is_empty() || case.query.trim().is_empty() {
                return Err(VisualBenchmarkError::InvalidCorpus(
                    "case_id and query must be non-empty".to_string(),
                ));
            }
            if !ids.insert(case.case_id.clone()) {
                return Err(VisualBenchmarkError::DuplicateCase(case.case_id.clone()));
            }
            if case.judgments.is_empty()
                || case.latency_budget_ms == 0
                || case.memory_budget_bytes == 0
                || case.disk_budget_bytes == 0
                || case.energy_budget_millijoules == 0
            {
                return Err(VisualBenchmarkError::InvalidCorpus(format!(
                    "case {} must have judgments and positive budgets",
                    case.case_id
                )));
            }
            if VisualQueryClass::classify(&case.query) != Some(case.class) {
                return Err(VisualBenchmarkError::InvalidCorpus(format!(
                    "case {} query does not classify as {:?}",
                    case.case_id, case.class
                )));
            }
            if maestria_domain::SearchIntent::classify(&case.query)
                != maestria_domain::SearchIntent::VisualDocument
            {
                return Err(VisualBenchmarkError::InvalidCorpus(format!(
                    "case {} is not a visual-document query",
                    case.case_id
                )));
            }
            classes.insert(case.class);
            for judgment in &case.judgments {
                if judgment.relevance == 0 {
                    return Err(VisualBenchmarkError::InvalidCorpus(format!(
                        "case {} contains a zero-relevance judgment",
                        case.case_id
                    )));
                }
                has_page |= judgment.kind == VisualEvidenceKind::Page;
                has_region |= judgment.kind == VisualEvidenceKind::Region;
            }
        }
        for class in VisualQueryClass::all() {
            if !classes.contains(&class) {
                return Err(VisualBenchmarkError::MissingClass(class));
            }
        }
        if !has_page || !has_region {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "visual benchmark must contain page and region judgments".to_string(),
            ));
        }
        Ok(())
    }

    fn case(&self, case_id: &str) -> Option<&VisualBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
}

/// Measurements for one case and one route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualBenchmarkObservation {
    pub corpus_id: String,
    pub corpus_revision: String,
    pub case_id: String,
    pub route: VisualRoute,
    pub page_region_recall: Metric,
    pub ndcg_at_10: Metric,
    pub citation_alignment: Metric,
    pub latency_ms: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub energy_millijoules: u64,
    pub privacy_violations: u32,
    pub security_violations: u32,
}

/// Aggregated metrics for one visual query class and route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualRouteMetrics {
    pub page_region_recall: Metric,
    pub ndcg_at_10: Metric,
    pub citation_alignment: Metric,
    pub p95_latency_ms: u64,
    pub peak_memory_bytes: u64,
    pub peak_disk_bytes: u64,
    pub energy_millijoules: u64,
    pub privacy_violations: u32,
    pub security_violations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VisualClassComparison {
    pub class: VisualQueryClass,
    pub text_layout: VisualRouteMetrics,
    pub visual: VisualRouteMetrics,
    pub visual_wins: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VisualBenchmarkComparison {
    corpus_id: String,
    corpus_revision: String,
    classes: BTreeMap<VisualQueryClass, VisualClassComparison>,
}

impl VisualBenchmarkComparison {
    pub fn evaluate(
        corpus: &VisualBenchmarkCorpus,
        observations: &[VisualBenchmarkObservation],
    ) -> Result<Self, VisualBenchmarkError> {
        corpus.validate()?;
        let mut seen = BTreeSet::new();
        for observation in observations {
            if observation.corpus_id != corpus.corpus_id
                || observation.corpus_revision != corpus.corpus_revision
            {
                return Err(VisualBenchmarkError::InvalidCorpus(
                    "observation identity does not match corpus".to_string(),
                ));
            }
            if corpus.case(&observation.case_id).is_none() {
                return Err(VisualBenchmarkError::UnknownCase(
                    observation.case_id.clone(),
                ));
            }
            if !seen.insert((observation.case_id.clone(), observation.route)) {
                return Err(VisualBenchmarkError::DuplicateObservation {
                    case_id: observation.case_id.clone(),
                    route: observation.route,
                });
            }
        }
        let mut classes = BTreeMap::new();
        for class in VisualQueryClass::all() {
            let cases = corpus
                .cases
                .iter()
                .filter(|case| case.class == class)
                .collect::<Vec<_>>();
            let text_layout =
                metrics::metrics_for(class, VisualRoute::TextLayout, &cases, observations)?;
            let visual = metrics::metrics_for(class, VisualRoute::Visual, &cases, observations)?;
            classes.insert(
                class,
                VisualClassComparison {
                    class,
                    visual_wins: metrics::wins(&cases, &text_layout, &visual, observations),
                    text_layout,
                    visual,
                },
            );
        }
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.corpus_revision.clone(),
            classes,
        })
    }

    pub fn corpus_id(&self) -> &str {
        &self.corpus_id
    }

    pub fn corpus_revision(&self) -> &str {
        &self.corpus_revision
    }

    pub fn classes(&self) -> &BTreeMap<VisualQueryClass, VisualClassComparison> {
        &self.classes
    }

    pub fn promotion(
        &self,
        evaluation_id: String,
    ) -> Result<VisualPromotionRecord, VisualBenchmarkError> {
        if evaluation_id.trim().is_empty() {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "evaluation_id must be non-empty".to_string(),
            ));
        }
        Ok(VisualPromotionRecord {
            evaluation_id,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            winning_classes: self
                .classes
                .values()
                .filter(|comparison| comparison.visual_wins)
                .map(|comparison| comparison.class)
                .collect(),
        })
    }
}

/// Benchmark evidence authorizing visual activation for selected classes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VisualPromotionRecord {
    evaluation_id: String,
    corpus_id: String,
    corpus_revision: String,
    winning_classes: BTreeSet<VisualQueryClass>,
}

impl VisualPromotionRecord {
    pub fn evaluation_id(&self) -> &str {
        &self.evaluation_id
    }

    pub fn corpus_id(&self) -> &str {
        &self.corpus_id
    }

    pub fn corpus_revision(&self) -> &str {
        &self.corpus_revision
    }

    pub fn winning_classes(&self) -> &BTreeSet<VisualQueryClass> {
        &self.winning_classes
    }

    fn is_valid(&self) -> bool {
        !self.evaluation_id.trim().is_empty()
            && !self.corpus_id.trim().is_empty()
            && !self.corpus_revision.trim().is_empty()
    }
}

/// Shadow-by-default policy for visual lane activation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VisualExecutionPolicy {
    #[default]
    Shadow,
    Active(VisualPromotionRecord),
}

impl VisualExecutionPolicy {
    pub fn route_for(&self, query: &str) -> VisualRoute {
        match self {
            Self::Active(record) if record.is_valid() => VisualQueryClass::classify(query)
                .filter(|class| record.winning_classes.contains(class))
                .map_or(VisualRoute::TextLayout, |_| VisualRoute::Visual),
            Self::Shadow | Self::Active(_) => VisualRoute::TextLayout,
        }
    }

    pub fn allows_visual(&self, query: &str) -> bool {
        self.route_for(query) == VisualRoute::Visual
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VisualBenchmarkError {
    #[error("invalid visual benchmark JSON: {0}")]
    InvalidJson(String),
    #[error("invalid visual benchmark corpus: {0}")]
    InvalidCorpus(String),
    #[error("visual benchmark is missing query class {0:?}")]
    MissingClass(VisualQueryClass),
    #[error("visual benchmark contains duplicate case {0}")]
    DuplicateCase(String),
    #[error("visual benchmark observation references unknown case {0}")]
    UnknownCase(String),
    #[error("visual benchmark has duplicate observation for case {case_id} on route {route:?}")]
    DuplicateObservation { case_id: String, route: VisualRoute },
    #[error("visual benchmark is missing observation for case {case_id} on route {route:?}")]
    MissingObservation { case_id: String, route: VisualRoute },
}
