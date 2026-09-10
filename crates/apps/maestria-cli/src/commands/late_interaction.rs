use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::content_hash;
use maestria_retrieval::{LateInteractionStageAPromotionRecord, LateInteractionStageAReport};
use maestria_storage_sqlite::SqliteStore;
use std::path::PathBuf;

use crate::cli_types::LateInteractionCommands;

pub fn run(command: LateInteractionCommands) -> Result<()> {
    match command {
        LateInteractionCommands::Show {
            instance_dir,
            stage,
        } => run_show(instance_dir, &stage),
        LateInteractionCommands::Set {
            instance_dir,
            record,
        } => run_set(instance_dir, record),
        LateInteractionCommands::Rollback {
            instance_dir,
            evaluation_id,
        } => run_rollback(instance_dir, &evaluation_id),
    }
}

fn run_show(instance_dir: PathBuf, stage: &str) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let store = SqliteStore::open_read_only(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let report = store
        .load_latest_late_interaction_report(stage)
        .map_err(|error| anyhow!("load late-interaction {stage} report: {error}"))?
        .ok_or_else(|| anyhow!("no late-interaction {stage} report"))?;
    println!("{}", report.report_json);
    Ok(())
}

fn run_set(instance_dir: PathBuf, record_path: PathBuf) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let record_json = std::fs::read_to_string(&record_path)
        .with_context(|| format!("read promotion record {}", record_path.display()))?;
    let record: LateInteractionStageAPromotionRecord = serde_json::from_str(&record_json)
        .map_err(|error| anyhow!("parse late-interaction promotion record: {error}"))?;
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let stored_stage_a = store
        .load_latest_late_interaction_report("stage-a")?
        .ok_or_else(|| anyhow!("a persisted Stage A report is required before promotion"))?;
    let stage_a: LateInteractionStageAReport = serde_json::from_str(&stored_stage_a.report_json)
        .map_err(|error| anyhow!("parse persisted Stage A report: {error}"))?;
    record
        .validate_against_report(&stage_a)
        .map_err(|error| anyhow!("late-interaction promotion record is invalid: {error}"))?;
    if record.stage_a_report_hash.as_str() != stored_stage_a.report_hash
        || record.evaluation_id != stored_stage_a.evaluation_id
    {
        return Err(anyhow!(
            "promotion record is not bound to the latest persisted Stage A report"
        ));
    }
    for existing in store.list_late_interaction_reports("promotion")? {
        if existing.evaluation_id == record.evaluation_id && existing.report_json != record_json {
            return Err(anyhow!(
                "promotion evaluation ID already exists with different content"
            ));
        }
    }
    let report_hash = content_hash(record_json.as_bytes());
    store.save_late_interaction_promotion_record(
        &record.corpus_id,
        &record.evaluation_id,
        &record.evaluation_date,
        &report_hash,
        &record_json,
    )?;
    println!(
        "Stored late-interaction Stage A promotion {} for {} class(es).",
        record.evaluation_id,
        record.promoted_classes.len()
    );
    Ok(())
}

fn run_rollback(instance_dir: PathBuf, evaluation_id: &str) -> Result<()> {
    if evaluation_id.trim().is_empty() {
        return Err(anyhow!(
            "late-interaction rollback requires an evaluation id"
        ));
    }
    let layout = InstanceLayout::for_root(instance_dir);
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    store
        .remove_late_interaction_promotion_record(evaluation_id)
        .context("remove late-interaction promotion record")?;
    println!(
        "Removed late-interaction promotion record {evaluation_id}; serving remains baseline/shadow."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rollback_removes_only_the_selected_late_promotion_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let layout = InstanceLayout::for_root(directory.path());
        crate::helpers::ensure_instance(directory.path().to_path_buf())?;
        let store = SqliteStore::open(&layout.database_path)?;
        store.save_late_interaction_promotion_record(
            "corpus-v1",
            "promotion-1",
            "2026-09-09",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "{\"authorized\":false}",
        )?;
        store.save_late_interaction_promotion_record(
            "corpus-v1",
            "promotion-2",
            "2026-09-09",
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "{\"authorized\":false}",
        )?;

        run_rollback(directory.path().to_path_buf(), "promotion-1")?;
        let remaining = store
            .load_latest_late_interaction_promotion_record()?
            .ok_or("late promotion record was unexpectedly removed")?;
        assert_eq!(remaining.evaluation_id, "promotion-2");
        Ok(())
    }

    #[test]
    fn set_requires_a_persisted_stage_a_report() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        crate::helpers::ensure_instance(directory.path().to_path_buf())?;
        let record_path = directory.path().join("promotion.json");
        let record = LateInteractionStageAPromotionRecord {
            schema_version: 1,
            evaluation_id: "stage-a".to_string(),
            evaluation_date: "2026-09-09".to_string(),
            corpus_id: "corpus-v1".to_string(),
            corpus_revision: "revision-v1".to_string(),
            stage_a_report_hash: maestria_domain::ContentHash::new(format!(
                "sha256:{}",
                "a".repeat(64)
            ))?,
            profile_identity:
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            generation_id: "generation-1".to_string(),
            corpus_snapshot: "snapshot-1".to_string(),
            rollback_generation_id: "generation-0".to_string(),
            promoted_classes: std::collections::BTreeSet::from([
                maestria_domain::SearchIntent::FactualLocal,
            ]),
            authorized: true,
        };
        std::fs::write(&record_path, serde_json::to_vec(&record)?)?;
        let error = run_set(directory.path().to_path_buf(), record_path)
            .err()
            .ok_or("promotion unexpectedly succeeded without Stage A")?;
        assert!(error.to_string().contains("Stage A report"));
        Ok(())
    }
}
