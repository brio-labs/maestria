use super::*;

#[tokio::test]
async fn legacy_hybrid_promotion_remains_shadowed() -> Result<()> {
    let fixture = super::fixture()?;
    super::write_manifest_with_profiles(&fixture.layout)?;
    super::seed_lexical_generation(&fixture.layout)?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let record = sillage_retrieval::HybridPromotionRecord::new(
        "old-hybrid-dense".to_string(),
        "2026-08-09".to_string(),
        BTreeSet::from([sillage_retrieval::LearnedSparseQueryClass::DomainTerminology]),
        sillage_retrieval::HYBRID_SERVING_POLICY_ID,
    )
    .ok_or_else(|| anyhow::anyhow!("hybrid promotion record"))?;
    let mut legacy_record = serde_json::to_value(record)?;
    legacy_record
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("hybrid promotion record object"))?
        .remove("ranking_policy");
    store.save_hybrid_promotion_record(
        "corpus-1",
        "old-hybrid-dense",
        "2026-08-09",
        "old-report-hash",
        &serde_json::to_string(&legacy_record)?,
    )?;
    assert_eq!(
        crate::runtime_construction::hybrid_policy(&store),
        sillage_retrieval::HybridExecutionPolicy::Shadow
    );
    drop(store);

    let context = super::status_context(fixture.layout)?;
    let response = super::super::super::search_services::retrieval_status(&context).await?;
    assert_eq!(response.lanes.hybrid_state, "Shadow");
    assert!(response.lanes.hybrid_served_classes.is_empty());
    assert!(response.lanes.hybrid_ranking_policy_id.is_none());
    assert!(response.promotion_records.hybrid.is_some());
    Ok(())
}
