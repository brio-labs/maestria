#[path = "late_interaction_benchmark_corpus.rs"]
mod corpus;
#[path = "late_interaction_benchmark_errors.rs"]
mod errors;
#[path = "late_interaction_benchmark_metrics.rs"]
mod metrics;
#[path = "late_interaction_benchmark_reports.rs"]
mod reports;
#[path = "late_interaction_benchmark_runner.rs"]
mod runner;
#[path = "late_interaction_benchmark_stage_b.rs"]
mod stage_b;
#[cfg(test)]
#[path = "late_interaction_benchmark_tests.rs"]
mod tests;

pub use crate::benchmark_common::Measurement;
pub use corpus::{
    LateInteractionBenchmarkCase, LateInteractionBenchmarkCorpus, LateInteractionBenchmarkJudgment,
    LateInteractionRoute, LateInteractionStage,
};
pub use errors::LateInteractionBenchmarkError;
pub use metrics::{
    LateInteractionClassComparison, LateInteractionClassDecision, LateInteractionObservation,
    LateInteractionQualityMetrics, LateInteractionResourceMetrics, LateInteractionSafetyMetrics,
    LateInteractionStageAComparison,
};
pub use reports::{
    LateInteractionStageAFingerprints, LateInteractionStageAPromotionRecord,
    LateInteractionStageAReport, LateInteractionStageAReportCorpus,
    LateInteractionStageAReportPromotion,
};
pub use runner::{LateInteractionBenchmarkExecutor, run_late_interaction_stage_a};
pub use stage_b::{
    IndexedRetrievalNeed, LateInteractionStageBDecision, LateInteractionStageBPromotion,
    LateInteractionStageBReport, decide_stage_b,
};

pub const LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION: u32 = 1;
pub const MATERIAL_QUALITY_DELTA: u32 = 500;
