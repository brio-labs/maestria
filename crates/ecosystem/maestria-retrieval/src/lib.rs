pub mod adapters;

/// Responsibility map:
/// - `adapters`: module responsibility.
/// - `bounded_reranker`: module responsibility.
/// - `diversity`: module responsibility.
/// - `engine`: module responsibility.
/// - `fusion`: module responsibility.
/// - `golden`: module responsibility.
/// - `learned_sparse_benchmark`: module responsibility.
/// - `learned_sparse_policy`: module responsibility.
/// - `repository_benchmark`: module responsibility.
/// - `rewrite`: module responsibility.
/// - `sync`: module responsibility.
/// - `sync_engine`: module responsibility.
/// - `traits`: module responsibility.
/// - `types`: module responsibility.
/// - `visual_benchmark`: module responsibility.
/// - `visual_reranker`: module responsibility.
/// - `monotonic`: monotonic time abstraction.
pub mod bounded_reranker;
pub mod diversity;
pub mod engine;
pub mod fusion;
pub mod golden;
pub mod learned_sparse_benchmark;
pub mod learned_sparse_policy;
mod monotonic;
pub mod repository_benchmark;
pub mod rewrite;
mod sync;
mod sync_engine;
pub mod traits;
pub mod types;
pub mod visual_benchmark;
pub mod visual_reranker;
pub use monotonic::MonotonicInstant;

pub use engine::{
    LearnedSparseShadowCandidate, LearnedSparseShadowLane, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowObservation, LearnedSparseShadowRoute, LearnedSparseShadowStore,
    LearnedSparseShadowStoreError, RetrievalEngine, SearchPlannerContext,
};
pub use fusion::FixedKRrf;
pub use learned_sparse_benchmark::{
    LearnedSparseBenchmarkCase, LearnedSparseBenchmarkComparison, LearnedSparseBenchmarkCorpus,
    LearnedSparseBenchmarkError, LearnedSparseBenchmarkObservation, LearnedSparseClassComparison,
    LearnedSparsePromotionRecord, LearnedSparseQueryClass, LearnedSparseRoute,
    LearnedSparseRouteMetrics,
};
pub use learned_sparse_policy::LearnedSparseExecutionPolicy;
pub use repository_benchmark::{
    MeasurementStatus, RepositoryBenchmarkCase, RepositoryBenchmarkComparison,
    RepositoryBenchmarkCorpus, RepositoryBenchmarkError, RepositoryBenchmarkObservation,
    RepositoryClassComparison, RepositoryExecutionPolicy, RepositoryExpectedOutcome,
    RepositoryPromotionRecord, RepositoryQueryClass, RepositoryRoute, RepositoryRouteMetrics,
};
pub use sync::SyncPipeline;
pub use sync_engine::SyncRetrievalEngine;
pub use traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RerankScorer,
    RetrievalEvaluator,
};
pub use types::{
    ContextExpansion, ExpansionPolicy, HybridExecutionPolicy, HybridPromotionRecord,
    RerankConstraintScore, RerankLimits, RerankRequest, RerankResult, RerankScoreComponents,
    RerankScorerInput, RetrievalError, RetrievalMode, RetrievalResult,
};
pub use visual_benchmark::{
    VisualBenchmarkCase, VisualBenchmarkComparison, VisualBenchmarkCorpus, VisualBenchmarkError,
    VisualBenchmarkExecutor, VisualBenchmarkObservation, VisualClassComparison, VisualEvidenceKind,
    VisualEvidenceLocation, VisualExecutionPolicy, VisualJudgment, VisualPromotionRecord,
    VisualProviderStatus, VisualProviderUnavailableExecutor, VisualQueryClass, VisualRoute,
    VisualRouteMetrics, VisualTextLayoutExecutor, run_visual_benchmark,
};
pub use visual_reranker::{VisualReranker, VisualRerankerParts};
