use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{ContentHash, SearchIntent};
use serde::{Deserialize, Serialize};

use super::{
    LateInteractionBenchmarkCorpus, LateInteractionBenchmarkError, LateInteractionRoute,
    MATERIAL_QUALITY_DELTA, Measurement,
};
fn measured_gain(candidate: &Measurement<u32>, baseline: &Measurement<u32>) -> Option<i64> {
    match (candidate, baseline) {
        (Measurement::Measured(candidate), Measurement::Measured(baseline)) => {
            Some(i64::from(*candidate) - i64::from(*baseline))
        }
        _ => None,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateInteractionQualityMetrics {
    pub recall_at_5: Measurement<u32>,
    pub recall_at_20: Measurement<u32>,
    pub recall_at_50: Measurement<u32>,
    pub recall_at_100: Measurement<u32>,
    pub ndcg_at_10: Measurement<u32>,
    pub ndcg_at_20: Measurement<u32>,
    pub mrr_at_10: Measurement<u32>,
    pub exact_span_recall: Measurement<u32>,
    pub constraint_satisfaction: Measurement<u32>,
}

impl LateInteractionQualityMetrics {
    pub fn validate(&self) -> Result<(), LateInteractionBenchmarkError> {
        for measurement in [
            &self.recall_at_5,
            &self.recall_at_20,
            &self.recall_at_50,
            &self.recall_at_100,
            &self.ndcg_at_10,
            &self.ndcg_at_20,
            &self.mrr_at_10,
            &self.exact_span_recall,
            &self.constraint_satisfaction,
        ] {
            measurement
                .validate()
                .map_err(LateInteractionBenchmarkError::InvalidMeasurement)?;
        }
        Ok(())
    }

    fn primary_gain_over(&self, baseline: &Self) -> bool {
        measured_gain(&self.ndcg_at_10, &baseline.ndcg_at_10)
            .is_some_and(|delta| delta >= i64::from(MATERIAL_QUALITY_DELTA))
            || measured_gain(&self.exact_span_recall, &baseline.exact_span_recall)
                .is_some_and(|delta| delta >= i64::from(MATERIAL_QUALITY_DELTA))
            || measured_gain(
                &self.constraint_satisfaction,
                &baseline.constraint_satisfaction,
            )
            .is_some_and(|delta| delta >= i64::from(MATERIAL_QUALITY_DELTA))
    }

    fn has_regression_against(&self, baseline: &Self) -> bool {
        [
            (&self.recall_at_5, &baseline.recall_at_5),
            (&self.recall_at_20, &baseline.recall_at_20),
            (&self.recall_at_50, &baseline.recall_at_50),
            (&self.recall_at_100, &baseline.recall_at_100),
            (&self.ndcg_at_10, &baseline.ndcg_at_10),
            (&self.ndcg_at_20, &baseline.ndcg_at_20),
            (&self.mrr_at_10, &baseline.mrr_at_10),
            (&self.exact_span_recall, &baseline.exact_span_recall),
            (
                &self.constraint_satisfaction,
                &baseline.constraint_satisfaction,
            ),
        ]
        .iter()
        .any(|(candidate, reference)| {
            measured_gain(candidate, reference).is_some_and(|delta| delta < 0)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateInteractionResourceMetrics {
    pub end_to_end_p50_ms: Measurement<u64>,
    pub end_to_end_p95_ms: Measurement<u64>,
    pub scorer_p95_ms: Measurement<u64>,
    pub peak_memory_bytes: Measurement<u64>,
    pub model_storage_bytes: Measurement<u64>,
    pub index_storage_bytes: Measurement<u64>,
    pub energy_millijoules: Measurement<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateInteractionSafetyMetrics {
    pub acl_leaks: u64,
    pub secret_exposure: u64,
    pub quarantine_escape: u64,
    pub prompt_injection_fail_open: u64,
    pub protected_provider_calls: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateInteractionObservation {
    pub corpus_id: String,
    pub corpus_revision: String,
    pub case_id: String,
    pub query_class: SearchIntent,
    pub route: LateInteractionRoute,
    pub candidate_input_hash: ContentHash,
    pub quality: LateInteractionQualityMetrics,
    pub resources: LateInteractionResourceMetrics,
    pub safety: LateInteractionSafetyMetrics,
    pub measurement_status: Measurement<()>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LateInteractionClassDecision {
    RetainBaseline { reason: String },
    RetainLateInteractionReranker { reason: String },
    RejectExperiment { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateInteractionClassComparison {
    pub query_class: SearchIntent,
    pub baseline: LateInteractionQualityMetrics,
    pub reranker: LateInteractionQualityMetrics,
    pub decision: LateInteractionClassDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LateInteractionStageAComparison {
    pub corpus_id: String,
    pub corpus_revision: String,
    pub classes: BTreeMap<SearchIntent, LateInteractionClassComparison>,
}

impl LateInteractionStageAComparison {
    pub fn evaluate(
        corpus: &LateInteractionBenchmarkCorpus,
        observations: &[LateInteractionObservation],
    ) -> Result<Self, LateInteractionBenchmarkError> {
        corpus.validate()?;
        let (by_class, late_observations_by_class) =
            collect_stage_a_observations(corpus, observations)?;
        let corpus_classes = corpus
            .cases
            .iter()
            .map(|case| case.query_class)
            .collect::<BTreeSet<_>>();
        let mut class_comparisons = BTreeMap::new();
        for class in corpus_classes {
            let routes = by_class
                .get(&class)
                .ok_or(LateInteractionBenchmarkError::MissingClass(class))?;
            let baseline_items = routes
                .get(&LateInteractionRoute::EligibleBoundedBaseline)
                .or_else(|| routes.get(&LateInteractionRoute::EligibleHybrid))
                .ok_or(LateInteractionBenchmarkError::MissingBaseline(class))?;
            let reranker_items = routes
                .get(&LateInteractionRoute::LateInteractionReranker)
                .ok_or(LateInteractionBenchmarkError::MissingReranker(class))?;
            let baseline = aggregate_quality(baseline_items);
            let reranker = aggregate_quality(reranker_items);
            let operationally_eligible =
                late_observations_by_class
                    .get(&class)
                    .is_some_and(|observations| {
                        observations.iter().all(|observation| {
                            late_observation_is_operationally_eligible(corpus, observation)
                        })
                    });
            let decision = if class.is_protected() {
                LateInteractionClassDecision::RetainBaseline {
                    reason: "protected intent is never late-reranked".into(),
                }
            } else if !operationally_eligible {
                LateInteractionClassDecision::RetainBaseline {
                    reason: "late interaction lacks complete bounded resource or safety evidence"
                        .into(),
                }
            } else if reranker.primary_gain_over(&baseline)
                && !reranker.has_regression_against(&baseline)
            {
                LateInteractionClassDecision::RetainLateInteractionReranker {
                    reason: "material primary quality gain without quality regression".into(),
                }
            } else {
                LateInteractionClassDecision::RetainBaseline {
                    reason: "no measured material quality gain without regression".into(),
                }
            };
            class_comparisons.insert(
                class,
                LateInteractionClassComparison {
                    query_class: class,

                    baseline,
                    reranker,
                    decision,
                },
            );
        }
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.revision.clone(),
            classes: class_comparisons,
        })
    }

    pub fn winning_classes(&self) -> BTreeSet<SearchIntent> {
        self.classes
            .iter()
            .filter_map(|(class, comparison)| {
                matches!(
                    comparison.decision,
                    LateInteractionClassDecision::RetainLateInteractionReranker { .. }
                )
                .then_some(*class)
            })
            .collect()
    }
}
fn late_observation_is_operationally_eligible(
    corpus: &LateInteractionBenchmarkCorpus,
    observation: &LateInteractionObservation,
) -> bool {
    let Some(case) = corpus.case(&observation.case_id) else {
        return false;
    };
    let Some(end_to_end_p95_ms) = observation.resources.end_to_end_p95_ms.measured_value() else {
        return false;
    };
    observation.measurement_status.is_measured()
        && *end_to_end_p95_ms <= case.latency_budget_ms
        && observation.resources.peak_memory_bytes.is_measured()
        && observation.resources.model_storage_bytes.is_measured()
        && observation.resources.index_storage_bytes.is_measured()
        && observation.safety.acl_leaks == 0
        && observation.safety.secret_exposure == 0
        && observation.safety.quarantine_escape == 0
        && observation.safety.prompt_injection_fail_open == 0
        && observation.safety.protected_provider_calls == 0
}

fn aggregate_quality(items: &[LateInteractionQualityMetrics]) -> LateInteractionQualityMetrics {
    LateInteractionQualityMetrics {
        recall_at_5: aggregate_metric(items.iter().map(|item| &item.recall_at_5), "recall@5"),
        recall_at_20: aggregate_metric(items.iter().map(|item| &item.recall_at_20), "recall@20"),
        recall_at_50: aggregate_metric(items.iter().map(|item| &item.recall_at_50), "recall@50"),
        recall_at_100: aggregate_metric(items.iter().map(|item| &item.recall_at_100), "recall@100"),
        ndcg_at_10: aggregate_metric(items.iter().map(|item| &item.ndcg_at_10), "ndcg@10"),
        ndcg_at_20: aggregate_metric(items.iter().map(|item| &item.ndcg_at_20), "ndcg@20"),
        mrr_at_10: aggregate_metric(items.iter().map(|item| &item.mrr_at_10), "mrr@10"),
        exact_span_recall: aggregate_metric(
            items.iter().map(|item| &item.exact_span_recall),
            "exact span recall",
        ),
        constraint_satisfaction: aggregate_metric(
            items.iter().map(|item| &item.constraint_satisfaction),
            "constraint satisfaction",
        ),
    }
}

fn aggregate_metric<'a, I>(values: I, label: &str) -> Measurement<u32>
where
    I: Iterator<Item = &'a Measurement<u32>>,
{
    let mut total = 0_u64;
    let mut measured = 0_u64;
    let mut unavailable = false;
    let mut not_applicable = false;
    for value in values {
        match value {
            Measurement::Measured(value) => {
                total = total.saturating_add(u64::from(*value));
                measured = measured.saturating_add(1);
            }
            Measurement::Unavailable { .. } => unavailable = true,
            Measurement::NotApplicable { .. } => not_applicable = true,
        }
    }
    if measured > 0 && !unavailable && !not_applicable {
        let average = total / measured;
        return match u32::try_from(average) {
            Ok(value) => Measurement::measured(value),
            Err(_) => Measurement::measured(u32::MAX),
        };
    }
    if unavailable {
        Measurement::unavailable(format!("{label} has unavailable case measurements"))
    } else {
        Measurement::not_applicable(format!("{label} has no applicable case measurements"))
    }
}
type StageAObservationGroups<'a> = (
    BTreeMap<SearchIntent, BTreeMap<LateInteractionRoute, Vec<LateInteractionQualityMetrics>>>,
    BTreeMap<SearchIntent, Vec<&'a LateInteractionObservation>>,
);

fn collect_stage_a_observations<'a>(
    corpus: &LateInteractionBenchmarkCorpus,
    observations: &'a [LateInteractionObservation],
) -> Result<StageAObservationGroups<'a>, LateInteractionBenchmarkError> {
    let mut by_class: BTreeMap<
        SearchIntent,
        BTreeMap<LateInteractionRoute, Vec<LateInteractionQualityMetrics>>,
    > = BTreeMap::new();
    let mut late_observations_by_class: BTreeMap<
        SearchIntent,
        Vec<&'a LateInteractionObservation>,
    > = BTreeMap::new();
    let mut observation_keys = BTreeSet::new();
    for observation in observations {
        let case = corpus
            .case(&observation.case_id)
            .ok_or(LateInteractionBenchmarkError::ObservationMismatch)?;
        if observation.corpus_id != corpus.corpus_id
            || observation.corpus_revision != corpus.revision
            || observation.query_class != case.query_class
        {
            return Err(LateInteractionBenchmarkError::ObservationMismatch);
        }
        if !observation_keys.insert((observation.case_id.clone(), observation.route)) {
            return Err(LateInteractionBenchmarkError::ObservationMismatch);
        }
        observation.quality.validate()?;
        observation
            .measurement_status
            .validate()
            .map_err(LateInteractionBenchmarkError::InvalidMeasurement)?;
        if observation.route == LateInteractionRoute::LateInteractionReranker {
            late_observations_by_class
                .entry(observation.query_class)
                .or_default()
                .push(observation);
        }
        by_class
            .entry(observation.query_class)
            .or_default()
            .entry(observation.route)
            .or_default()
            .push(observation.quality.clone());
    }
    for case in &corpus.cases {
        for route in LateInteractionRoute::all_stage_a() {
            if !observation_keys.contains(&(case.case_id.clone(), route)) {
                return Err(LateInteractionBenchmarkError::ObservationMismatch);
            }
        }
    }
    Ok((by_class, late_observations_by_class))
}
