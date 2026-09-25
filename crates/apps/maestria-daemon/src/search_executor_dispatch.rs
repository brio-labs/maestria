//! Search runtime query dispatch and blocking executor pool.

use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, anyhow};
use maestria_domain::{ActiveSourceVersions, SearchOutcome, SearchPlan};

use super::{InteractivePathCandidate, InteractiveSnapshot, SearchRuntime};

const MAX_INTERACTIVE_PATH_RESULTS: usize = 100;
const MAX_INTERACTIVE_PATH_CHECKS: usize = 100;
const MAX_INTERACTIVE_PATH_BYTES: usize = 4096;
const MAX_INTERACTIVE_PATH_SCANNED_SOURCES: usize = 16_384;
const MAX_INTERACTIVE_PATH_TERMS: usize = 32;
const MAX_INTERACTIVE_PATH_FRESHNESS_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn path_is_within_allowed_roots(allowed_roots: Option<&[PathBuf]>, path: &Path) -> bool {
    allowed_roots.is_none_or(|roots| {
        path.is_absolute()
            && !path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            && roots.iter().any(|root| path.starts_with(root))
    })
}

fn ensure_interactive_not_cancelled(
    cancellation: Option<&maestria_retrieval::SearchCancellation>,
    cancellation_signal: Option<&tokio_util::sync::CancellationToken>,
) -> Result<()> {
    if cancellation.is_some_and(maestria_retrieval::SearchCancellation::is_cancelled)
        || cancellation_signal.is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
    {
        return Err(anyhow!("interactive search was cancelled"));
    }
    Ok(())
}

