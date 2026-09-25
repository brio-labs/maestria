use super::RetrievalEngine;
use crate::rewrite::QueryRewriteSession;
use crate::types::{RetrievalError, RetrievalResult};
use maestria_domain::{
    CorpusSnapshotId, IndexGenerationId, Modality, RetrievalModelFingerprint, SearchIntent,
    SearchPlan, SearchRouteDecision,
};

/// Runtime inputs used to build a deterministic search plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPlannerContext {
    pub corpus_snapshot: CorpusSnapshotId,
    pub primary_generation: IndexGenerationId,
    pub fingerprint: RetrievalModelFingerprint,
    /// Scope the produced plan is confined to; `None` builds a global plan.
    pub scope: Option<maestria_domain::ScopeId>,
}

/// Build a rewrite session for the plan's query with the plan budgets.
pub fn rewrite_session(plan: &SearchPlan) -> QueryRewriteSession {
    let mut session = QueryRewriteSession::with_limits(
        plan.original_query(),
        plan.budgets().max_tokens() as usize,
        plan.budgets().max_latency_ms(),
        plan.budgets().max_queries(),
    );
    session.expand_deterministic();
    session
}

struct PlanOptions {
    expansion_enabled: bool,
    reranking_enabled: bool,
    budget_limits: maestria_domain::SearchBudgetLimits,
}

struct RouteParameters {
    intent: SearchIntent,
    modality: Modality,
    original_intent: Option<SearchIntent>,
    route_decision: Option<SearchRouteDecision>,
}

/// Maximum UTF-8 byte length accepted by an interactive local query.
pub const INTERACTIVE_MAX_QUERY_BYTES: usize = 512;
/// Maximum latency budget for an interactive local query.
pub const INTERACTIVE_MAX_LATENCY_MS: u32 = 100;

const INTERACTIVE_MAX_CANDIDATES: u32 = 100_000;
const INTERACTIVE_MAX_WORK_UNITS: u64 = 200_000;
const INTERACTIVE_MAX_BYTES_READ: u64 = 8 * 1024 * 1024;

fn general_budget_limits(
    max_stages: u32,
    web_limits: (u32, u64, u32),
) -> maestria_domain::SearchBudgetLimits {
    let (max_web_requests, max_bytes_read, max_concurrency) = web_limits;
    maestria_domain::SearchBudgetLimits {
        max_tokens: 1_000,
        max_latency_ms: 30_000,
        max_queries: 8,
        max_stages,
        max_web_requests,
        max_bytes_read,
        max_concurrency,
        max_candidates: 30_000,
        max_work_units: 30_000_000,
    }
}

fn interactive_budget_limits() -> maestria_domain::SearchBudgetLimits {
    maestria_domain::SearchBudgetLimits {
        max_tokens: 128,
        max_latency_ms: INTERACTIVE_MAX_LATENCY_MS,
        max_queries: 1,
        max_stages: 1,
        max_web_requests: 0,
        max_bytes_read: INTERACTIVE_MAX_BYTES_READ,
        max_concurrency: 1,
        max_candidates: INTERACTIVE_MAX_CANDIDATES,
        max_work_units: INTERACTIVE_MAX_WORK_UNITS,
    }
}

fn build_plan(
    original_query: &str,
    limit: usize,
    context: &SearchPlannerContext,
    options: PlanOptions,
    route: RouteParameters,
    authorization: maestria_domain::RetrievalPolicySnapshot,
) -> RetrievalResult<SearchPlan> {
    let budgets = maestria_domain::SearchBudget::with_execution_limits(options.budget_limits)
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;
    let max_results = u32::try_from(limit)
        .map_err(|_| RetrievalError::InvalidResultLimit { limit })?
        .max(1);
    let mut stages = vec![maestria_domain::SearchStage::InitialRetrieval];
    if options.reranking_enabled {
        stages.push(maestria_domain::SearchStage::Reranking);
    }
    if options.expansion_enabled {
        stages.push(maestria_domain::SearchStage::Filtering);
    }
    SearchPlan::builder()
        .query_id(maestria_domain::QueryId::from_query_text(original_query))
        .original_query(original_query.to_string())
        .intent(route.intent)
        .scope(
            context
                .scope
                .map_or(maestria_domain::CorpusScope::Global, |scope| {
                    maestria_domain::CorpusScope::Restricted(vec![scope])
                }),
        )
        .corpus_snapshot(context.corpus_snapshot)
        .index_generation(context.primary_generation)
        .freshness(if route.intent == SearchIntent::CurrentWeb {
            maestria_domain::FreshnessRequirement::Realtime
        } else {
            maestria_domain::FreshnessRequirement::Any
        })
        .modalities(match route.intent {
            SearchIntent::VisualDocument => {
                maestria_domain::ModalitySet::new(vec![Modality::Text, Modality::Image])
            }
            _ => maestria_domain::ModalitySet::new(vec![route.modality]),
        })
        .stages(stages)
        .budgets(budgets)
        .stop_conditions(maestria_domain::StopConditions {
            max_results,
            min_score_threshold: 0,
        })
        .evidence_requirements(maestria_domain::EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        })
        .fingerprint(context.fingerprint.clone())
        .authorization(authorization)
        .original_intent(route.original_intent)
        .route_decision(route.route_decision)
        .build()
        .map_err(RetrievalError::Compatibility)
}

