pub mod adapters;

/// Responsibility map:
/// - `adapters`: module responsibility.
/// - `bounded_reranker`: module responsibility.
/// - `diversity`: module responsibility.
/// - `engine`: module responsibility.
/// - `fusion`: module responsibility.
/// - `golden`: module responsibility.
/// - `learned_sparse_benchmark`: module responsibility.
/// - `learned_sparse_corpus`: module responsibility.
/// - `learned_sparse_policy`: module responsibility.
/// - `repository_benchmark`: module responsibility.
/// - `rewrite`: module responsibility.
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
pub mod learned_sparse_corpus;
pub mod learned_sparse_policy;
mod monotonic;
pub mod repository_benchmark;
pub mod rewrite;
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
pub use fusion::{FixedKRrf, NormalizedBlend};
pub use learned_sparse_benchmark::{
    CheckStatus, LearnedSparseAcceptedSpan, LearnedSparseBenchmarkBudget,
    LearnedSparseBenchmarkCase, LearnedSparseBenchmarkComparison, LearnedSparseBenchmarkCorpus,
    LearnedSparseBenchmarkError, LearnedSparseBenchmarkExecutor, LearnedSparseBenchmarkIdentity,
    LearnedSparseBenchmarkObservation, LearnedSparseClassComparison, LearnedSparseClassDecision,
    LearnedSparseDataFidelity, LearnedSparseDataSplit, LearnedSparseEnvironment,
    LearnedSparseExpectedOutcome, LearnedSparseOperationMeasurement, LearnedSparsePromotionRecord,
    LearnedSparseProviderDisclosure, LearnedSparseQualityMetrics, LearnedSparseQueryClass,
    LearnedSparseResourceMetrics, LearnedSparseRetentionPolicy, LearnedSparseRetrievedCandidate,
    LearnedSparseRetrievedSpan, LearnedSparseRollbackTarget, LearnedSparseRoute,
    LearnedSparseRouteConfiguration, LearnedSparseRouteMetrics, LearnedSparseSafetyMetrics,
    Measurement, run_learned_sparse_benchmark, score_case,
};
pub use learned_sparse_corpus::{
    LEARNED_SPARSE_TASK_CORPUS_SCHEMA_VERSION, LearnedSparseAbstentionReason,
    LearnedSparseAdjudicationRule, LearnedSparseCaseTag, LearnedSparseCitationExpectation,
    LearnedSparseCitationPolicy, LearnedSparseCorpusError, LearnedSparseEvidenceJudgment,
    LearnedSparseFreshnessExpectation, LearnedSparseJudgmentGuidance, LearnedSparseQueryLanguage,
    LearnedSparseRelevanceGrade, LearnedSparseRelevanceScale, LearnedSparseSecurityExpectation,
    LearnedSparseSourceInput, LearnedSparseSourceRole, LearnedSparseTaskCase,
    LearnedSparseTaskCorpus, LearnedSparseTaskExpectation,
};
pub use learned_sparse_policy::LearnedSparseExecutionPolicy;
pub use repository_benchmark::{
    MeasurementStatus, RepositoryBenchmarkCase, RepositoryBenchmarkComparison,
    RepositoryBenchmarkCorpus, RepositoryBenchmarkError, RepositoryBenchmarkObservation,
    RepositoryClassComparison, RepositoryExecutionPolicy, RepositoryExpectedOutcome,
    RepositoryPromotionRecord, RepositoryQueryClass, RepositoryRoute, RepositoryRouteMetrics,
};
pub use traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RerankScorer,
    RetrievalEvaluator,
};
pub use types::{
    CandidateSourceFilter, CandidateSourceFilterError, ContextExpansion, ExpansionPolicy,
    HybridExecutionPolicy, HybridPromotionRecord, RerankConstraintScore, RerankLimits,
    RerankRequest, RerankResult, RerankScoreComponents, RerankScorerInput, RetrievalError,
    RetrievalMode, RetrievalResult,
};
pub use visual_benchmark::{
    VisualBenchmarkCase, VisualBenchmarkComparison, VisualBenchmarkCorpus, VisualBenchmarkError,
    VisualBenchmarkExecutor, VisualBenchmarkObservation, VisualClassComparison, VisualEvidenceKind,
    VisualEvidenceLocation, VisualExecutionPolicy, VisualJudgment, VisualPromotionRecord,
    VisualProviderStatus, VisualProviderUnavailableExecutor, VisualQueryClass, VisualRoute,
    VisualRouteMetrics, VisualTextLayoutExecutor, run_visual_benchmark,
};
pub use visual_reranker::{VisualReranker, VisualRerankerParts};
