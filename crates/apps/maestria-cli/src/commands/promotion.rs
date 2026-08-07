use anyhow::{Context, Result, anyhow};
use maestria_retrieval::LearnedSparsePromotionRecord;
use maestria_storage_sqlite::SqliteStore;
use std::path::PathBuf;

use crate::helpers;

/// Loads, validates, and durably stores a promotion record for an instance.
///
/// Invalid records are rejected with the validation error printed; no
/// partially validated record is ever persisted.
pub fn run_set(instance_dir: PathBuf, record_path: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let contents = std::fs::read_to_string(&record_path).with_context(|| {
        format!("read promotion record {}", record_path.display())
    })?;
    let record: LearnedSparsePromotionRecord = serde_json::from_str(&contents)
        .map_err(|error| anyhow!("parse promotion record: {error}"))?;
    record
        .validate()
        .map_err(|error| anyhow!("promotion record is invalid: {error}"))?;

    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    store
        .save_promotion_record(
            &record.corpus_id,
            &record.evaluation_id,
            &record.evaluation_date,
            record.report_hash.as_str(),
            &contents,
        )
        .context("persist promotion record")?;
    let promoted = record
        .decisions
        .iter()
        .filter(|(_, decision)| {
            matches!(
                decision,
                maestria_retrieval::LearnedSparseClassDecision::PromoteSparseFused
            )
        })
        .map(|(class, _)| format!("{class:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "Stored promotion record {} (corpus {}); sparse-fused classes: {}",
        record.evaluation_id, record.corpus_id, promoted
    );
    Ok(())
}

pub fn run_remove(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let store = SqliteStore::open(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let removed = store
        .remove_all_promotion_records()
        .context("remove promotion records")?;
    if removed == 0 {
        println!("No promotion record to remove; the lexical/hybrid route is already restored.");
    } else {
        println!(
            "Removed {removed} promotion record(s); the lexical/hybrid route is restored."
        );
    }
    Ok(())
}

pub fn run_show(instance_dir: PathBuf) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let store = SqliteStore::open_read_only(&layout.database_path)
        .with_context(|| format!("open sqlite store {}", layout.database_path.display()))?;
    let Some(record) = store
        .load_latest_promotion_record()
        .context("load promotion record")?
    else {
        println!("no promotion record");
        return Ok(());
    };
    println!("{}", record.record_json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_core::InstanceLayout;
    use maestria_domain::{ContentHash, IndexGenerationId, SearchExecutionBudget};
    use maestria_ports::learned_sparse_contract_tests::fixture_sparse_identity;
    use maestria_retrieval::{
        LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkIdentity,
        LearnedSparseClassDecision, LearnedSparseDataFidelity, LearnedSparseEnvironment,
        LearnedSparseQueryClass, LearnedSparseRollbackTarget, LearnedSparseRoute,
        LearnedSparseRouteConfiguration,
    };
    use std::collections::BTreeMap;

    fn hash(digit: char) -> String {
        format!("sha256:{}", digit.to_string().repeat(64))
    }

    fn valid_record() -> Result<LearnedSparsePromotionRecord, Box<dyn std::error::Error>> {
        let sparse = fixture_sparse_identity()?;
        let mut decisions = BTreeMap::new();
        for class in LearnedSparseQueryClass::all() {
            let decision = if class == LearnedSparseQueryClass::VocabularyExpansion {
                LearnedSparseClassDecision::PromoteSparseFused
            } else if matches!(
                class,
                LearnedSparseQueryClass::ExactLiteral
                    | LearnedSparseQueryClass::NoEvidence
                    | LearnedSparseQueryClass::Security
            ) {
                LearnedSparseClassDecision::RetainLexical
            } else {
                LearnedSparseClassDecision::RetainHybrid
            };
            decisions.insert(class, decision);
        }
        let mut class_final_real = BTreeMap::new();
        let mut budgets = BTreeMap::new();
        for class in LearnedSparseQueryClass::all() {
            class_final_real.insert(class, true);
            budgets.insert(
                class,
                LearnedSparseBenchmarkBudget {
                    latency_ms: 250,
                    memory_bytes: 268_435_456,
                    disk_bytes: 536_870_912,
                    indexing_cost_micros: 5_000_000,
                    incremental_update_cost_micros: 5_000_000,
                    energy_millijoules: 5_000,
                },
            );
        }
        let record = LearnedSparsePromotionRecord {
            evaluation_id: "cli-test-evaluation".to_string(),
            evaluation_date: "2026-08-07".to_string(),
            corpus_id: "learned-sparse-task-corpus-v1".to_string(),
            corpus_revision: "v1".to_string(),
            judgment_set_id: "learned-sparse-judgments-v1".to_string(),
            source_input_hash: hash('1'),
            final_evaluation: true,
            class_final_real,
            judgment_set_hash: Some(ContentHash::new(hash('2'))?),
            environment: LearnedSparseEnvironment {
                operating_system: "linux".to_string(),
                architecture: "x86_64".to_string(),
                cpu_model: "documented benchmark host".to_string(),
                software_revision: "maestria-v0.6.1".to_string(),
                warmup_policy: "one warmup sample excluded from summaries".to_string(),
                sample_count: 5,
            },
            data_fidelity: LearnedSparseDataFidelity::RealMaestriaTask,
            identity: LearnedSparseBenchmarkIdentity::from_sparse_identity(
                &sparse,
                "sqlite-projection-v1",
            )?,
            route_configuration: LearnedSparseRouteConfiguration {
                route: LearnedSparseRoute::SparseFused,
                result_limit: 20,
                candidate_limit: 50,
                budget: SearchExecutionBudget::new(20, 50, 1_000, 0)?,
            },
            budgets,
            decisions,
            rollback_target: LearnedSparseRollbackTarget {
                route: LearnedSparseRoute::Hybrid,
                index_generation: IndexGenerationId::new(1),
            },
            report_hash: ContentHash::new(hash('3'))?,
        };
        record.validate()?;
        Ok(record)
    }

    fn instance() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        helpers::ensure_instance(directory.path().to_path_buf())?;
        Ok(directory)
    }

    #[test]
    fn promotion_set_show_remove_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let directory = instance()?;
        let record = valid_record()?;
        let record_path = directory.path().join("record.json");
        std::fs::write(
            &record_path,
            serde_json::to_vec_pretty(&record).map_err(anyhow::Error::from)?,
        )?;
        run_set(directory.path().to_path_buf(), record_path)?;
        let store = SqliteStore::open(&InstanceLayout::for_root(directory.path().to_path_buf()).database_path)?;
        let stored = store
            .load_latest_promotion_record()?
            .ok_or("promotion record was not persisted")?;
        assert_eq!(stored.evaluation_id, "cli-test-evaluation");
        run_remove(directory.path().to_path_buf())?;
        assert!(store.load_latest_promotion_record()?.is_none());
        Ok(())
    }

    #[test]
    fn promotion_set_rejects_invalid_record() -> Result<(), Box<dyn std::error::Error>> {
        let directory = instance()?;
        let mut record = valid_record()?;
        record.final_evaluation = false;
        let record_path = directory.path().join("invalid.json");
        std::fs::write(
            &record_path,
            serde_json::to_vec_pretty(&record).map_err(anyhow::Error::from)?,
        )?;
        let result = run_set(directory.path().to_path_buf(), record_path);
        assert!(
            result.is_err(),
            "an invalid record must be refused with a validation error"
        );
        let store = SqliteStore::open(&InstanceLayout::for_root(directory.path().to_path_buf()).database_path)?;
        assert!(store.load_latest_promotion_record()?.is_none());
        Ok(())
    }
}
