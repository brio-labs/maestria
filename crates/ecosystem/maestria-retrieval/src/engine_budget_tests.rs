use super::lane_budget;
use maestria_domain::{
    CorpusScope, DEFAULT_CORPUS_SNAPSHOT_ID, EvidenceRequirements, FreshnessRequirement,
    IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalPolicySnapshot, SearchBudget, SearchCompatibilityError, SearchExecutionUsage,
    SearchIntent, SearchPlan, SearchStage, StopConditions,
};

fn plan(max_results: u32) -> Result<SearchPlan, SearchCompatibilityError> {
    SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query("lane budget".to_string())
        .intent(SearchIntent::FactualLocal)
        .scope(CorpusScope::Global)
        .corpus_snapshot(DEFAULT_CORPUS_SNAPSHOT_ID)
        .index_generation(IndexGenerationId::new(1))
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Text]))
        .stages(vec![SearchStage::InitialRetrieval])
        .budgets(SearchBudget::with_resource_limits(
            1_000, 30_000, 8, 1, 0, 0, 1,
        )?)
        .stop_conditions(StopConditions {
            max_results,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        })
        .fingerprint(RetrievalModelFingerprint::new(
            "lane-budget-test".to_string(),
        )?)
        .authorization(RetrievalPolicySnapshot::global_default())
        .build()
}

#[test]
fn lane_budget_keeps_full_result_ceiling_for_every_lane() -> Result<(), Box<dyn std::error::Error>>
{
    let plan = plan(3)?;
    let global = plan.execution_budget()?;
    for lane in 0..3 {
        let allocation = lane_budget(&plan, SearchExecutionUsage::default(), 3, lane)
            .ok_or("lane budget was not allocated")?;
        assert_eq!(
            allocation.max_results(),
            global.max_results(),
            "lane {lane} must retain the plan's full result ceiling"
        );
    }
    Ok(())
}

#[test]
fn lane_budget_partitions_consumable_resources_across_lanes()
-> Result<(), Box<dyn std::error::Error>> {
    let plan = plan(3)?;
    let global = plan.execution_budget()?;
    let allocation = lane_budget(&plan, SearchExecutionUsage::default(), 3, 0)
        .ok_or("lane budget was not allocated")?;
    let base = global.max_candidates() / 3;
    let remainder = u64::from(global.max_candidates() % 3 > 0);
    assert_eq!(allocation.max_candidates(), base + remainder);
    assert_eq!(
        allocation.max_work_units(),
        global.max_work_units() / 3 + u64::from(global.max_work_units() % 3 > 0),
        "remainder of work partition lands on lane 0"
    );
    assert!(allocation.max_candidates() < global.max_candidates());
    assert!(allocation.max_work_units() < global.max_work_units());
    Ok(())
}
