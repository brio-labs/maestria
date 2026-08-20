//! Search runtime query dispatch and blocking executor pool.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_domain::{SearchOutcome, SearchPlan};

use super::SearchRuntime;

impl SearchRuntime {
    pub(super) fn execute_plan_blocking(&self, plan: SearchPlan) -> Result<SearchOutcome> {
        let plan = plan
            .confine_to_scope(self.scope_id)
            .map_err(anyhow::Error::new)?;
        let engine = self.cached_retrieval_engine()?;
        tokio::runtime::Handle::current()
            .block_on(engine.search(&plan))
            .map_err(anyhow::Error::new)
    }

    fn execute_query_blocking<F>(
        &self,
        query: String,
        limit: usize,
        run: F,
    ) -> Result<(SearchPlan, SearchOutcome)>
    where
        F: FnOnce(&maestria_retrieval::RetrievalEngine, &SearchPlan) -> Result<SearchOutcome>,
    {
        let engine = self.cached_retrieval_engine()?;
        let plan = engine
            .plan(query, limit, &self.planner_context())
            .map_err(anyhow::Error::new)?;
        let outcome = run(&engine, &plan)?;
        Ok((plan, outcome))
    }

    fn execute_search_blocking(
        &self,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        self.execute_query_blocking(query, limit, |engine, plan| {
            tokio::runtime::Handle::current()
                .block_on(engine.search(plan))
                .map_err(anyhow::Error::new)
        })
    }

    fn execute_pre_authorized_blocking(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        self.execute_query_blocking(query, limit, |engine, plan| {
            tokio::runtime::Handle::current()
                .block_on(engine.search_pre_authorized(plan, authorization))
                .map_err(anyhow::Error::new)
        })
    }

    fn execute_selected_blocking(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        source_filter: maestria_retrieval::CandidateSourceFilter,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        self.execute_query_blocking(query, limit, |engine, plan| {
            tokio::runtime::Handle::current()
                .block_on(engine.search_pre_authorized_selected(plan, authorization, source_filter))
                .map_err(anyhow::Error::new)
        })
    }

    /// Build and execute the same plan used by daemon search effects.
    ///
    /// # Cancellation
    /// Cancelling the returned future does not abort the blocking search worker; the spawned
    /// blocking task continues until completion.
    pub async fn execute(
        &self,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || runtime.execute_search_blocking(query, limit))
            .await
            .map_err(|error| anyhow!("search worker failed: {error}"))?
    }

    /// Arc-optimized path: avoids an extra struct clone when the caller already holds an `Arc`.
    ///
    /// # Cancellation
    /// Cancelling the returned future does not abort the blocking search worker; the spawned
    /// blocking task continues until completion.
    pub async fn execute_arc(
        self: Arc<Self>,
        query: String,
        limit: usize,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || runtime.execute_search_blocking(query, limit))
            .await
            .map_err(|error| anyhow!("search worker failed: {error}"))?
    }

    /// Executes a provider-composed authorization context without rebuilding
    /// policy in any retrieval lane.
    ///
    /// # Cancellation
    /// Cancelling the returned future does not abort the blocking search worker; the spawned
    /// blocking task continues until completion.
    pub async fn execute_pre_authorized(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || {
            runtime.execute_pre_authorized_blocking(query, limit, authorization)
        })
        .await
        .map_err(|error| anyhow!("search worker failed: {error}"))?
    }

    /// # Cancellation
    /// Cancelling the returned future does not abort the blocking search worker; the spawned
    /// blocking task continues until completion.
    pub async fn execute_pre_authorized_arc(
        self: Arc<Self>,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || {
            runtime.execute_pre_authorized_blocking(query, limit, authorization)
        })
        .await
        .map_err(|error| anyhow!("search worker failed: {error}"))?
    }

    /// Executes a search restricted to the explicitly selected artifact set.
    ///
    /// # Cancellation
    ///
    /// Dropping the future stops awaiting the bounded worker; the worker
    /// itself does not mutate shared state after cancellation.
    pub async fn execute_selected_sources(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        source_filter: maestria_retrieval::CandidateSourceFilter,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || {
            runtime.execute_selected_blocking(query, limit, authorization, source_filter)
        })
        .await
        .map_err(|error| anyhow!("search worker failed: {error}"))?
    }

    /// # Cancellation
    ///
    /// Dropping the future stops awaiting the bounded worker; the worker
    /// itself does not mutate shared state after cancellation.
    pub async fn execute_selected_sources_arc(
        self: Arc<Self>,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        source_filter: maestria_retrieval::CandidateSourceFilter,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || {
            runtime.execute_selected_blocking(query, limit, authorization, source_filter)
        })
        .await
        .map_err(|error| anyhow!("search worker failed: {error}"))?
    }
}
