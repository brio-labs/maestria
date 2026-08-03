//! Visual retrieval evaluation and promotion (Rule 13: one concept per
//! module). The frozen benchmark evidence schema lives in the private
//! `schema` module, corpus parsing in `corpus`, aggregation in `metrics`,
//! and execution in `runner`; this module owns the comparison, promotion
//! record, and shadow-by-default execution policy.

#[path = "visual_benchmark_corpus.rs"]
mod corpus;
#[path = "visual_benchmark_metrics.rs"]
mod metrics;
#[path = "visual_benchmark_runner.rs"]
mod runner;
#[path = "visual_benchmark_schema.rs"]
mod schema;

pub use runner::{
    VisualBenchmarkExecutor, VisualProviderUnavailableExecutor, VisualTextLayoutExecutor,
    run_visual_benchmark,
};
pub use schema::{
    VisualBenchmarkCase, VisualBenchmarkCorpus, VisualBenchmarkObservation, VisualEvidenceKind,
    VisualEvidenceLocation, VisualJudgment, VisualProviderStatus, VisualQueryClass, VisualRoute,
    VisualRouteMetrics,
};

use serde::Serialize;
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
