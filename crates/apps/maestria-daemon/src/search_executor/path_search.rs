use std::{path::Path, sync::Arc};

use anyhow::{Result, anyhow};
use maestria_domain::ActiveSourceVersions;

use super::super::{InteractivePathCandidate, SearchRuntime};

const MAX_INTERACTIVE_PATH_RESULTS: usize = 100;
const MAX_INTERACTIVE_PATH_CHECKS: usize = 100;
const MAX_INTERACTIVE_PATH_BYTES: usize = 4096;
const MAX_INTERACTIVE_PATH_SCANNED_SOURCES: usize = 16_384;
const MAX_INTERACTIVE_PATH_TERMS: usize = 32;
const MAX_INTERACTIVE_PATH_FRESHNESS_BYTES: u64 = 8 * 1024 * 1024;
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
impl SearchRuntime {
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
}
