use std::collections::{BTreeMap, BTreeSet};

use maestria_domain::{ContentHash, IndexGenerationId};
use serde::{Deserialize, Serialize};

use super::metrics;
use super::{
    LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkCorpus, LearnedSparseBenchmarkError,
    LearnedSparseBenchmarkIdentity, LearnedSparseBenchmarkObservation, LearnedSparseDataFidelity,
    LearnedSparseDataSplit, LearnedSparseEnvironment, LearnedSparseQualityMetrics,
    LearnedSparseQueryClass, LearnedSparseResourceMetrics, LearnedSparseRoute,
    LearnedSparseRouteConfiguration, LearnedSparseSafetyMetrics,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LearnedSparseRouteMetrics {
    pub quality: LearnedSparseQualityMetrics,
    pub resources: LearnedSparseResourceMetrics,
    pub safety: LearnedSparseSafetyMetrics,
    pub budget_violations: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LearnedSparseClassComparison {
    pub class: LearnedSparseQueryClass,
    pub routes: BTreeMap<LearnedSparseRoute, LearnedSparseRouteMetrics>,
    pub winning_route: Option<LearnedSparseRoute>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LearnedSparseBenchmarkComparison {
    corpus_id: String,
    corpus_revision: String,
    judgment_set_id: String,
    source_input_hash: String,
    judgment_set_hash: Option<ContentHash>,
    evaluation_date: String,
    environment: LearnedSparseEnvironment,
    data_fidelity: LearnedSparseDataFidelity,
    final_evaluation: bool,
    class_final_real: BTreeMap<LearnedSparseQueryClass, bool>,
    classes: BTreeMap<LearnedSparseQueryClass, LearnedSparseClassComparison>,
    identities: BTreeMap<LearnedSparseRoute, LearnedSparseBenchmarkIdentity>,
    route_configurations: BTreeMap<LearnedSparseRoute, LearnedSparseRouteConfiguration>,
    budgets: BTreeMap<LearnedSparseQueryClass, LearnedSparseBenchmarkBudget>,
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
        let identities = observations
            .iter()
            .map(|observation| (observation.route, observation.identity.clone()))
            .collect();
        let final_evaluation = corpus
            .cases
            .iter()
            .all(|case| case.split == LearnedSparseDataSplit::FinalEvaluation)
            && corpus.data_fidelity == LearnedSparseDataFidelity::RealMaestriaTask
            && corpus
                .cases
                .iter()
                .any(|case| case.fidelity == LearnedSparseDataFidelity::RealMaestriaTask);
        let class_final_real = LearnedSparseQueryClass::all()
            .into_iter()
            .map(|class| {
                let mut count = 0_u32;
                let valid = corpus
                    .cases
                    .iter()
                    .filter(|case| case.class == class)
                    .all(|case| {
                        count = count.saturating_add(1);
                        case.split == LearnedSparseDataSplit::FinalEvaluation
                            && case.fidelity == LearnedSparseDataFidelity::RealMaestriaTask
                    })
                    && count > 0;
                (class, valid)
            })
            .collect();
        Ok(Self {
            corpus_id: corpus.corpus_id.clone(),
            corpus_revision: corpus.corpus_revision.clone(),
            judgment_set_id: corpus.judgment_set_id.clone(),
            source_input_hash: corpus.source_input_hash.clone(),
            judgment_set_hash: corpus.judgment_set_hash.clone(),
            evaluation_date: corpus.evaluation_date.clone(),
            environment: corpus.environment.clone(),
            data_fidelity: corpus.data_fidelity,
            final_evaluation,
            class_final_real,
            classes,
            identities,
            route_configurations: corpus.route_configurations.clone(),
            budgets: corpus
                .cases
                .iter()
                .map(|case| (case.class, case.budget()))
                .collect(),
        })
    }

    pub fn promotion(
        &self,
        evaluation_id: String,
        evaluation_date: String,
        rollback_target: LearnedSparseRollbackTarget,
        report_hash: ContentHash,
    ) -> Result<LearnedSparsePromotionRecord, LearnedSparseBenchmarkError> {
        let identity = self
            .identities
            .get(&LearnedSparseRoute::SparseFused)
            .ok_or_else(|| {
                LearnedSparseBenchmarkError::InvalidPromotion(
                    "sparse-fused identity is missing".to_string(),
                )
            })?;
        let route_configuration = self
            .route_configurations
            .get(&LearnedSparseRoute::SparseFused)
            .ok_or_else(|| {
                LearnedSparseBenchmarkError::InvalidPromotion(
                    "sparse-fused route configuration is missing".to_string(),
                )
            })?;
        if evaluation_id.trim().is_empty()
            || evaluation_date.trim().is_empty()
            || evaluation_date != self.evaluation_date
            || route_configuration.route != LearnedSparseRoute::SparseFused
        {
            return Err(LearnedSparseBenchmarkError::InvalidPromotion(
                "promotion identity and route configuration must match the evaluation".to_string(),
            ));
        }
        let mut decisions = BTreeMap::new();
        for class in LearnedSparseQueryClass::all() {
            let decision = self
                .classes
                .get(&class)
                .and_then(|comparison| comparison.winning_route)
                .filter(|route| *route == LearnedSparseRoute::SparseFused)
                .map_or_else(
                    || {
                        if matches!(
                            class,
                            LearnedSparseQueryClass::ExactLiteral
                                | LearnedSparseQueryClass::NoEvidence
                                | LearnedSparseQueryClass::Security
                        ) {
                            LearnedSparseClassDecision::RetainLexical
                        } else {
                            LearnedSparseClassDecision::RetainHybrid
                        }
                    },
                    |_| LearnedSparseClassDecision::PromoteSparseFused,
                );
            decisions.insert(class, decision);
        }
        Ok(LearnedSparsePromotionRecord {
            evaluation_id,
            evaluation_date,
            corpus_id: self.corpus_id.clone(),
            corpus_revision: self.corpus_revision.clone(),
            judgment_set_id: self.judgment_set_id.clone(),
            source_input_hash: self.source_input_hash.clone(),
            final_evaluation: self.final_evaluation,
            class_final_real: self.class_final_real.clone(),
            judgment_set_hash: self.judgment_set_hash.clone(),
            environment: self.environment.clone(),
            data_fidelity: self.data_fidelity,
            identity: identity.clone(),
            route_configuration: route_configuration.clone(),
            budgets: self.budgets.clone(),
            decisions,
            rollback_target,
            report_hash,
        })
    }

    pub fn classes(&self) -> &BTreeMap<LearnedSparseQueryClass, LearnedSparseClassComparison> {
        &self.classes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseClassDecision {
    PromoteSparseFused,
    RetainHybrid,
    RetainLexical,
    RemainShadowed,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseRollbackTarget {
    pub route: LearnedSparseRoute,
    pub index_generation: IndexGenerationId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedSparsePromotionRecord {
    pub evaluation_id: String,
    pub evaluation_date: String,
    pub corpus_id: String,
    pub corpus_revision: String,
    pub judgment_set_id: String,
    pub source_input_hash: String,
    pub final_evaluation: bool,
    pub class_final_real: BTreeMap<LearnedSparseQueryClass, bool>,
    pub judgment_set_hash: Option<ContentHash>,
    pub environment: LearnedSparseEnvironment,
    pub data_fidelity: LearnedSparseDataFidelity,
    pub identity: LearnedSparseBenchmarkIdentity,
    pub route_configuration: LearnedSparseRouteConfiguration,
    pub budgets: BTreeMap<LearnedSparseQueryClass, LearnedSparseBenchmarkBudget>,
    pub decisions: BTreeMap<LearnedSparseQueryClass, LearnedSparseClassDecision>,
    pub rollback_target: LearnedSparseRollbackTarget,
    pub report_hash: ContentHash,
}

impl LearnedSparsePromotionRecord {
    pub(crate) fn is_valid(&self) -> bool {
        let protected_promotion = self.decisions.iter().any(|(class, decision)| {
            matches!(
                class,
                LearnedSparseQueryClass::ExactLiteral
                    | LearnedSparseQueryClass::NoEvidence
                    | LearnedSparseQueryClass::Security
            ) && matches!(decision, LearnedSparseClassDecision::PromoteSparseFused)
        });
        !self.evaluation_id.trim().is_empty()
            && !self.evaluation_date.trim().is_empty()
            && !self.corpus_id.trim().is_empty()
            && !self.corpus_revision.trim().is_empty()
            && !self.judgment_set_id.trim().is_empty()
            && !self.source_input_hash.trim().is_empty()
            && ContentHash::new(self.source_input_hash.clone()).is_ok()
            && self.judgment_set_hash.is_some()
            && self.final_evaluation
            && self.data_fidelity == LearnedSparseDataFidelity::RealMaestriaTask
            && self.environment.validate().is_ok()
            && self.identity.validate().is_ok()
            && self.identity.corpus_snapshot.value() > 0
            && self.identity.index_generation.value() > 0
            && self.route_configuration.route == LearnedSparseRoute::SparseFused
            && self.route_configuration.validate().is_ok()
            && self.rollback_target.index_generation.value() > 0
            && self.rollback_target.route != LearnedSparseRoute::SparseFused
            && LearnedSparseQueryClass::all().iter().all(|class| {
                self.decisions.contains_key(class)
                    && self.budgets.contains_key(class)
                    && self.class_final_real.contains_key(class)
            })
            && self.decisions.iter().all(|(class, decision)| {
                !matches!(decision, LearnedSparseClassDecision::PromoteSparseFused)
                    || self.class_final_real.get(class) == Some(&true)
            })
            && !protected_promotion
            && self
                .decisions
                .values()
                .any(|decision| matches!(decision, LearnedSparseClassDecision::PromoteSparseFused))
    }

    pub fn winning_routes(&self) -> BTreeMap<LearnedSparseQueryClass, LearnedSparseRoute> {
        self.decisions
            .iter()
            .filter_map(|(class, decision)| {
                matches!(decision, LearnedSparseClassDecision::PromoteSparseFused)
                    .then_some((*class, LearnedSparseRoute::SparseFused))
            })
            .collect()
    }
}

fn all_routes() -> [LearnedSparseRoute; 4] {
    LearnedSparseRoute::all()
}

fn validate_observations(
    corpus: &LearnedSparseBenchmarkCorpus,
    observations: &[LearnedSparseBenchmarkObservation],
) -> Result<(), LearnedSparseBenchmarkError> {
    let mut seen = BTreeSet::new();
    let mut identities = BTreeMap::new();
    for observation in observations {
        observation.validate(corpus)?;
        if let Some(existing) = identities.insert(observation.route, observation.identity.clone())
            && existing != observation.identity
        {
            return Err(LearnedSparseBenchmarkError::InvalidIdentity(
                "route observations disagree on sparse/provider/backend identity".to_string(),
            ));
        }
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
