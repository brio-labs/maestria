//! Repository code index operations: candidates, selection profile, run,
//! and status. The selection vocabulary (Recommended/Maybe/Noise classes,
//! per-directory policies) is shared with the document index choice layer;
//! the scope rules (`validate_scope_root`/`validate_within_root`) are the
//! exact ones from `index_services.rs` and are reused, not copied.
//!
//! All handlers run CPU/IO work in `spawn_blocking` so the daemon stays
//! responsive; the run handler is long-running by design (like the CLI
//! `index repository` command) and runs per-connection.

use super::index_services::{validate_scope_root, validate_within_root};
use crate::api::server::ApiContext;
use crate::api::{
    RepositoryIndexCandidatesResponse, RepositoryIndexProgressResponse, RepositoryIndexRunResponse,
    RepositoryIndexSelectionResponse, RepositoryIndexStatusResponse, RepositoryIndexSummary,
};
use anyhow::{Result, anyhow};
use maestria_code_intel::{
    REPOSITORY_CODE_CANDIDATES_FILENAME, REPOSITORY_CODE_INDEX_FILENAME,
    REPOSITORY_CODE_PARSER_GENERATION, RepositoryCodeIndex, RepositoryIndexBuildMode,
    RepositorySelection, build_or_update_repository_index,
};
use maestria_index_selection::{IndexPolicy, IndexSelectionProfile};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Resolve a client-supplied include or policy key against the validated
/// root: repository-relative paths are joined to the root, absolute paths
/// used as-is, and the containment rule (`validate_within_root`) applies to
/// the resolved path. A path equal to the root selects the whole
/// repository and is dropped from the selection.
fn resolve_within_root(path: &Path, root: &Path) -> Result<PathBuf> {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    validate_within_root(&resolved, root)?;
    Ok(resolved)
}

/// The selection built from client-supplied includes: every include is
/// resolved, validated, and stripped to repository-relative (the root
/// itself → whole repository).
fn selection_from_includes(root: &Path, includes: &[String]) -> Result<RepositorySelection> {
    let mut relative = Vec::new();
    for include in includes {
        let resolved = resolve_within_root(Path::new(include), root)?;
        if resolved == root {
            continue;
        }
        relative.push(
            resolved
                .strip_prefix(root)
                .map_err(|_| anyhow!("resolved include escapes its root"))?
                .to_string_lossy()
                .into_owned(),
        );
    }
    RepositorySelection::try_new(relative).map_err(|error| anyhow!(error))
}

/// The per-directory policies with keys normalized to repository-relative
/// paths (the root itself is dropped).
fn relative_policies(
    root: &Path,
    policies: BTreeMap<PathBuf, IndexPolicy>,
) -> Result<BTreeMap<String, IndexPolicy>> {
    let mut relative = BTreeMap::new();
    for (key, policy) in policies {
        let resolved = resolve_within_root(&key, root)?;
        if resolved == root {
            continue;
        }
        relative.insert(
            resolved
                .strip_prefix(root)
                .map_err(|_| anyhow!("resolved policy key escapes its root"))?
                .to_string_lossy()
                .into_owned(),
            policy,
        );
    }
    Ok(relative)
}

/// Unique source file paths in the indexed symbols (what registration
/// attempts to bind).
fn expected_source_files(index: &RepositoryCodeIndex) -> BTreeSet<String> {
    index
        .symbols
        .iter()
        .map(|symbol| symbol.provenance.file_path.clone())
        .collect()
}

/// Scan a repository root and return its classified candidate tree.
///
/// # Cancellation
/// Cancelling drops the in-flight scan task; no state is changed.
pub(super) async fn candidates(
    context: &ApiContext,
    root: String,
) -> Result<RepositoryIndexCandidatesResponse> {
    validate_scope_root(context, &root)?;
    let root_path = match PathBuf::from(&root).canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => PathBuf::from(&root),
    };
    let mut tree = tokio::task::spawn_blocking(move || {
        maestria_index_selection::scan_repository_candidates(&root_path)
    })
    .await
    .map_err(|error| anyhow!("repository candidate scan task failed: {error}"))??;
    // Bound the wire tree: a scan of a large repository must fit the
    // protocol response cap.
    maestria_index_selection::bound_candidate_tree(&mut tree);
    Ok(RepositoryIndexCandidatesResponse { root, tree })
}