fn path_matches_query(path: &Path, terms: &[&str]) -> bool {
    let Some(path) = path.to_str() else {
        return false;
    };
    if path.is_empty() || path.len() > MAX_INTERACTIVE_PATH_BYTES {
        return false;
    }
    let path = path.to_lowercase();
    terms.iter().all(|term| path.contains(term))
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn path_freshness_inputs(candidates: &[InteractivePathCandidate]) -> Vec<(String, String)> {
    candidates
        .iter()
        .filter_map(|candidate| {
            candidate
                .path
                .to_str()
                .map(|path| (path.to_owned(), candidate.content_hash.as_str().to_owned()))
        })
        .collect()
}

fn active_source_matches_candidate(
    sources: &ActiveSourceVersions,
    candidate: &InteractivePathCandidate,
) -> bool {
    sources
        .get(&candidate.path)
        .is_some_and(|(artifact_id, artifact_version, content_hash)| {
            *artifact_id == candidate.artifact_id
                && *artifact_version == candidate.artifact_version
                && content_hash == &candidate.content_hash
        })
}

fn interactive_path_candidate_is_authorized(
    runtime: &SearchRuntime,
    candidate: &InteractivePathCandidate,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
) -> bool {
    if !runtime.path_is_allowed_by_roots(&candidate.path) {
        return false;
    }
    let Ok(Some(artifact)) = runtime.artifacts.get(candidate.artifact_id) else {
        return false;
    };
    artifact.id == candidate.artifact_id
        && artifact.index_status == maestria_domain::IndexStatus::Indexed
        && artifact.content_hash.as_ref() == Some(&candidate.content_hash)
        && matches!(
            authorization.evaluate(&artifact.security),
            maestria_governance::RetrievalDecision::Allowed
        )
}

fn ensure_search_not_cancelled(
    cancellation: Option<&maestria_retrieval::SearchCancellation>,
) -> Result<()> {
    if cancellation.is_some_and(maestria_retrieval::SearchCancellation::is_cancelled) {
        return Err(anyhow!("interactive search was cancelled"));
    }
    Ok(())
}
impl SearchRuntime {
    /// Returns a request-owned runtime constrained to the provider-approved
    /// roots frozen into this consumer's grant. `None` preserves legacy
    /// all-approved behavior; `Some(&[])` explicitly denies every source.
    pub fn with_allowed_roots(&self, roots: Option<&[PathBuf]>) -> Self {
        let mut runtime = self.clone();
        runtime.allowed_roots = roots.map(|roots| Arc::from(roots.to_vec()));
        runtime
    }

    pub(crate) fn allowed_roots(&self) -> Option<&[PathBuf]> {
        self.allowed_roots.as_deref()
    }

    fn path_is_allowed_by_roots(&self, path: &Path) -> bool {
        path_is_within_allowed_roots(self.allowed_roots(), path)
    }

    fn approved_source_filter(&self) -> Result<Option<maestria_retrieval::CandidateSourceFilter>> {
        self.approved_source_filter_with_cancellation(None)
    }

    fn approved_source_filter_with_cancellation(
        &self,
        cancellation: Option<&maestria_retrieval::SearchCancellation>,
    ) -> Result<Option<maestria_retrieval::CandidateSourceFilter>> {
        if self.allowed_roots().is_some_and(|roots| roots.is_empty()) {
            return Ok(Some(maestria_retrieval::CandidateSourceFilter::deny_all()));
        }
        let Some(source_manifest) = &self.source_manifest else {
            return Ok(self
                .allowed_roots()
                .map(|_| maestria_retrieval::CandidateSourceFilter::deny_all()));
        };
        ensure_search_not_cancelled(cancellation)?;
        let manifest = source_manifest.read().clone();
        let events = self.domain_events()?;
        ensure_search_not_cancelled(cancellation)?;
        let sources = maestria_domain::active_source_versions(&events);
        let mut freshness_inputs = Vec::new();
        for (path, (_, _, content_hash)) in &sources {
            ensure_search_not_cancelled(cancellation)?;
            if self.path_is_allowed_by_roots(path)
                && manifest.allows_source(path)
                && !maestria_index_selection::is_privacy_excluded_path(path)
            {
                freshness_inputs
                    .push((path.display().to_string(), content_hash.as_str().to_owned()));
            }
        }
        let fresh = self.source_layout.as_ref().map_or_else(
            || {
                freshness_inputs
                    .iter()
                    .map(|(path, _)| path.clone())
                    .collect()
            },
            |layout| crate::watcher::fresh_source_paths(layout, &manifest, &freshness_inputs),
        );
        ensure_search_not_cancelled(cancellation)?;
        let mut allowed = BTreeSet::new();
        for (path, (artifact_id, _, _)) in sources {
            ensure_search_not_cancelled(cancellation)?;
            if self.path_is_allowed_by_roots(&path)
                && manifest.allows_source(&path)
                && !maestria_index_selection::is_privacy_excluded_path(&path)
                && fresh.contains(&path.display().to_string())
            {
                allowed.insert(artifact_id);
            }
        }
        let filter = if allowed.is_empty() {
            maestria_retrieval::CandidateSourceFilter::deny_all()
        } else {
            maestria_retrieval::CandidateSourceFilter::try_new(allowed)
                .map_err(anyhow::Error::new)?
        };
        Ok(Some(filter))
    }
    fn interactive_source_filter(
        &self,
        snapshot: &InteractiveSnapshot,
        cancellation: &maestria_retrieval::SearchCancellation,
    ) -> Result<Option<maestria_retrieval::CandidateSourceFilter>> {
        if self.allowed_roots().is_some_and(|roots| roots.is_empty()) {
            return Ok(Some(maestria_retrieval::CandidateSourceFilter::deny_all()));
        }
        let Some(source_manifest) = &self.source_manifest else {
            return Ok(self
                .allowed_roots()
                .map(|_| maestria_retrieval::CandidateSourceFilter::deny_all()));
        };
        let manifest = source_manifest.read();
        let cached_filter = snapshot.approved.read().as_ref().and_then(
            |(cached_manifest, cached_roots, cached_filter)| {
                (cached_manifest == &*manifest && cached_roots.as_deref() == self.allowed_roots())
                    .then(|| cached_filter.clone())
            },
        );
        if let Some(filter) = cached_filter {
            return Ok(Some(filter));
        }
        let mut allowed = BTreeSet::new();
        for (path, (artifact_id, _, _)) in snapshot.sources.iter() {
            ensure_search_not_cancelled(Some(cancellation))?;
            if self.path_is_allowed_by_roots(path)
                && manifest.allows_source(path)
                && !maestria_index_selection::is_privacy_excluded_path(path)
            {
                allowed.insert(*artifact_id);
            }
        }
        let filter = if allowed.is_empty() {
            maestria_retrieval::CandidateSourceFilter::deny_all()
        } else {
            maestria_retrieval::CandidateSourceFilter::try_new(allowed)
                .map_err(anyhow::Error::new)?
        };
        *snapshot.approved.write() =
            Some((manifest.clone(), self.allowed_roots.clone(), filter.clone()));
        Ok(Some(filter))
    }

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

    fn interactive_path_candidates_blocking(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        cancellation: maestria_retrieval::SearchCancellation,
        cancellation_signal: tokio_util::sync::CancellationToken,
    ) -> Result<Vec<InteractivePathCandidate>> {
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        if self.allowed_roots().is_some_and(|roots| roots.is_empty()) {
            return Ok(Vec::new());
        }
        if !authorization
            .effective_scopes()
            .is_some_and(|scopes| scopes.contains(&self.scope_id))
        {
            return Ok(Vec::new());
        }
        let (Some(source_manifest), Some(_)) = (&self.source_manifest, &self.source_layout) else {
            return Ok(Vec::new());
        };
        let terms = query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if terms.is_empty()
            || terms.len() > MAX_INTERACTIVE_PATH_TERMS
            || query.len() > maestria_retrieval::INTERACTIVE_MAX_QUERY_BYTES
            || query.as_bytes().contains(&0)
            || limit == 0
        {
            return Ok(Vec::new());
        }
        let term_refs = terms.iter().map(String::as_str).collect::<Vec<_>>();
        let manifest = source_manifest.read().clone();
        let snapshot = self.interactive_snapshot()?;
        let revision = snapshot.revision;
        let sources = snapshot.sources;
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        if self.event_log.searchable_source_revision()? != revision {
            return Ok(Vec::new());
        }
        let result_limit = limit.min(MAX_INTERACTIVE_PATH_RESULTS);
        let mut candidates = Vec::with_capacity(result_limit);
        let mut checked_candidates = 0;
        for (path, (artifact_id, artifact_version, content_hash)) in
            sources.iter().take(MAX_INTERACTIVE_PATH_SCANNED_SOURCES)
        {
            ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
            if !path_matches_query(path, &term_refs)
                || !self.path_is_allowed_by_roots(path)
                || is_pdf_path(path)
                || !manifest.allows_source(path)
                || maestria_index_selection::is_privacy_excluded_path(path)
            {
                continue;
            }
            if checked_candidates == MAX_INTERACTIVE_PATH_CHECKS {
                break;
            }
            checked_candidates += 1;
            let candidate = InteractivePathCandidate {
                path: path.clone(),
                artifact_id: *artifact_id,
                artifact_version: *artifact_version,
                content_hash: content_hash.clone(),
            };
            if interactive_path_candidate_is_authorized(self, &candidate, &authorization) {
                candidates.push(candidate);
            }
            if candidates.len() == result_limit {
                break;
            }
        }
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        if self.event_log.searchable_source_revision()? != revision
            || *source_manifest.read() != manifest
        {
            return Ok(Vec::new());
        }
        Ok(candidates)
    }

    fn validate_interactive_path_candidates_blocking(
        &self,
        candidates: Vec<InteractivePathCandidate>,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        cancellation: maestria_retrieval::SearchCancellation,
        cancellation_signal: tokio_util::sync::CancellationToken,
    ) -> Result<Vec<String>> {
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        if candidates.is_empty()
            || !authorization
                .effective_scopes()
                .is_some_and(|scopes| scopes.contains(&self.scope_id))
            || candidates
                .iter()
                .all(|candidate| !self.path_is_allowed_by_roots(&candidate.path))
        {
            return Ok(Vec::new());
        }
        let (Some(source_manifest), Some(layout)) = (&self.source_manifest, &self.source_layout)
        else {
            return Ok(Vec::new());
        };
        let manifest = source_manifest.read().clone();
        let snapshot = self.interactive_snapshot()?;
        let revision = snapshot.revision;
        let sources = snapshot.sources;
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        let mut current_candidates =
            Vec::with_capacity(candidates.len().min(MAX_INTERACTIVE_PATH_RESULTS));
        for candidate in candidates.into_iter().take(MAX_INTERACTIVE_PATH_RESULTS) {
            ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
            if self.path_is_allowed_by_roots(&candidate.path)
                && candidate.path.to_str().is_some()
                && manifest.allows_source(&candidate.path)
                && !maestria_index_selection::is_privacy_excluded_path(&candidate.path)
                && !is_pdf_path(&candidate.path)
                && active_source_matches_candidate(&sources, &candidate)
                && interactive_path_candidate_is_authorized(self, &candidate, &authorization)
            {
                current_candidates.push(candidate);
            }
        }
        if current_candidates.is_empty() {
            return Ok(Vec::new());
        }
        let freshness_inputs = path_freshness_inputs(&current_candidates);
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        let fresh = crate::watcher::fresh_source_paths_bounded(
            layout,
            &manifest,
            &freshness_inputs,
            MAX_INTERACTIVE_PATH_FRESHNESS_BYTES,
        );
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        if self.event_log.searchable_source_revision()? != revision
            || *source_manifest.read() != manifest
        {
            return Ok(Vec::new());
        }
        Ok(current_candidates
            .into_iter()
            .filter_map(|candidate| {
                let path = candidate.path.to_str()?;
                fresh.contains(path).then(|| path.to_owned())
            })
            .take(MAX_INTERACTIVE_PATH_RESULTS)
            .collect())
    }

    /// Searches only approved active source paths. The candidates remain
    /// provider-private until `validate_interactive_path_candidates` reopens
    /// the fresh source scope immediately before response release.
    pub(crate) async fn interactive_path_candidates(
        &self,
        query: String,
        limit: usize,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        cancellation: maestria_retrieval::SearchCancellation,
        cancellation_signal: tokio_util::sync::CancellationToken,
    ) -> Result<Vec<InteractivePathCandidate>> {
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        let permit = tokio::select! {
            biased;
            _ = cancellation_signal.cancelled() => {
                return Err(anyhow!("interactive search was cancelled"));
            }
            permit = self.interactive_search_workers.clone().acquire_owned() => {
                permit.map_err(|error| anyhow!("interactive search worker pool closed: {error}"))?
            }
        };
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        let runtime = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.interactive_path_candidates_blocking(
                query,
                limit,
                authorization,
                cancellation,
                cancellation_signal,
            )
        })
        .await
        .map_err(|error| anyhow!("interactive path search worker failed: {error}"))?
    }

    /// Revalidates path candidates against the live grant scope, active source
    /// version, approved root, privacy exclusions, and on-disk freshness.
    pub(crate) async fn validate_interactive_path_candidates(
        &self,
        candidates: Vec<InteractivePathCandidate>,
        authorization: maestria_governance::RetrievalAuthorizationContext,
        cancellation: maestria_retrieval::SearchCancellation,
        cancellation_signal: tokio_util::sync::CancellationToken,
    ) -> Result<Vec<String>> {
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        let permit = tokio::select! {
            biased;
            _ = cancellation_signal.cancelled() => {
                return Err(anyhow!("interactive search was cancelled"));
            }
            permit = self.interactive_search_workers.clone().acquire_owned() => {
                permit.map_err(|error| anyhow!("interactive search worker pool closed: {error}"))?
            }
        };
        ensure_interactive_not_cancelled(Some(&cancellation), Some(&cancellation_signal))?;
        let runtime = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.validate_interactive_path_candidates_blocking(
                candidates,
                authorization,
                cancellation,
                cancellation_signal,
            )
        })
        .await
        .map_err(|error| anyhow!("interactive path validation worker failed: {error}"))?
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
