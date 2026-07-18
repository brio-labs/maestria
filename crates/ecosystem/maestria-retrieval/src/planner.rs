use super::{RetrievalEngine, SearchPlannerContext};
use crate::types::{RetrievalError, RetrievalResult};
use maestria_domain::{Modality, SearchIntent, SearchPlan};

fn build_plan(
    original_query: &str,
    limit: usize,
    context: &SearchPlannerContext,
    max_stages: u32,
    intent: SearchIntent,
    modality: Modality,
    web_limits: (u32, u64, u32),
) -> RetrievalResult<SearchPlan> {
    let (web_requests, web_bytes, web_concurrency) = web_limits;
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
    let stages = if max_stages == 2 {
        vec![
            maestria_domain::SearchStage::InitialRetrieval,
            maestria_domain::SearchStage::Filtering,
        ]
    } else {
        vec![maestria_domain::SearchStage::InitialRetrieval]
    };
    Ok(SearchPlan {
        query_id: maestria_domain::QueryId::new(1),
        original_query: original_query.to_string(),
        intent,
        scope: maestria_domain::CorpusScope::Global,
        corpus_snapshot: context.corpus_snapshot,
        index_generation: context.primary_generation,
        freshness: if intent == SearchIntent::CurrentWeb {
            maestria_domain::FreshnessRequirement::Realtime
        } else {
            maestria_domain::FreshnessRequirement::Any
        },
        modalities: maestria_domain::ModalitySet::new(vec![modality]),
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
        let inferred_intent = SearchIntent::classify(&original_query);
        let inferred_modality = match inferred_intent {
            SearchIntent::RepositoryCode => Modality::Code,
            SearchIntent::VisualDocument => Modality::Image,
            SearchIntent::CurrentWeb => Modality::Web,
            _ => Modality::Text,
        };
        let max_stages = if self.expander.is_some() { 2 } else { 1 };
        let capabilities = self
            .capabilities
            .clone()
            .with_snapshot(context.corpus_snapshot);
        let policy = maestria_governance::RetrievalSecurityPolicy::default();
        let (web_requests, web_bytes, web_concurrency) =
            if inferred_intent == SearchIntent::CurrentWeb {
                (3, 1_000_000, 3)
            } else {
                (0, 0, 1)
            };
        let inferred_plan = build_plan(
            &original_query,
            limit,
            context,
            max_stages,
            inferred_intent,
            inferred_modality,
            (web_requests, web_bytes, web_concurrency),
        )?;
        let inferred_error = match maestria_governance::SearchPlanValidator::validate(
            &inferred_plan,
            &capabilities,
            &policy,
        ) {
            Ok(()) => return Ok(inferred_plan),
            Err(error) => error,
        };
        let fallback_eligible = matches!(
            inferred_intent,
            SearchIntent::CurrentWeb | SearchIntent::VisualDocument
        ) && matches!(
            inferred_error,
            maestria_governance::SearchPlanValidationError::UnsupportedIntent(
                SearchIntent::CurrentWeb
            ) | maestria_governance::SearchPlanValidationError::UnsupportedIntent(
                SearchIntent::VisualDocument
            ) | maestria_governance::SearchPlanValidationError::UnsupportedModality(_)
                | maestria_governance::SearchPlanValidationError::WebCapabilityMissing
        );
        if !fallback_eligible {
            return Err(RetrievalError::SearchPlan(inferred_error));
        }
        let fallback_plan = build_plan(
            &original_query,
            limit,
            context,
            max_stages,
            SearchIntent::FactualLocal,
            Modality::Text,
            (0, 0, 1),
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
