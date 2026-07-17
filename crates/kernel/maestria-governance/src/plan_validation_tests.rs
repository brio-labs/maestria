use std::error::Error;

use super::*;
use maestria_domain::{
    CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement, IndexGenerationId,
    Modality, ModalitySet, QueryId, RetrievalModelFingerprint, ScopeId, SearchBudget, SearchIntent,
    SearchPlan, SearchStage, StopConditions,
};

fn plan() -> Result<SearchPlan, Box<dyn Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "find local notes".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(100, 1000)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new("test:v1".to_string())?,
    })
}

fn capabilities() -> SearchCapabilities {
    SearchCapabilities::core_defaults(
        CorpusSnapshotId::new(1),
        IndexGenerationId::new(1),
        (100, 1000),
    )
}

#[test]
fn accepts_plan_with_matching_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    let candidate = plan()?;
    assert!(
        SearchPlanValidator::validate(
            &candidate,
            &capabilities(),
            &RetrievalSecurityPolicy::default()
        )
        .is_ok()
    );
    Ok(())
}
#[test]
fn rejects_unsupported_stage_and_budget() -> Result<(), Box<dyn std::error::Error>> {
    let mut candidate = plan()?;
    candidate.stages.push(SearchStage::Reranking);
    candidate.budgets = SearchBudget::with_limits(101, 1000, 1, 2, 0)?;
    assert!(matches!(
        SearchPlanValidator::validate(
            &candidate,
            &capabilities(),
            &RetrievalSecurityPolicy::default()
        ),
        Err(SearchPlanValidationError::UnsupportedStage(
            SearchStage::Reranking
        ))
    ));
    Ok(())
}

#[test]
fn rejects_web_without_web_capability() -> Result<(), Box<dyn std::error::Error>> {
    let mut candidate = plan()?;
    candidate.intent = SearchIntent::CurrentWeb;
    candidate.original_query = "latest notes".to_string();
    candidate.budgets = SearchBudget::with_limits(100, 1000, 1, 1, 1)?;
    let web_capabilities = capabilities()
        .with_intent(SearchIntent::CurrentWeb)
        .with_modality(Modality::Web)
        .max_budgets(100, 1000, 1, 1, 1);
    assert!(matches!(
        SearchPlanValidator::validate(
            &candidate,
            &web_capabilities,
            &RetrievalSecurityPolicy::default()
        ),
        Err(SearchPlanValidationError::WebCapabilityMissing)
    ));
    Ok(())
}
#[test]
fn rejects_scope_count_with_typed_error() -> Result<(), Box<dyn std::error::Error>> {
    let mut candidate = plan()?;
    candidate.scope = CorpusScope::Restricted(vec![ScopeId::new(1), ScopeId::new(2)]);
    let capabilities = capabilities().max_scope_ids(1);
    assert!(matches!(
        SearchPlanValidator::validate(
            &candidate,
            &capabilities,
            &RetrievalSecurityPolicy::default()
        ),
        Err(SearchPlanValidationError::TooManyScopes {
            requested: 2,
            allowed: 1
        })
    ));
    Ok(())
}
