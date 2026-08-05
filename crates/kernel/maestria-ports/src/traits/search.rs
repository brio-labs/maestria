use maestria_domain::{SearchOutcome, SearchPlan};
use std::future::Future;
use std::pin::Pin;

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

    /// Execute a query restricted to an explicit artifact allowlist.
    ///
    /// The port uses typed IDs rather than the retrieval crate's filter type
    /// to keep the dependency graph acyclic; the daemon constructs and
    /// validates the retrieval filter at its boundary.
    fn plan_and_search_selected(
        &self,
        _query: String,
        _limit: usize,
        _artifact_ids: std::collections::BTreeSet<maestria_domain::ArtifactId>,
    ) -> SearchFuture<'_, (SearchPlan, SearchOutcome)> {
        Box::pin(async {
            Err(crate::PortError::InternalContext {
                context: "selected-source search",
                source: "executor does not support source selection".to_string(),
            })
        })
    }
}
