use std::future::Future;
use std::pin::Pin;

use maestria_domain::{SearchOutcome, SearchPlan};

/// Executes a typed knowledge search and returns one provenance-bearing outcome.
pub trait SearchKnowledgeExecutor: Send + Sync {
    fn search(
        &self,
        plan: SearchPlan,
    ) -> Pin<Box<dyn Future<Output = Result<SearchOutcome, crate::PortError>> + Send + '_>>;
}
