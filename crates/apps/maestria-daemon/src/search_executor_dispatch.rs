//! Search runtime query dispatch and blocking executor pool.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_domain::{SearchOutcome, SearchPlan};

#[path = "search_executor/path_search.rs"]
mod path_search;
#[path = "search_executor/source_filter.rs"]
mod source_filter;
use self::source_filter::ensure_search_not_cancelled;
pub(crate) use self::source_filter::path_is_within_allowed_roots;

use super::{InteractiveSnapshot, SearchRuntime};

impl SearchRuntime {
    fn search_with_authorization(
        &self,
        engine: &maestria_retrieval::RetrievalEngine,
        plan: &SearchPlan,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        explicit_filter: Option<&maestria_retrieval::CandidateSourceFilter>,
    ) -> Result<SearchOutcome> {
        let approved_filter = self.approved_source_filter()?;
        let effective_filter = match (approved_filter, explicit_filter) {
            (Some(approved), Some(explicit)) => Some(approved.intersect(explicit)),
            (Some(approved), None) => Some(approved),
            (None, Some(explicit)) => Some(explicit.clone()),
            (None, None) => None,
        };
        match effective_filter {
            Some(filter) => engine
                .search_pre_authorized_selected(plan, authorization, filter)
                .map_err(anyhow::Error::new),
            None => engine
                .search_pre_authorized(plan, authorization)
                .map_err(anyhow::Error::new),
        }
    }
    fn search_interactive_with_authorization(
        &self,
        engine: &maestria_retrieval::RetrievalEngine,
        plan: &SearchPlan,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        snapshot: &InteractiveSnapshot,
        cancellation: maestria_retrieval::SearchCancellation,
    ) -> Result<SearchOutcome> {
        ensure_search_not_cancelled(Some(&cancellation))?;
        let approved_filter = self.interactive_source_filter(snapshot, &cancellation)?;
        ensure_search_not_cancelled(Some(&cancellation))?;
        let outcome = match approved_filter {
            Some(filter) => engine.search_interactive_pre_authorized_selected(
                plan,
                authorization,
                filter,
                cancellation,
            ),
            None => engine.search_interactive_pre_authorized(plan, authorization, cancellation),
        };
        outcome.map_err(anyhow::Error::new)
    }

    pub(super) fn execute_plan_blocking(&self, plan: SearchPlan) -> Result<SearchOutcome> {
        let plan = plan
            .confine_to_scope(self.scope_id)
            .map_err(anyhow::Error::new)?;
        let authorization = self
            .retrieval_policy
            .authorization_context(plan.scope())
            .map_err(|error| anyhow!("retrieval authorization denied: {error}"))?;
        let engine = self.cached_retrieval_engine()?;
        self.search_with_authorization(&engine, &plan, authorization, None)
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
            let authorization = self
                .retrieval_policy
                .authorization_context(plan.scope())
                .map_err(|error| anyhow!("retrieval authorization denied: {error}"))?;
            self.search_with_authorization(engine, plan, authorization, None)
        })
    }

    fn execute_pre_authorized_blocking(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        self.execute_query_blocking(query, limit, |engine, plan| {
            self.search_with_authorization(engine, plan, authorization, None)
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
            self.search_with_authorization(engine, plan, authorization, Some(&source_filter))
        })
    }
    fn execute_interactive_blocking(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        cancellation: maestria_retrieval::SearchCancellation,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        ensure_search_not_cancelled(Some(&cancellation))?;
        let snapshot = self.interactive_snapshot()?;
        ensure_search_not_cancelled(Some(&cancellation))?;
        let plan = snapshot
            .engine
            .plan_interactive(query, limit, &self.planner_context())
            .map_err(anyhow::Error::new)?
            .confine_to_scope(self.scope_id)
            .map_err(anyhow::Error::new)?;
        ensure_search_not_cancelled(Some(&cancellation))?;
        let outcome = self.search_interactive_with_authorization(
            &snapshot.engine,
            &plan,
            authorization,
            &snapshot,
            cancellation,
        )?;
        Ok((plan, outcome))
    }

    /// Runs a bounded interactive lexical query on a daemon-owned blocking
    /// worker. The permit is moved into that worker so cancelling its caller
    /// cannot release capacity while synchronous index work is still running.
    ///
    /// # Cancellation
    /// Supersession cancels the worker's token; dropping the caller does not
    /// release its permit before the blocking search has completed.
    pub async fn execute_interactive(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        cancellation: maestria_retrieval::SearchCancellation,
        cancellation_signal: tokio_util::sync::CancellationToken,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        ensure_search_not_cancelled(Some(&cancellation))?;
        let permit = tokio::select! {
            biased;
            _ = cancellation_signal.cancelled() => {
                return Err(anyhow!("interactive search was cancelled"));
            }
            permit = self.interactive_search_workers.clone().acquire_owned() => {
                permit.map_err(|error| anyhow!("interactive search worker pool closed: {error}"))?
            }
        };
        ensure_search_not_cancelled(Some(&cancellation))?;
        let runtime = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.execute_interactive_blocking(query, limit, authorization, cancellation)
        })
        .await
        .map_err(|error| anyhow!("interactive search worker failed: {error}"))?
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::path_is_within_allowed_roots;

    #[test]
    fn consumer_grant_path_check_rejects_sibling_prefix_and_parent_traversal() {
        let roots = [PathBuf::from("/approved/allowed")];
        assert!(path_is_within_allowed_roots(
            Some(&roots),
            Path::new("/approved/allowed/file.md"),
        ));
        assert!(!path_is_within_allowed_roots(
            Some(&roots),
            Path::new("/approved/allowed-other/file.md"),
        ));
        assert!(!path_is_within_allowed_roots(
            Some(&roots),
            Path::new("/approved/allowed/../allowed-other/file.md"),
        ));
        assert!(!path_is_within_allowed_roots(
            Some(&roots),
            Path::new("allowed/file.md"),
        ));
    }
}