/// Load the persisted repository selection profile, when one exists.
///
/// # Cancellation
/// Cancelling while the blocking load is in flight leaves no state changed.
pub(super) async fn selection_get(
    context: &ApiContext,
) -> Result<RepositoryIndexSelectionResponse> {
    let profile_path = context
        .layout
        .system_dir
        .join("repository-index-selection.json");
    let profile =
        tokio::task::spawn_blocking(move || maestria_index_selection::load_profile(&profile_path))
            .await
            .map_err(|error| anyhow!("repository selection load task failed: {error}"))??;
    Ok(RepositoryIndexSelectionResponse { profile })
}

/// Persist an approved repository selection profile. Includes and policy
/// keys are resolved against the profile root (repository-relative entries
/// allowed), validated for containment, and stored normalized to
/// repository-relative paths; the root itself is dropped (whole-repo).
///
/// # Cancellation
/// Cancelling while the blocking write is in flight may leave the file
/// partially written; the caller should re-save to repair it.
pub(super) async fn selection_save(
    context: &ApiContext,
    profile: IndexSelectionProfile,
) -> Result<()> {
    validate_scope_root(context, &profile.root.display().to_string())?;
    let root_path = &profile.root;
    let mut includes = Vec::new();
    for include in profile.includes {
        let resolved = resolve_within_root(&include, root_path)?;
        if resolved == *root_path {
            continue;
        }
        includes.push(
            resolved
                .strip_prefix(root_path)
                .map_err(|_| anyhow!("resolved include escapes its root"))?
                .to_string_lossy()
                .into_owned(),
        );
    }
    let policies = relative_policies(root_path, profile.policies)?;
    let profile = IndexSelectionProfile {
        root: profile.root,
        includes: includes.into_iter().map(PathBuf::from).collect(),
        policies: policies
            .into_iter()
            .map(|(key, policy)| (PathBuf::from(key), policy))
            .collect(),
    };
    let profile_path = context
        .layout
        .system_dir
        .join("repository-index-selection.json");
    tokio::task::spawn_blocking(move || {
        maestria_index_selection::save_profile(&profile_path, &profile)
    })
    .await
    .map_err(|error| anyhow!("repository selection save task failed: {error}"))?
}

