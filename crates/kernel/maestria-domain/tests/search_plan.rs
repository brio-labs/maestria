use std::error::Error;

use maestria_domain::{
    CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement, IndexGenerationId,
    Modality, ModalitySet, QueryId, RetrievalModelFingerprint, SearchBudget, SearchIntent,
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
        budgets: SearchBudget::with_limits(100, 1000, 2, 1, 0)?,
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

#[test]
fn every_canonical_intent_classifies_deterministically() {
    let cases = [
        ("\"exact identifier\"", SearchIntent::ExactLookup),
        (
            "what changed in the local project",
            SearchIntent::FactualLocal,
        ),
        ("find related ideas", SearchIntent::SemanticDiscovery),
        (
            "what must be true and where is the constraint",
            SearchIntent::CompositionalConstraints,
        ),
        (
            "how does the parser relate to the index",
            SearchIntent::MultiHop,
        ),
        (
            "summarize the corpus across projects",
            SearchIntent::CorpusSynthesis,
        ),
        (
            "Rust function in the repository",
            SearchIntent::RepositoryCode,
        ),
        ("find the chart in the PDF", SearchIntent::VisualDocument),
        (
            "when did the previous decision change",
            SearchIntent::TemporalMemory,
        ),
        ("what is the latest web guidance", SearchIntent::CurrentWeb),
        (
            "audit contradictory and disputed evidence",
            SearchIntent::ContradictionAudit,
        ),
    ];
    for (query, expected) in cases {
        assert_eq!(SearchIntent::classify(query), expected, "query: {query}");
        assert_eq!(
            SearchIntent::classify(query),
            expected,
            "classification must be stable"
        );
    }
    assert_eq!(
        SearchIntent::classify("trust policy for local notes"),
        SearchIntent::FactualLocal
    );
}

#[test]
fn schema_rejects_empty_query_and_repeated_stage() -> Result<(), Box<dyn Error>> {
    let mut candidate = plan()?;
    candidate.original_query.clear();
    assert!(candidate.validate_schema().is_err());

    let mut candidate = plan()?;
    candidate.stages = vec![SearchStage::InitialRetrieval, SearchStage::InitialRetrieval];
    candidate.budgets = SearchBudget::with_limits(100, 1000, 2, 2, 0)?;
    assert!(candidate.validate_schema().is_err());

    let mut candidate = plan()?;
    candidate.stages = vec![
        SearchStage::InitialRetrieval,
        SearchStage::Filtering,
        SearchStage::Reranking,
    ];
    candidate.budgets = SearchBudget::with_limits(100, 1000, 3, 3, 0)?;
    assert!(candidate.validate_schema().is_err());
    Ok(())
}

#[test]
fn schema_accepts_multi_stage_plan_with_explicit_budget() -> Result<(), Box<dyn Error>> {
    let mut candidate = plan()?;
    candidate.stages = vec![SearchStage::InitialRetrieval, SearchStage::Reranking];
    candidate.budgets = SearchBudget::with_limits(100, 1000, 2, 2, 0)?;
    assert!(candidate.validate_schema().is_ok());
    Ok(())
}
