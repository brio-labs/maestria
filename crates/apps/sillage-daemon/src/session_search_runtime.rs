use std::{collections::BTreeSet, time::Duration};

use anyhow::{Context, Result, bail};
use sillage_domain::{SearchOutcome, SearchPlan, SearchStatus, SourceAvailability, SourceChoice};
use sillage_search_tantivy::RamMemoryReservation;
use tokio_util::sync::CancellationToken;

use super::{
    SessionCoverage, SessionSearchResponse, SessionSearchRuntime, deadline::SessionDeadline,
};
use crate::{SearchRuntime, SessionOperationSettlement};

struct CancellableSearchContext<'a> {
    runtime: &'a SearchRuntime,
    authorization: sillage_governance::RetrievalAuthorizationContext,
    search_cancellation: sillage_retrieval::SearchCancellation,
    cancellation: CancellationToken,
    deadline: SessionDeadline,
}

impl SessionSearchRuntime {
    pub fn choice(&self) -> &SourceChoice {
        &self.choice
    }

    pub fn coverage(&self) -> &SessionCoverage {
        &self.coverage
    }

    pub fn availability(&self) -> SourceAvailability {
        self.control.availability()
    }

    pub fn set_availability(&mut self, availability: SourceAvailability) {
        self.choice.set_availability(availability);
        self.control.set_availability(availability);
    }

    pub fn source_revision(&self) -> Result<i64> {
        self.control.ensure_enabled()?;
        self.resources.with_resources(|resources| {
            resources
                .runtime
                .event_log
                .searchable_source_revision()
                .context("session source revision is unavailable")
        })
    }
    /// Wait for accepted session work to settle; this does not cancel accepted work.
    /// If retirement was requested, it also waits for resource cleanup to finish.
    ///
    /// # Cancellation
    ///
    /// Dropping this wait stops only waiting. It does not cancel accepted operations or consume a
    /// pending retirement result.
    pub async fn wait_for_idle(&self) {
        self.control.wait_for_idle().await;
    }

    /// Stop admitting work, cancel accepted operations, join supervisors and clear resources.
    ///
    /// # Cancellation
    ///
    /// Dropping this future stops only this wait; session-owned cleanup continues. Its completion
    /// errors remain available to one later `retire` caller; `wait_for_idle` joins cleanup without
    /// consuming that result. Retirement never purges retained documentary data.
    pub async fn retire(&self) -> Result<()> {
        self.control.retire().await
    }

    /// Search with a fresh cancellation token owned by this call.
    ///
    /// # Cancellation
    /// Dropping this future requests cancellation of the search. Its blocking worker is joined
    /// before the tracked operation settles.
    pub async fn search(&self, query: String, limit: usize) -> Result<SessionSearchResponse> {
        self.search_cancellable(query, limit, CancellationToken::new())
            .await
    }

    /// Search with caller-provided cancellation.
    ///
    /// # Cancellation
    /// Cancelling the token or dropping this future requests cancellation. Accepted blocking work
    /// remains tracked until its worker is joined; dropping the caller does not release admission.
    pub async fn search_cancellable(
        &self,
        query: String,
        limit: usize,
        cancellation: CancellationToken,
    ) -> Result<SessionSearchResponse> {
        self.ensure_enabled()?;
        let session = self.operation_snapshot();
        let input_bytes = query.capacity();
        let worker_token = cancellation.clone();
        self.control
            .track(cancellation, async move {
                let _reservation = session.admit_operation(input_bytes, limit)?;
                session.search_admitted(query, limit, worker_token).await
            })
            .await
    }
    pub(super) fn operation_snapshot(&self) -> Self {
        Self {
            resources: self.resources.clone(),
            choice: self.choice.clone(),
            selected_path: self.selected_path.clone(),
            source_root: self.source_root.clone(),
            manifest: self.manifest.clone(),
            coverage: self.coverage.clone(),
            control: self.control.clone(),
        }
    }

