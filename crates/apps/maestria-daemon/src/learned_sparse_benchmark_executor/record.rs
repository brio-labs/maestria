//! Benchmark instrumentation promotion records.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use maestria_domain::{ContentHash, IndexGenerationId};
use maestria_retrieval::{
    LearnedSparseBenchmarkBudget, LearnedSparseBenchmarkIdentity, LearnedSparseClassDecision,
    LearnedSparsePromotionRecord, LearnedSparseQueryClass, LearnedSparseRollbackTarget,
    LearnedSparseRoute,
};

use super::LearnedSparseBenchmarkExecutor;

impl LearnedSparseBenchmarkExecutor {
    /// A valid instrumentation record promoting exactly one class.
    ///
    /// Protected classes cannot be promoted by policy; for their fused-route
    /// observations the record promotes an eligible class so the sparse lane
    /// stays eligible in the engine while the query itself routes hybrid.
    pub(super) fn active_record(
        &self,
        class: LearnedSparseQueryClass,
    ) -> Result<LearnedSparsePromotionRecord> {
        let promoted = if matches!(
            class,
            LearnedSparseQueryClass::ExactLiteral
                | LearnedSparseQueryClass::NoEvidence
                | LearnedSparseQueryClass::Security
        ) {
            LearnedSparseQueryClass::VocabularyExpansion
        } else {
            class
        };
        let mut decisions = BTreeMap::new();
        let mut class_final_real = BTreeMap::new();
        let mut budgets = BTreeMap::new();
        for candidate in LearnedSparseQueryClass::all() {
            let decision = if candidate == promoted {
                LearnedSparseClassDecision::PromoteSparseFused
            } else if matches!(
                candidate,
                LearnedSparseQueryClass::ExactLiteral
                    | LearnedSparseQueryClass::NoEvidence
                    | LearnedSparseQueryClass::Security
            ) {
                LearnedSparseClassDecision::RetainLexical
            } else {
                LearnedSparseClassDecision::RetainHybrid
            };
            decisions.insert(candidate, decision);
            class_final_real.insert(candidate, true);
            budgets.insert(candidate, self.budget_for_class(candidate)?);
        }
        let lane = self
            .sparse
            .as_ref()
            .ok_or_else(|| anyhow!("sparse lane is unavailable"))?;
        let benchmark_identity = LearnedSparseBenchmarkIdentity::from_sparse_identity(
            &lane.identity,
            super::BACKEND_FINGERPRINT,
        )?;
        let report_hash = ContentHash::new(maestria_domain::content_hash(
            format!("learned-sparse-benchmark-{class:?}").as_bytes(),
        ))
        .map_err(|error| anyhow!("invalid benchmark report hash: {error}"))?;
        let record = LearnedSparsePromotionRecord {
            evaluation_id: format!("benchmark-instrumentation-{class:?}"),
            evaluation_date: self.corpus.evaluation_date.clone(),
            corpus_id: self.corpus.corpus_id.clone(),
            corpus_revision: self.corpus.corpus_revision.clone(),
            judgment_set_id: self.corpus.judgment_set_id.clone(),
            source_input_hash: self.corpus.source_input_hash.clone(),
            final_evaluation: true,
            class_final_real,
            judgment_set_hash: self.corpus.judgment_set_hash.clone(),
            environment: self.corpus.environment.clone(),
            data_fidelity: self.corpus.data_fidelity,
            identity: benchmark_identity,
            route_configuration: self
                .corpus
                .route_configurations
                .get(&LearnedSparseRoute::SparseFused)
                .cloned()
                .ok_or_else(|| anyhow!("sparse-fused route configuration is missing"))?,
            budgets,
            decisions,
            rollback_target: LearnedSparseRollbackTarget {
                route: LearnedSparseRoute::Hybrid,
                index_generation: IndexGenerationId::new(1),
            },
            report_hash,
        };
        record
            .validate()
            .map_err(|error| anyhow!("benchmark instrumentation record is invalid: {error}"))?;
        Ok(record)
    }

    fn budget_for_class(
        &self,
        class: LearnedSparseQueryClass,
    ) -> Result<LearnedSparseBenchmarkBudget> {
        let case = self
            .corpus
            .cases
            .iter()
            .find(|case| case.class == class)
            .ok_or_else(|| anyhow!("corpus has no case for query class {class:?}"))?;
        Ok(LearnedSparseBenchmarkBudget {
            latency_ms: case.latency_budget_ms,
            memory_bytes: case.memory_budget_bytes,
            disk_bytes: case.disk_budget_bytes,
            indexing_cost_micros: case.ingest_update_budget_ms.saturating_mul(1_000),
            incremental_update_cost_micros: case.ingest_update_budget_ms.saturating_mul(1_000),
            energy_millijoules: case.energy_budget_millijoules,
        })
    }
}
