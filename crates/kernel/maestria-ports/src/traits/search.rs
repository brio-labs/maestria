use std::future::Future;
use std::pin::Pin;

use maestria_domain::{SearchOutcome, SearchPlan};

pub type SearchFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, crate::PortError>> + Send + 'a>>;

/// Executes a typed knowledge search and returns one provenance-bearing outcome.
///
/// # Cancellation
/// Dropping the returned future abandons the in-flight search request. The
/// daemon implementation executes search on a blocking worker, so an in-flight
/// search may continue to completion in the background after the future is
/// dropped; its results are simply never delivered. Callers that require
/// bounded staleness must rely on the plan's budget and stop conditions rather
/// than on future cancellation.
pub trait SearchKnowledgeExecutor: Send + Sync {
    fn search(&self, plan: SearchPlan) -> SearchFuture<'_, SearchOutcome>;

    fn plan_and_search(
        &self,
        query: String,
        limit: usize,
    ) -> SearchFuture<'_, (SearchPlan, SearchOutcome)>;
}
