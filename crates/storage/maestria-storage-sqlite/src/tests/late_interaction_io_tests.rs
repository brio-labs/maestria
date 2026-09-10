use crate::SqliteStore;

#[test]
fn late_interaction_reports_round_trip_and_rollback() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    store.save_late_interaction_report(
        "stage-a",
        "corpus-v1",
        "evaluation-1",
        "2026-09-09",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "{\"stage\":\"stage-a\"}",
    )?;
    store.save_late_interaction_report(
        "stage-b",
        "corpus-v1",
        "evaluation-1",
        "2026-09-09",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "{\"stage\":\"stage-b\"}",
    )?;

    let stage_a = store
        .load_latest_late_interaction_report("stage-a")?
        .ok_or("Stage A report was not stored")?;
    assert_eq!(stage_a.evaluation_id, "evaluation-1");
    assert_eq!(stage_a.report_json, "{\"stage\":\"stage-a\"}");
    assert_eq!(store.list_late_interaction_reports("stage-b")?.len(), 1);
    store.save_late_interaction_promotion_record(
        "corpus-v1",
        "promotion-1",
        "2026-09-09",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "{\"authorized\":false}",
    )?;
    let promotion = store
        .load_latest_late_interaction_promotion_record()?
        .ok_or("late promotion record was not stored")?;
    assert_eq!(promotion.stage, "promotion");
    store.remove_late_interaction_promotion_record("promotion-1")?;
    assert!(
        store
            .load_latest_late_interaction_promotion_record()?
            .is_none()
    );

    store.remove_late_interaction_report("stage-b", "evaluation-1")?;
    assert!(
        store
            .load_latest_late_interaction_report("stage-b")?
            .is_none()
    );
    Ok(())
}

#[test]
fn late_interaction_reports_reject_invalid_stage_and_oversized_json()
-> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::in_memory()?;
    let invalid_stage = store.save_late_interaction_report(
        "stage-c",
        "corpus-v1",
        "evaluation-1",
        "2026-09-09",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "{}",
    );
    assert!(invalid_stage.is_err());

    let oversized = "x".repeat(32 * 1024 * 1024 + 1);
    let oversized_result = store.save_late_interaction_report(
        "stage-a",
        "corpus-v1",
        "evaluation-2",
        "2026-09-09",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        &oversized,
    );
    assert!(oversized_result.is_err());
    assert!(
        store
            .load_latest_late_interaction_report("stage-a")?
            .is_none()
    );
    Ok(())
}
