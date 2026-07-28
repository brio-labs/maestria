use std::{future::Future, pin::Pin};

use maestria_domain::{SearchOutcome, SearchPlan};
use maestria_ports::SearchKnowledgeExecutor;

use super::SearchRuntime;

impl SearchKnowledgeExecutor for SearchRuntime {
    fn search(
        &self,
        plan: SearchPlan,
    ) -> Pin<Box<dyn Future<Output = Result<SearchOutcome, maestria_ports::PortError>> + Send + '_>>
    {
        let runtime = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || runtime.execute_plan_blocking(plan))
                .await
                .map_err(|error| maestria_ports::PortError::InternalContext {
                    context: "search worker",
                    source: error.to_string(),
                })?
                .map_err(|error| maestria_ports::PortError::InternalContext {
                    context: "search plan execution",
                    source: error.to_string(),
                })
        })
    }

    fn plan_and_search(
        &self,
        query: String,
        limit: usize,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<(SearchPlan, SearchOutcome), maestria_ports::PortError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            self.execute(query, limit).await.map_err(|error| {
                maestria_ports::PortError::InternalContext {
                    context: "search query execution",
                    source: error.to_string(),
                }
            })
        })
    }
}
