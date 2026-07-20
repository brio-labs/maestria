use super::*;
use crate::{
    CorpusScope, EvidenceRequirements, FreshnessRequirement, ModalitySet,
    RetrievalModelFingerprint, SearchBudget, SearchIntent, SearchPlan, StopConditions,
    ids::{CorpusSnapshotId, IndexGenerationId, QueryId},
};

#[test]
fn test_deterministic_id_with_diversity() -> Result<(), SearchCompatibilityError> {
    let plan = SearchPlan {
        query_id: QueryId::new(1),
        original_query: "test".to_string(),
        intent: SearchIntent::ExactLookup,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(1),
        index_generation: IndexGenerationId::new(1),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![]),
        stages: vec![],
        budgets: SearchBudget::new(1000, 1000)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: vec![],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("test".to_string())?,
        original_intent: None,
        route_decision: None,
    };

    let mut trace = SearchTrace::from_plan(
        &plan,
        vec![],
        &[],
        vec![],
        None,
        vec![],
        SearchStopReason::EvidenceComplete,
    );
    let id1 = trace.deterministic_id();
    trace.diversity = Some(crate::SearchTraceDiversity {
        distinct_sources: 1,
        distinct_documents: 1,
        distinct_sections: 1,
        required_claims: vec!["claim1".to_string()],
        required_subquestions: vec![],
        covered_keys: vec!["claim1".to_string()],
        stop_reason: SearchStopReason::EvidenceComplete,
        candidates: vec![],
    });
    let id2 = trace.deterministic_id();
    assert_ne!(id1, id2);
    let diversity = trace
        .diversity
        .as_mut()
        .ok_or(SearchCompatibilityError::TracePlanMismatch(
            "diversity trace fixture missing",
        ))?;
    diversity
        .candidates
        .push(crate::SearchTraceDiversityCandidate {
            candidate_id: crate::ids::EvidenceId::new(1),
            original_rank: 0,
            selected_rank: Some(0),
            duplicate_cluster: None,
            marginal_coverage: 1,
            coverage_keys: vec!["key1".to_string()],
        });
    let id3 = trace.deterministic_id();
    assert_ne!(id2, id3);
    Ok(())
}
