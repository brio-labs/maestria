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
            for route in corpus.route_configurations.keys() {
                routes.insert(*route, metrics::aggregate(&cases, *route, observations)?);
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

    /// Classes where the hybrid (lexical + dense) route beats the lexical
    /// route with complete telemetry. The dense lane's promotion is global
    /// (v0.5 semantics): the daemon serves the dense fusion when at least
    /// one class is eligible and no eligible class regresses.
    pub fn hybrid_winning_classes(&self) -> Vec<LearnedSparseQueryClass> {
        self.classes
            .iter()
            .filter(|(class, comparison)| {
                super::metrics::hybrid_serving_eligible(**class, &comparison.routes)
            })
            .map(|(class, _)| *class)
            .collect()
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
    /// Validates every promotion rule, naming the first violated rule.
    ///
    /// The daemon and CLI call this boundary before persisting or activating
    /// a record; `is_valid()` remains the fast gate for in-crate policy
    /// decisions.
    pub fn validate(&self) -> Result<(), LearnedSparseBenchmarkError> {
        let invalid =
            |rule: &'static str| LearnedSparseBenchmarkError::InvalidPromotion(rule.to_string());
        if self.evaluation_id.trim().is_empty() {
            return Err(invalid("evaluation_id must be non-empty"));
        }
        if self.evaluation_date.trim().is_empty() {
            return Err(invalid("evaluation_date must be non-empty"));
        }
        if self.corpus_id.trim().is_empty() {
            return Err(invalid("corpus_id must be non-empty"));
        }
        if self.corpus_revision.trim().is_empty() {
            return Err(invalid("corpus_revision must be non-empty"));
        }
        if self.judgment_set_id.trim().is_empty() {
            return Err(invalid("judgment_set_id must be non-empty"));
        }
        if ContentHash::new(self.source_input_hash.clone()).is_err() {
            return Err(invalid("source_input_hash must be a SHA-256 content hash"));
        }
        if self.judgment_set_hash.is_none() {
            return Err(invalid("judgment_set_hash must be present"));
        }
        if !self.final_evaluation {
            return Err(invalid("final_evaluation must be true"));
        }
        if self.data_fidelity != LearnedSparseDataFidelity::RealMaestriaTask {
            return Err(invalid("data_fidelity must be RealMaestriaTask"));
        }
        self.environment
            .validate()
            .map_err(|_| invalid("environment is incomplete"))?;
        self.identity
            .validate()
            .map_err(|_| invalid("identity is incomplete"))?;
        if self.identity.corpus_snapshot.value() == 0 {
            return Err(invalid("identity corpus snapshot must be positive"));
        }
        if self.identity.index_generation.value() == 0 {
            return Err(invalid("identity index generation must be positive"));
        }
        if self.route_configuration.route != LearnedSparseRoute::SparseFused {
            return Err(invalid(
                "route_configuration must be the sparse-fused route",
            ));
        }
        self.route_configuration
            .validate()
            .map_err(|_| invalid("route_configuration is incomplete"))?;
        if self.rollback_target.index_generation.value() == 0 {
            return Err(invalid("rollback target index generation must be positive"));
        }
        if self.rollback_target.route == LearnedSparseRoute::SparseFused {
            return Err(invalid(
                "rollback target must not be the sparse-fused route",
            ));
        }
        for class in LearnedSparseQueryClass::all() {
            if !self.decisions.contains_key(&class) {
                return Err(invalid("decisions must cover every query class"));
            }
            if !self.budgets.contains_key(&class) {
                return Err(invalid("budgets must cover every query class"));
            }
            if !self.class_final_real.contains_key(&class) {
                return Err(invalid("class_final_real must cover every query class"));
            }
        }
        for (class, decision) in &self.decisions {
            if matches!(decision, LearnedSparseClassDecision::PromoteSparseFused)
                && self.class_final_real.get(class) != Some(&true)
            {
                return Err(invalid(
                    "a promoted class must be final-evaluation real-task measured",
                ));
            }
        }
        let protected_promotion = self.decisions.iter().any(|(class, decision)| {
            matches!(
                class,
                LearnedSparseQueryClass::ExactLiteral
                    | LearnedSparseQueryClass::NoEvidence
                    | LearnedSparseQueryClass::Security
            ) && matches!(decision, LearnedSparseClassDecision::PromoteSparseFused)
        });
        if protected_promotion {
            return Err(invalid(
                "protected query classes cannot be promoted to sparse fusion",
            ));
        }
        if !self
            .decisions
            .values()
            .any(|decision| matches!(decision, LearnedSparseClassDecision::PromoteSparseFused))
        {
            return Err(invalid(
                "a promotion record must promote at least one class",
            ));
        }
        Ok(())
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.validate().is_ok()
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

pub(super) fn validate_observations(
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
        for route in corpus.route_configurations.keys() {
            if !seen.contains(&(case.case_id.clone(), *route)) {
                return Err(LearnedSparseBenchmarkError::MissingObservation {
                    case_id: case.case_id.clone(),
                    route: *route,
                });
            }
        }
    }
    Ok(())
}
