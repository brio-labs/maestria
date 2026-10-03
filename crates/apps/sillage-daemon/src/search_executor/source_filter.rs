use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use super::super::{InteractiveSnapshot, SearchRuntime};
use anyhow::{Result, anyhow};

pub(super) fn ensure_search_not_cancelled(
    cancellation: Option<&sillage_retrieval::SearchCancellation>,
) -> Result<()> {
    if cancellation.is_some_and(sillage_retrieval::SearchCancellation::is_cancelled) {
        return Err(anyhow!("interactive search was cancelled"));
    }
    Ok(())
}

pub(crate) fn path_is_within_allowed_roots(allowed_roots: Option<&[PathBuf]>, path: &Path) -> bool {
    allowed_roots.is_none_or(|roots| {
        path.is_absolute()
            && !path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            && roots.iter().any(|root| path.starts_with(root))
    })
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

    pub(super) fn path_is_allowed_by_roots(&self, path: &Path) -> bool {
        path_is_within_allowed_roots(self.allowed_roots(), path)
    }

    pub(super) fn approved_source_filter(
        &self,
    ) -> Result<Option<sillage_retrieval::CandidateSourceFilter>> {
        self.approved_source_filter_with_cancellation(None)
    }

    fn approved_source_filter_with_cancellation(
        &self,
        cancellation: Option<&sillage_retrieval::SearchCancellation>,
    ) -> Result<Option<sillage_retrieval::CandidateSourceFilter>> {
        if self.allowed_roots().is_some_and(|roots| roots.is_empty()) {
            return Ok(Some(sillage_retrieval::CandidateSourceFilter::deny_all()));
        }
        let Some(source_manifest) = &self.source_manifest else {
            return Ok(self
                .allowed_roots()
                .map(|_| sillage_retrieval::CandidateSourceFilter::deny_all()));
        };
        ensure_search_not_cancelled(cancellation)?;
        let manifest = source_manifest.read().clone();
        let events = self.domain_events()?;
        ensure_search_not_cancelled(cancellation)?;
        let sources = sillage_domain::active_source_versions(&events);
        let mut freshness_inputs = Vec::new();
        for (path, (_, _, content_hash)) in &sources {
            ensure_search_not_cancelled(cancellation)?;
            if self.path_is_allowed_by_roots(path)
                && manifest.allows_source(path)
                && !sillage_index_selection::is_privacy_excluded_path(path)
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
                && !sillage_index_selection::is_privacy_excluded_path(&path)
                && fresh.contains(&path.display().to_string())
            {
                allowed.insert(artifact_id);
            }
        }
        let filter = if allowed.is_empty() {
            sillage_retrieval::CandidateSourceFilter::deny_all()
        } else {
            sillage_retrieval::CandidateSourceFilter::try_new(allowed)
                .map_err(anyhow::Error::new)?
        };
        Ok(Some(filter))
    }

    pub(super) fn interactive_source_filter(
        &self,
        snapshot: &InteractiveSnapshot,
        cancellation: &sillage_retrieval::SearchCancellation,
    ) -> Result<Option<sillage_retrieval::CandidateSourceFilter>> {
        if self.allowed_roots().is_some_and(|roots| roots.is_empty()) {
            return Ok(Some(sillage_retrieval::CandidateSourceFilter::deny_all()));
        }
        let Some(source_manifest) = &self.source_manifest else {
            return Ok(self
                .allowed_roots()
                .map(|_| sillage_retrieval::CandidateSourceFilter::deny_all()));
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
                && !sillage_index_selection::is_privacy_excluded_path(path)
            {
                allowed.insert(*artifact_id);
            }
        }
        let filter = if allowed.is_empty() {
            sillage_retrieval::CandidateSourceFilter::deny_all()
        } else {
            sillage_retrieval::CandidateSourceFilter::try_new(allowed)
                .map_err(anyhow::Error::new)?
        };
        *snapshot.approved.write() =
            Some((manifest.clone(), self.allowed_roots.clone(), filter.clone()));
        Ok(Some(filter))
    }
}
