pub mod adapters;
pub mod bounded_reranker;
pub mod diversity;
pub mod engine;
pub mod fusion;
pub mod golden;
pub mod repository_benchmark;
pub mod rewrite;
mod sync;
mod sync_engine;
pub mod traits;
pub mod types;

/// Monotonic timestamp used for retrieval latency accounting.
#[derive(Clone, Copy)]
pub struct MonotonicInstant(tokio::time::Instant);

impl MonotonicInstant {
    /// Capture the current monotonic instant.
    pub fn now() -> Self {
        Self(tokio::time::Instant::now())
    }

    /// Return the elapsed duration since this instant.
    pub fn elapsed(self) -> std::time::Duration {
        self.0.elapsed()
    }
}

pub use engine::{RetrievalEngine, SearchPlannerContext};
pub use fusion::FixedKRrf;
pub use repository_benchmark::{
    RepositoryBenchmarkCase, RepositoryBenchmarkComparison, RepositoryBenchmarkCorpus,
    RepositoryBenchmarkError, RepositoryBenchmarkObservation, RepositoryClassComparison,
    RepositoryExecutionPolicy, RepositoryExpectedOutcome, RepositoryPromotionRecord,
    RepositoryQueryClass, RepositoryRoute, RepositoryRouteMetrics,
};
pub use sync::SyncPipeline;
pub use sync_engine::SyncRetrievalEngine;
pub use traits::{
    CandidateReranker, CandidateRetriever, ContextExpander, RankFusion, RerankScorer,
    RetrievalEvaluator,
};
pub use types::{
    HybridExecutionPolicy, HybridPromotionRecord, RerankConstraintScore, RerankLimits,
    RerankRequest, RerankResult, RerankScoreComponents, RerankScorerInput, RetrievalError,
    RetrievalMode, RetrievalResult,
};
