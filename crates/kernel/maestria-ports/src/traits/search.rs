use std::future::Future;
use std::pin::Pin;

use maestria_domain::{SearchOutcome, SearchPlan};

pub type SearchFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, crate::PortError>> + Send + 'a>>;

/// Executes a typed knowledge search and returns one provenance-bearing outcome.
pub trait SearchKnowledgeExecutor: Send + Sync {
    fn search(&self, plan: SearchPlan) -> SearchFuture<'_, SearchOutcome>;

    fn plan_and_search(
        &self,
        query: String,
        limit: usize,
    ) -> SearchFuture<'_, (SearchPlan, SearchOutcome)>;
}
