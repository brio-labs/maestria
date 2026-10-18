use std::{collections::BTreeSet, future::Future, pin::Pin, sync::Arc};

use sillage_domain::{ArtifactId, SearchOutcome, SearchPlan};
use sillage_ports::SearchKnowledgeExecutor;

use super::SearchRuntime;

impl SearchKnowledgeExecutor for SearchRuntime {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self as &dyn std::any::Any)
    }

    fn search(
        &self,
        plan: SearchPlan,
    ) -> Pin<Box<dyn Future<Output = Result<SearchOutcome, sillage_ports::PortError>> + Send + '_>>
    {
        let runtime = Arc::new(self.clone());
        Box::pin(async move {
            tokio::task::spawn_blocking(move || runtime.execute_plan_blocking(plan))
                .await
                .map_err(|error| sillage_ports::PortError::InternalContext {
                    context: "search worker",
                    source: error.to_string(),
                })?
                .map_err(|error| sillage_ports::PortError::InternalContext {
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
            dyn Future<Output = Result<(SearchPlan, SearchOutcome), sillage_ports::PortError>>
                + Send
                + '_,
        >,
    > {
        let runtime = Arc::new(self.clone());
        Box::pin(async move {
            runtime.execute_arc(query, limit).await.map_err(|error| {
                sillage_ports::PortError::InternalContext {
                    context: "search query execution",
                    source: error.to_string(),
                }
            })
        })
    }
    fn plan_and_search_selected(
        &self,
        query: String,
        limit: usize,
        artifact_ids: BTreeSet<ArtifactId>,
    ) -> sillage_ports::SearchFuture<'_, (SearchPlan, SearchOutcome)> {
        let runtime = Arc::new(self.clone());
        Box::pin(async move {
            let source_filter = sillage_retrieval::CandidateSourceFilter::try_new(artifact_ids)
                .map_err(|error| sillage_ports::PortError::InvalidInputContext {
                    context: "selected source filter",
                    source: error.to_string(),
                })?;
            let authorization = runtime
                .retrieval_policy
                .authorization_context(&sillage_domain::CorpusScope::Restricted(vec![
                    runtime.scope_id,
                ]))
                .map_err(|error| sillage_ports::PortError::InternalContext {
                    context: "selected search authorization",
                    source: format!("{error:?}"),
                })?;
            runtime
                .execute_selected_sources_arc(query, limit, authorization, source_filter)
                .await
                .map_err(|error| sillage_ports::PortError::InternalContext {
                    context: "selected source search",
                    source: error.to_string(),
                })
        })
    }
}