    pub(super) fn admit_operation(
        &self,
        input_bytes: usize,
        limit: usize,
    ) -> Result<RamMemoryReservation> {
        let resources = self.resources.snapshot()?;
        let bytes = resources
            .stores
            .retained_bytes()?
            .saturating_mul(2)
            .saturating_add(u64::try_from(input_bytes)?.saturating_mul(8))
            .saturating_add(u64::try_from(limit)?.saturating_mul(16_384))
            .saturating_add(32_768);
        Ok(resources.stores.memory.reserve(bytes)?)
    }

    async fn search_admitted(
        &self,
        query: String,
        limit: usize,
        cancellation: CancellationToken,
    ) -> Result<SessionSearchResponse> {
        self.ensure_enabled()?;
        let resources = self.resources.snapshot()?;
        let budget = self.choice.budget();
        let query_bytes = u64::try_from(query.len())
            .map_err(anyhow::Error::new)
            .context("session search query length exceeds work accounting")?;
        if limit == 0
            || limit > budget.max_results() as usize
            || query_bytes > budget.max_work_units()
        {
            bail!("session search request exceeds the selected source budget");
        }
        let source_revision = resources
            .runtime
            .event_log
            .searchable_source_revision()
            .context("session source revision is unavailable")?;
        let deadline_at =
            SessionDeadline::after(Duration::from_millis(u64::from(budget.max_duration_ms())));
        let search_cancellation = sillage_retrieval::SearchCancellation::default();
        let mut cancellation_guard =
            SearchCancellationGuard::new(cancellation.clone(), search_cancellation.clone());
        let authorization = self.authorization(&resources.runtime)?;
        let (plan, mut outcome) = self
            .execute_cancellable_search(
                query,
                limit,
                CancellableSearchContext {
                    runtime: &resources.runtime,
                    authorization: authorization.clone(),
                    search_cancellation: search_cancellation.clone(),
                    cancellation: cancellation.clone(),
                    deadline: deadline_at,
                },
            )
            .await?;
        let candidates = outcome
            .evidence
            .iter()
            .map(|candidate| (candidate.evidence_id(), candidate.artifact_version()))
            .collect::<Vec<_>>();
        let (previews, stale_or_denied_candidates) = self
            .open_candidate_batch(
                resources.clone(),
                candidates,
                source_revision,
                authorization,
                cancellation.clone(),
                deadline_at,
            )
            .await?;
        let current_revision = resources
            .runtime
            .event_log
            .searchable_source_revision()
            .context("session source revision is unavailable")?;
        if current_revision != source_revision {
            bail!("session source revision changed during retrieval");
        }
        let valid_ids = previews
            .iter()
            .map(|preview| preview.evidence_id)
            .collect::<BTreeSet<_>>();
        let original_count = outcome.evidence.len();
        outcome
            .evidence
            .retain(|candidate| valid_ids.contains(&candidate.evidence_id()));
        if outcome.evidence.len() < original_count {
            outcome.status = if outcome.evidence.is_empty() {
                SearchStatus::StaleEvidenceOnly
            } else {
                SearchStatus::EvidenceIncomplete
            };
            outcome.trace_data = None;
        }
        let response = SessionSearchResponse {
            plan,
            outcome,
            previews,
            coverage: self.coverage.clone(),
            source_revision,
            stale_or_denied_candidates,
        };
        cancellation_guard.complete();
        Ok(response)
    }

