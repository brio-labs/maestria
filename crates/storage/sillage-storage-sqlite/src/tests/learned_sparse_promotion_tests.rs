use crate::SqliteStore;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn record_json(evaluation_id: &str) -> String {
    format!(
        "{{\"evaluation_id\":\"{evaluation_id}\",\"decisions\":{{\"VocabularyExpansion\":\"PromoteSparseFused\"}}}}"
    )
}

#[test]
fn promotion_records_survive_restart_and_replace_by_evaluation_id() -> TestResult {
    let store = SqliteStore::in_memory()?;
    store.save_promotion_record(
        "corpus-v1",
        "eval-1",
        "2026-08-07",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        &record_json("eval-1"),
    )?;
    store.save_promotion_record(
        "corpus-v1",
        "eval-2",
        "2026-08-07",
        "sha256:2222222222222222222222222222222222222222222222222222222222222222",
        &record_json("eval-2"),
    )?;

    let latest = store
        .load_latest_promotion_record()?
        .ok_or("latest promotion record is missing")?;
    assert_eq!(latest.evaluation_id, "eval-2");
    assert_eq!(latest.corpus_id, "corpus-v1");
    assert_eq!(latest.evaluation_date, "2026-08-07");
    assert_eq!(latest.report_hash.len(), 71);

    // Saving the same evaluation id replaces the row instead of duplicating.
    store.save_promotion_record(
        "corpus-v1",
        "eval-1",
        "2026-08-07",
        "sha256:3333333333333333333333333333333333333333333333333333333333333333",
        &record_json("eval-1"),
    )?;
    let all = store.list_promotion_records()?;
    assert_eq!(all.len(), 2);

    store.remove_promotion_record("eval-2")?;
    let latest = store
        .load_latest_promotion_record()?
        .ok_or("latest promotion record is missing")?;
    assert_eq!(latest.evaluation_id, "eval-1");
    assert_eq!(latest.record_json, record_json("eval-1"));
    Ok(())
}

#[test]
fn promotion_record_remove_all_reports_count() -> TestResult {
    let store = SqliteStore::in_memory()?;
    for index in 0..3 {
        store.save_promotion_record(
            "corpus-v1",
            &format!("eval-{index}"),
            "2026-08-07",
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            &record_json(&format!("eval-{index}")),
        )?;
    }
    let removed = store.remove_all_promotion_records()?;
    assert_eq!(removed, 3);
    assert!(store.load_latest_promotion_record()?.is_none());
    Ok(())
}

#[test]
fn empty_store_has_no_latest_promotion_record() -> TestResult {
    let store = SqliteStore::in_memory()?;
    assert!(store.load_latest_promotion_record()?.is_none());
    assert!(store.list_promotion_records()?.is_empty());
    Ok(())
}

#[test]
fn promotion_record_rejects_empty_identity_fields() -> TestResult {
    let store = SqliteStore::in_memory()?;
    let result = store.save_promotion_record(
        "corpus-v1",
        "",
        "2026-08-07",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        &record_json("eval-1"),
    );
    assert!(
        result.is_err_and(|error| error.is_invalid_input()),
        "empty evaluation id must be rejected"
    );
    Ok(())
}
