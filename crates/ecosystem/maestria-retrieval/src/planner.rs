use super::{RetrievalEngine, SearchPlannerContext};
use crate::types::{RetrievalError, RetrievalResult};
use maestria_domain::{Modality, SearchIntent, SearchPlan};

struct PlanOptions {
    max_stages: u32,
    expansion_enabled: bool,
    reranking_enabled: bool,
    web_limits: (u32, u64, u32),
}

struct RouteParameters {
    intent: SearchIntent,
    modality: Modality,
    original_intent: Option<SearchIntent>,
    route_decision: Option<String>,
}

fn build_plan(
    original_query: &str,
    limit: usize,
    context: &SearchPlannerContext,
    options: PlanOptions,
    route: RouteParameters,
) -> RetrievalResult<SearchPlan> {
    let (web_requests, web_bytes, web_concurrency) = options.web_limits;
    let max_stages = options.max_stages;
    let expansion_enabled = options.expansion_enabled;
    let reranking_enabled = options.reranking_enabled;
    let budgets = maestria_domain::SearchBudget::with_resource_limits(
        1_000,
        30_000,
        8,
        max_stages,
        web_requests,
        web_bytes,
        web_concurrency,
    )
    .map_err(|error| RetrievalError::Internal(error.to_string()))?;
    let mut stages = vec![maestria_domain::SearchStage::InitialRetrieval];
    if reranking_enabled {
        stages.push(maestria_domain::SearchStage::Reranking);
    }
    if expansion_enabled {
        stages.push(maestria_domain::SearchStage::Filtering);
    }
    Ok(SearchPlan {
        query_id: maestria_domain::QueryId::new(1),
        original_query: original_query.to_string(),
        intent: route.intent,
        scope: maestria_domain::CorpusScope::Global,
        corpus_snapshot: context.corpus_snapshot,
        index_generation: context.primary_generation,
        freshness: if route.intent == SearchIntent::CurrentWeb {
            maestria_domain::FreshnessRequirement::Realtime
        } else {
            maestria_domain::FreshnessRequirement::Any
        },
        modalities: match route.intent {
            SearchIntent::VisualDocument => {
                maestria_domain::ModalitySet::new(vec![Modality::Text, Modality::Image])
            }
            _ => maestria_domain::ModalitySet::new(vec![route.modality]),
        },
        stages,
        budgets,
        stop_conditions: maestria_domain::StopConditions {
            max_results: match u32::try_from(limit) {
                Ok(value) => value.max(1),
                Err(_) => u32::MAX,
            },
            min_score_threshold: 0,
        },
        evidence_requirements: maestria_domain::EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: context.fingerprint.clone(),
        original_intent: route.original_intent,
        route_decision: route.route_decision,
    })
}
impl RetrievalEngine {
    pub fn plan(
        &self,
        query: impl Into<String>,
        limit: usize,
        context: &SearchPlannerContext,
    ) -> RetrievalResult<SearchPlan> {
        let original_query = query.into();
        if maestria_governance::contains_prompt_injection_risk(&original_query) {
            return build_plan(
                &original_query,
                limit,
                context,
                PlanOptions {
                    max_stages: 1,
                    expansion_enabled: false,
                    reranking_enabled: false,
                    web_limits: (0, 0, 1),
                },
                RouteParameters {
                    intent: SearchIntent::FactualLocal,
                    modality: Modality::Text,
                    original_intent: None,
                    route_decision: None,
                },
            );
        }
        let (options, route, inferred_intent) = self.select_plan_options(&original_query);
        let inferred_plan = build_plan(&original_query, limit, context, options, route)?;
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
        let (web_requests, web_bytes, web_concurrency) =
            if inferred_intent == SearchIntent::CurrentWeb {
                (3, 1_000_000, 3)
            } else {
                (0, 0, 1)
            };
        let options = PlanOptions {
            max_stages,
            expansion_enabled,
            reranking_enabled,
            web_limits: (web_requests, web_bytes, web_concurrency),
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
        let fallback_reason = match &inferred_error {
            maestria_governance::SearchPlanValidationError::UnsupportedIntent(intent) => {
                format!(
                    "governed fallback to local text retrieval for unavailable {intent:?} intent"
                )
            }
            maestria_governance::SearchPlanValidationError::UnsupportedModality(modality) => {
                format!(
                    "governed fallback to local text retrieval for unsupported {modality:?} modality"
                )
            }
            maestria_governance::SearchPlanValidationError::WebCapabilityMissing => {
                "governed fallback to local text retrieval: web capability missing".to_string()
            }
            _ => "governed fallback to local text retrieval".to_string(),
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
                max_stages: 1,
                expansion_enabled: false,
                reranking_enabled: false,
                web_limits: (0, 0, 1),
            },
            RouteParameters {
                intent: SearchIntent::FactualLocal,
                modality: Modality::Text,
                original_intent: Some(inferred_intent),
                route_decision: Some(fallback_reason),
            },
        )?;
        let mut validation_plan = fallback_plan.clone();
        validation_plan.original_query = "fallback local text retrieval".to_string();
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
mod tests {
    use super::*;
    use maestria_domain::{CorpusSnapshotId, IndexGenerationId, RetrievalModelFingerprint};

    #[test]
    fn visual_document_plan_requests_text_and_visual_modalities()
    -> Result<(), Box<dyn std::error::Error>> {
        let context = SearchPlannerContext {
            corpus_snapshot: CorpusSnapshotId::new(3),
            primary_generation: IndexGenerationId::new(7),
            fingerprint: RetrievalModelFingerprint::new("test:visual".to_string())?,
        };
        let plan = build_plan(
            "show the table in the visual PDF",
            5,
            &context,
            PlanOptions {
                max_stages: 1,
                expansion_enabled: false,
                reranking_enabled: false,
                web_limits: (0, 0, 1),
            },
            RouteParameters {
                intent: SearchIntent::VisualDocument,
                modality: Modality::Image,
                original_intent: None,
                route_decision: None,
            },
        )?;
        assert_eq!(plan.modalities.values(), &[Modality::Text, Modality::Image]);
        Ok(())
    }
    #[test]
    fn visual_plan_can_request_bounded_reranking_stage() -> Result<(), Box<dyn std::error::Error>> {
        let context = SearchPlannerContext {
            corpus_snapshot: CorpusSnapshotId::new(3),
            primary_generation: IndexGenerationId::new(7),
            fingerprint: RetrievalModelFingerprint::new("test:visual".to_string())?,
        };
        let plan = build_plan(
            "show the figure in the visual PDF",
            5,
            &context,
            PlanOptions {
                max_stages: 2,
                expansion_enabled: false,
                reranking_enabled: true,
                web_limits: (0, 0, 1),
            },
            RouteParameters {
                intent: SearchIntent::VisualDocument,
                modality: Modality::Image,
                original_intent: None,
                route_decision: None,
            },
        )?;
        assert_eq!(
            plan.stages,
            vec![
                maestria_domain::SearchStage::InitialRetrieval,
                maestria_domain::SearchStage::Reranking,
            ]
        );
        Ok(())
    }
}
