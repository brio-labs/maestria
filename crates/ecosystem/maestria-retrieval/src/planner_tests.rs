use super::*;
use maestria_domain::{CorpusSnapshotId, IndexGenerationId, RetrievalModelFingerprint};

#[test]
fn visual_document_plan_requests_text_and_visual_modalities()
-> Result<(), Box<dyn std::error::Error>> {
    let context = SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(3),
        primary_generation: IndexGenerationId::new(7),
        fingerprint: RetrievalModelFingerprint::new("test:visual".to_string())?,
        scope: None,
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
        maestria_domain::RetrievalPolicySnapshot::global_default(),
    )?;
    assert_eq!(
        plan.modalities().values(),
        &[Modality::Text, Modality::Image]
    );
    Ok(())
}

#[test]
fn visual_plan_can_request_bounded_reranking_stage() -> Result<(), Box<dyn std::error::Error>> {
    let context = SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(3),
        primary_generation: IndexGenerationId::new(7),
        fingerprint: RetrievalModelFingerprint::new("test:visual".to_string())?,
        scope: None,
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
        maestria_domain::RetrievalPolicySnapshot::global_default(),
    )?;
    assert_eq!(
        plan.stages(),
        &[
            maestria_domain::SearchStage::InitialRetrieval,
            maestria_domain::SearchStage::Reranking,
        ]
    );
    Ok(())
}

#[cfg(target_pointer_width = "64")]
#[test]
fn result_limit_overflow_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let context = SearchPlannerContext {
        corpus_snapshot: CorpusSnapshotId::new(3),
        primary_generation: IndexGenerationId::new(7),
        fingerprint: RetrievalModelFingerprint::new("test:limit".to_string())?,
        scope: None,
    };
    let result = build_plan(
        "bounded query",
        usize::MAX,
        &context,
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
        maestria_domain::RetrievalPolicySnapshot::global_default(),
    );
    assert!(matches!(
        result,
        Err(RetrievalError::InvalidResultLimit { limit: usize::MAX })
    ));
    Ok(())
}