/// Build or incrementally update the repository code index for `root`
/// under the given selection and per-directory policies, then register
/// every indexed source as a canonical artifact through the live runtime.
/// On a registration mismatch (a file changed between extraction and
/// registration) the index is rebuilt once and re-registered, mirroring
/// the CLI repair loop.
///
/// # Cancellation
/// Cancelling stops the build/registration between phases; the persisted
/// index is always consistent with the worktree at save time, and a
/// concurrent run is last-write-wins with consistent snapshots.
pub(super) async fn run(
    context: &ApiContext,
    root: String,
    includes: Vec<String>,
    policies: BTreeMap<String, IndexPolicy>,
) -> Result<RepositoryIndexRunResponse> {
    validate_scope_root(context, &root)?;
    let root_path = match PathBuf::from(&root).canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => PathBuf::from(&root),
    };
    let selection = selection_from_includes(&root_path, &includes)?;
    let policies = relative_policies(
        &root_path,
        policies
            .into_iter()
            .map(|(key, policy)| (PathBuf::from(key), policy))
            .collect(),
    )?;
    let Some(runtime) = context.runtime.clone() else {
        return Err(anyhow!("repository index run requires the live runtime"));
    };
    let (_, manifest) = super::support::load_state_and_manifest(&context.layout)?;
    let excluded_patterns = manifest.excluded_patterns.clone();
    let layout = context.layout.clone();
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    let candidates_path = layout.system_dir.join(REPOSITORY_CODE_CANDIDATES_FILENAME);

    let build = {
        let index_path = index_path.clone();
        let candidates_path = candidates_path.clone();
        let root_path = root_path.clone();
        let excluded_patterns = excluded_patterns.clone();
        let selection = selection.clone();
        let policies = policies.clone();
        move || {
            build_or_update_repository_index(
                &index_path,
                &candidates_path,
                &root_path,
                REPOSITORY_CODE_PARSER_GENERATION,
                &excluded_patterns,
                &selection,
                &policies,
            )
            .map_err(|error| anyhow!("build repository code index: {error}"))
        }
    };
    // The run publishes its live progress for the status handler; the guard
    // clears it when the run finishes, fails, or is cancelled.
    struct ProgressGuard;
    impl Drop for ProgressGuard {
        fn drop(&mut self) {
            crate::set_repository_index_progress(None);
        }
    }
    let _progress_guard = ProgressGuard;
    crate::set_repository_index_progress(Some(crate::api::RepositoryIndexProgress {
        phase: "building".to_string(),
        total: 0,
        registered: 0,
    }));
    let mut built = tokio::task::spawn_blocking(build.clone())
        .await
        .map_err(|error| anyhow!("repository index build task failed: {error}"))??;
    let mut mode = built.1;
    if !matches!(mode, RepositoryIndexBuildMode::Noop) {
        built
            .0
            .save(&index_path)
            .map_err(|error| anyhow!("save repository code index: {error}"))?;
    }
    crate::set_repository_index_progress(Some(crate::api::RepositoryIndexProgress {
        phase: "registering".to_string(),
        total: expected_source_files(&built.0).len(),
        registered: 0,
    }));
    let (mismatched, skipped) =
        crate::register_repository_sources_with_runtime(&runtime, &built.0, &root_path).await?;
    let mut registered = expected_source_files(&built.0).len() - mismatched.len() - skipped;
    if !mismatched.is_empty() {
        // The repository changed between extraction and registration:
        // rebuild once and re-register; the incremental path re-extracts
        // the mismatched files.
        let rebuilt = tokio::task::spawn_blocking(build)
            .await
            .map_err(|error| anyhow!("repository index rebuild task failed: {error}"))??;
        mode = rebuilt.1;
        if !matches!(mode, RepositoryIndexBuildMode::Noop) {
            rebuilt
                .0
                .save(&index_path)
                .map_err(|error| anyhow!("save repository code index: {error}"))?;
        }
        let (remaining, skipped_again) =
            crate::register_repository_sources_with_runtime(&runtime, &rebuilt.0, &root_path)
                .await?;
        registered = expected_source_files(&rebuilt.0).len() - remaining.len() - skipped_again;
        built = rebuilt;
    }
    Ok(RepositoryIndexRunResponse {
        mode: mode.as_str().to_string(),
        summary: RepositoryIndexSummary::from_index(&built.0.summary),
        registered,
        skipped,
    })
}

/// The live progress of the active repository index run. Deliberately
/// lightweight (no index load, no git) so the Studio can poll it while a
/// run is in flight.
///
/// # Cancellation
/// Cancelling drops the request; no state is changed.
pub(super) async fn progress(context: &ApiContext) -> Result<RepositoryIndexProgressResponse> {
    let _ = context;
    Ok(RepositoryIndexProgressResponse {
        progress: crate::repository_index_progress(),
    })
}

/// Load the persisted repository code index status for `root`: present,
/// its summary, and the current freshness (computed via git in a blocking
/// task).
///
/// # Cancellation
/// Cancelling drops the in-flight load/freshness tasks; no state is changed.
pub(super) async fn status(
    context: &ApiContext,
    root: String,
) -> Result<RepositoryIndexStatusResponse> {
    validate_scope_root(context, &root)?;
    let canonical_root = match PathBuf::from(&root).canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => PathBuf::from(&root),
    };
    let layout = context.layout.clone();
    let (index, present) = tokio::task::spawn_blocking(move || {
        let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
        match RepositoryCodeIndex::load(&index_path) {
            Ok(index) if Path::new(&index.summary.repository_root) == canonical_root.as_path() => {
                (Some(index), true)
            }
            _ => (None, false),
        }
    })
    .await
    .map_err(|error| anyhow!("repository index load task failed: {error}"))?;
    let freshness = if let Some(index) = &index {
        let index = index.clone();
        Some(
            tokio::task::spawn_blocking(move || index.freshness())
                .await
                .map_err(|error| anyhow!("repository freshness task failed: {error}"))??,
        )
    } else {
        None
    };
    Ok(RepositoryIndexStatusResponse {
        root,
        present,
        summary: index.map(|index| RepositoryIndexSummary::from_index(&index.summary)),
        freshness,
        progress: crate::repository_index_progress(),
    })
}

#[cfg(test)]
#[path = "repository_index_services_tests.rs"]
mod tests;