    async fn execute_cancellable_search(
        &self,
        query: String,
        limit: usize,
        context: CancellableSearchContext<'_>,
    ) -> Result<(SearchPlan, SearchOutcome)> {
        let search = context.runtime.execute_interactive(
            query,
            limit,
            context.authorization,
            context.search_cancellation.clone(),
            context.cancellation.clone(),
        );
        tokio::pin!(search);
        tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => {
                context.cancellation.cancel();
                context.search_cancellation.cancel();
                settle_search(search.await, SessionOperationSettlement::RoutineCancellation)
            }
            _ = tokio::time::sleep(context.deadline.remaining()) => {
                context.search_cancellation.cancel();
                settle_search(search.await, SessionOperationSettlement::DeadlineExceeded)
            }
            result = &mut search => result,
        }
    }

    pub(super) fn authorization(
        &self,
        runtime: &SearchRuntime,
    ) -> Result<sillage_governance::RetrievalAuthorizationContext> {
        runtime
            .retrieval_policy
            .authorization_context(&sillage_domain::CorpusScope::Restricted(vec![
                runtime.scope_id,
            ]))
            .map_err(anyhow::Error::new)
            .context("session retrieval authorization denied")
    }

    pub(super) fn ensure_enabled(&self) -> Result<()> {
        self.control.ensure_enabled()
    }
}

fn settle_search<T>(result: Result<T>, interruption: SessionOperationSettlement) -> Result<T> {
    match result {
        Err(error) => {
            if error
                .downcast_ref::<super::SessionLifecycleFailure>()
                .is_some()
            {
                return Err(error);
            }
            match SessionOperationSettlement::classify_error(&error) {
                Some(SessionOperationSettlement::RoutineCancellation)
                    if interruption == SessionOperationSettlement::DeadlineExceeded =>
                {
                    Err(error.context(interruption))
                }
                Some(_) => Err(error),
                None if is_engine_cancellation(&error) => Err(error.context(interruption)),
                None => Err(error),
            }
        }
        Ok(_) => Err(anyhow::Error::new(interruption)),
    }
}

fn is_engine_cancellation(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<sillage_retrieval::RetrievalError>(),
            Some(sillage_retrieval::RetrievalError::Cancelled)
        )
    })
}
struct SearchCancellationGuard {
    request: CancellationToken,
    search: sillage_retrieval::SearchCancellation,
    completed: bool,
}

impl SearchCancellationGuard {
    fn new(request: CancellationToken, search: sillage_retrieval::SearchCancellation) -> Self {
        Self {
            request,
            search,
            completed: false,
        }
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for SearchCancellationGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.request.cancel();
            self.search.cancel();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::super::SessionLifecycleFailure;
    use super::{SessionOperationSettlement, settle_search};
    use anyhow::Result;

    #[test]
    fn lifecycle_failure_is_not_reclassified_as_deadline_settlement() -> Result<()> {
        let original = SessionOperationSettlement::RoutineCancellation;
        let compound =
            anyhow::Error::new(original).context(SessionLifecycleFailure::OperationState);
        let error =
            settle_search::<()>(Err(compound), SessionOperationSettlement::DeadlineExceeded)
                .err()
                .ok_or_else(|| anyhow::anyhow!("the lifecycle failure must remain an error"))?;
        if error.downcast_ref::<SessionOperationSettlement>() != Some(&original) {
            anyhow::bail!("deadline settlement discarded the original cancellation");
        }
        if format!("{error:#}").contains("session operation exceeded its selected deadline") {
            anyhow::bail!("deadline superseded a lifecycle failure");
        }
        Ok(())
    }
    #[test]
    fn engine_cancellation_preserves_cause_and_deadline_classification() -> Result<()> {
        use sillage_retrieval::RetrievalError;

        for interruption in [
            SessionOperationSettlement::RoutineCancellation,
            SessionOperationSettlement::DeadlineExceeded,
        ] {
            let error = settle_search::<()>(
                Err(anyhow::Error::new(RetrievalError::Cancelled)),
                interruption,
            )
            .err()
            .ok_or_else(|| anyhow::anyhow!("engine cancellation must remain an error"))?;
            if SessionOperationSettlement::classify_error(&error) != Some(interruption) {
                anyhow::bail!("engine cancellation lost its typed settlement classification");
            }
            if !error.chain().any(|cause| {
                matches!(
                    cause.downcast_ref::<RetrievalError>(),
                    Some(RetrievalError::Cancelled)
                )
            }) {
                anyhow::bail!("engine cancellation lost its original retrieval cause");
            }
        }
        Ok(())
    }
}