impl RetrievalEngine {
    fn authorization_snapshot(
        &self,
        context: &SearchPlannerContext,
    ) -> RetrievalResult<maestria_domain::RetrievalPolicySnapshot> {
        let scope = context
            .scope
            .map_or(maestria_domain::CorpusScope::Global, |scope| {
                maestria_domain::CorpusScope::Restricted(vec![scope])
            });
        self.security_policy
            .authorization_context(&scope)
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval authorization denied: {error:?}"))
            })?
            .policy_snapshot()
            .map_err(|error| {
                RetrievalError::Internal(format!("retrieval policy snapshot invalid: {error}"))
            })
    }

    pub fn plan(
        &self,
        query: impl Into<String>,
        limit: usize,
        context: &SearchPlannerContext,
    ) -> RetrievalResult<SearchPlan> {
        let authorization = self.authorization_snapshot(context)?;
        let original_query = query.into();
        if maestria_governance::contains_prompt_injection_risk(&original_query) {
            return build_plan(
                &original_query,
                limit,
                context,
                PlanOptions {
                    expansion_enabled: false,
                    reranking_enabled: false,
                    budget_limits: general_budget_limits(1, (0, 0, 1)),
                },
                RouteParameters {
                    intent: SearchIntent::FactualLocal,
                    modality: Modality::Text,
                    original_intent: None,
                    route_decision: None,
                },
                authorization,
            );
        }
        let (options, route, inferred_intent) = self.select_plan_options(&original_query);
        let inferred_plan = build_plan(
            &original_query,
            limit,
            context,
            options,
            route,
            authorization,
        )?;
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(context.corpus_snapshot);
        let policy = maestria_governance::RetrievalSecurityPolicy::default();
        match maestria_governance::SearchPlanValidator::validate(
            &inferred_plan,
            &capabilities,
            &policy,
        ) {
            Ok(()) => Ok(inferred_plan),
            Err(error) => self.try_fallback_plan(
                &original_query,
                limit,
                context,
                inferred_plan,
                error,
                inferred_intent,
            ),
        }
    }
    /// Builds the bounded local lexical plan used by interactive passage queries.
    ///
    /// Unlike `plan`, this route never selects expansion, reranking, web,
    /// dense, repository-code, or sparse work; execution further restricts the
    /// registered retrievers to the text lexical lane.
    pub fn plan_interactive(
        &self,
        query: impl Into<String>,
        limit: usize,
        context: &SearchPlannerContext,
    ) -> RetrievalResult<SearchPlan> {
        let original_query = query.into();
        if original_query.trim().is_empty() {
            return Err(RetrievalError::InvalidQuery(
                "interactive query must not be empty".to_string(),
            ));
        }
        if original_query.len() > INTERACTIVE_MAX_QUERY_BYTES {
            return Err(RetrievalError::InvalidQuery(format!(
                "interactive query exceeds {INTERACTIVE_MAX_QUERY_BYTES} bytes"
            )));
        }
        if !(1..=100).contains(&limit) {
            return Err(RetrievalError::InvalidQuery(
                "interactive result limit must be between 1 and 100".to_string(),
            ));
        }

        let inferred_intent = SearchIntent::classify(&original_query);
        let (intent, original_intent, route_decision) = match inferred_intent {
            SearchIntent::ExactLookup | SearchIntent::FactualLocal => (inferred_intent, None, None),
            other => (
                SearchIntent::FactualLocal,
                Some(other),
                Some(SearchRouteDecision::LocalTextFallback),
            ),
        };
        let plan = build_plan(
            &original_query,
            limit,
            context,
            PlanOptions {
                expansion_enabled: false,
                reranking_enabled: false,
                budget_limits: interactive_budget_limits(),
            },
            RouteParameters {
                intent,
                modality: Modality::Text,
                original_intent,
                route_decision,
            },
            self.authorization_snapshot(context)?,
        )?;
        let validation_plan = if plan.route_decision().is_some() {
            plan.clone()
                .with_original_query("interactive local text query".to_string())
                .map_err(RetrievalError::Compatibility)?
        } else {
            plan.clone()
        };
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(context.corpus_snapshot);
        maestria_governance::SearchPlanValidator::validate(
            &validation_plan,
            &capabilities,
            &maestria_governance::RetrievalSecurityPolicy::default(),
        )
        .map_err(RetrievalError::SearchPlan)?;
        Ok(plan)
    }

    fn select_plan_options(
        &self,
        original_query: &str,
    ) -> (PlanOptions, RouteParameters, SearchIntent) {
        let inferred_intent = SearchIntent::classify(original_query);
        let inferred_modality = match inferred_intent {
            SearchIntent::RepositoryCode => Modality::Code,
            SearchIntent::VisualDocument => Modality::Image,
            SearchIntent::CurrentWeb => Modality::Web,
            _ => Modality::Text,
        };
        let expansion_enabled = self.expander.is_some();
        let reranking_enabled = self.reranker.is_some()
            && inferred_intent == SearchIntent::VisualDocument
            && self.visual_execution_policy.allows_visual(original_query);
        let max_stages = 1 + u32::from(expansion_enabled) + u32::from(reranking_enabled);
        let web_limits = if inferred_intent == SearchIntent::CurrentWeb {
            (3, 1_000_000, 3)
        } else {
            (0, 0, 1)
        };
        let options = PlanOptions {
            expansion_enabled,
            reranking_enabled,
            budget_limits: general_budget_limits(max_stages, web_limits),
        };
        let route = RouteParameters {
            intent: inferred_intent,
            modality: inferred_modality,
            original_intent: None,
            route_decision: None,
        };
        (options, route, inferred_intent)
    }

    fn try_fallback_plan(
        &self,
        original_query: &str,
        limit: usize,
        context: &SearchPlannerContext,
        _inferred_plan: SearchPlan,
        inferred_error: maestria_governance::SearchPlanValidationError,
        inferred_intent: SearchIntent,
    ) -> RetrievalResult<SearchPlan> {
        let fallback_eligible = !matches!(
            inferred_intent,
            SearchIntent::ExactLookup | SearchIntent::RepositoryCode
        ) && matches!(
            inferred_error,
            maestria_governance::SearchPlanValidationError::UnsupportedIntent(_)
                | maestria_governance::SearchPlanValidationError::UnsupportedModality(_)
                | maestria_governance::SearchPlanValidationError::WebCapabilityMissing
        );
        if !fallback_eligible {
            return Err(RetrievalError::SearchPlan(inferred_error));
        }
        let fallback_decision = match &inferred_error {
            maestria_governance::SearchPlanValidationError::UnsupportedIntent(intent) => {
                SearchRouteDecision::UnsupportedIntent { intent: *intent }
            }
            maestria_governance::SearchPlanValidationError::UnsupportedModality(modality) => {
                SearchRouteDecision::UnsupportedModality {
                    modality: *modality,
                }
            }
            maestria_governance::SearchPlanValidationError::WebCapabilityMissing => {
                SearchRouteDecision::MissingWebCapability
            }
            _ => SearchRouteDecision::LocalTextFallback,
        };
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(context.corpus_snapshot);
        let policy = maestria_governance::RetrievalSecurityPolicy::default();
        let fallback_plan = build_plan(
            original_query,
            limit,
            context,
            PlanOptions {
                expansion_enabled: false,
                reranking_enabled: false,
                budget_limits: general_budget_limits(1, (0, 0, 1)),
            },
            RouteParameters {
                intent: SearchIntent::FactualLocal,
                modality: Modality::Text,
                original_intent: Some(inferred_intent),
                route_decision: Some(fallback_decision),
            },
            self.authorization_snapshot(context)?,
        )?;
        let validation_plan = fallback_plan
            .clone()
            .with_original_query("fallback local text retrieval".to_string())
            .map_err(RetrievalError::Compatibility)?;
        maestria_governance::SearchPlanValidator::validate(
            &validation_plan,
            &capabilities,
            &policy,
        )
        .map_err(RetrievalError::SearchPlan)?;
        Ok(fallback_plan)
    }
}
#[cfg(test)]
#[path = "planner_tests.rs"]
mod tests;
