//! Incremental repository index rebuild: re-parse only files whose extraction
//! inputs changed and patch the persisted index, producing a result exactly
//! equivalent to a full rebuild at the same repository state.
//!
//! - `assemble`: rebuilt index and candidate-list assembly.
//! - `candidates`: relation candidate sidecar persistence.
//! - `reconcile`: deleted, gitignored, dirty, and stale-source passes.
//! - `state`: working stores threaded through the rebuild phases.

use crate::CodeIntelError;
use crate::changes::compute_delta_files;
use crate::identity::{discover_dirty_paths, discover_file_set, discover_repository_identity};
use crate::language::{LanguageBackend, active_backends, is_backend_manifest_path};
use crate::selection::{FileGate, RepositorySelection};
use crate::types::{FileContextRecord, RepositoryCodeIndex};
use maestria_index_selection::IndexPolicy;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

mod assemble;
mod candidates;
mod reconcile;
mod state;

use assemble::{assemble_index, check_new_auto_targets};
use candidates::{load_relation_candidates, write_relation_candidates};
use reconcile::{
    discover_stale_content_files, drop_deleted_files, reconcile_dirty_files, reextract_gitignored,
};
pub(crate) use state::candidate_id_prefix;
use state::{RebuildInputs, RebuildState, rebuild_working_stores};

/// How the index was produced by [`build_or_update_repository_index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryIndexBuildMode {
    /// Index on disk already matches the current repository state; nothing was written.
    Noop,
    /// The persisted index was patched in place.
    Incremental,
    /// The index was rebuilt from scratch.
    Full,
}

impl RepositoryIndexBuildMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            RepositoryIndexBuildMode::Noop => "noop",
            RepositoryIndexBuildMode::Incremental => "incremental",
            RepositoryIndexBuildMode::Full => "full",
        }
    }
}

/// Whether any deleted path invalidates persisted package records (a Python
/// top-level package init removes a discovered target), forcing a full
/// rebuild. Unstaged deletions appear in the porcelain dirty set (still
/// cached in the git index); staged and committed deletions only in the
/// file-set comparison.
fn deletions_force_full(
    root: &Path,
    canonical_root: &Path,
    backends: &[Box<dyn LanguageBackend>],
    porcelain_dirty: &BTreeSet<String>,
    indexed_contexts: &BTreeMap<String, FileContextRecord>,
    file_set: &BTreeSet<String>,
) -> bool {
    let deleted_paths: Vec<String> = indexed_contexts
        .keys()
        .filter(|key| !file_set.contains(*key) && !root.join(key).exists())
        .cloned()
        .collect();
    let deletion_forces_full = |path: &str| {
        !root.join(path).exists()
            && backends
                .iter()
                .any(|backend| backend.deleted_file_requires_full(canonical_root, path))
    };
    deleted_paths.iter().any(|path| deletion_forces_full(path))
        || porcelain_dirty
            .iter()
            .any(|path| deletion_forces_full(path))
}

/// Persisted sidecar filename for the relation candidate list.
pub const REPOSITORY_CODE_CANDIDATES_FILENAME: &str = "repository-code-index.candidates.json";

/// Restrict a repository-wide path set to the selection.
fn selection_scoped(paths: BTreeSet<String>, selection: &RepositorySelection) -> BTreeSet<String> {
    paths
        .into_iter()
        .filter(|path| selection.contains(path))
        .collect()
}

/// Build or incrementally update the repository code index at `index_path`.
///
/// The sidecar at `candidates_path` is written by this function (before
/// returning); the caller saves the index last — the index rename is the
/// commit point, and a crash between the two renames self-heals on the next
/// rebuild. Returns `Noop` without touching either file when the persisted
/// index already matches the current repository identity. Identity, delta,
/// records, and freshness are scoped to `selection` with per-directory
/// `policies`; a selection or policy change forces a full rebuild.
pub fn build_or_update_repository_index(
    index_path: &Path,
    candidates_path: &Path,
    root: &Path,
    parser_generation: &str,
    excluded_patterns: &[String],
    selection: &RepositorySelection,
    policies: &BTreeMap<String, IndexPolicy>,
) -> Result<(RepositoryCodeIndex, RepositoryIndexBuildMode), CodeIntelError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| CodeIntelError::Identity {
            context: "canonicalize repository root for index update".to_string(),
            details: error.to_string(),
        })?;
    let full = |candidates_path: &Path| {
        let (index, candidates) = RepositoryCodeIndex::build_with_exclusions_and_candidates(
            root,
            parser_generation,
            excluded_patterns,
            selection,
            policies,
        )?;
        write_relation_candidates(candidates_path, parser_generation, &candidates)?;
        Ok::<_, CodeIntelError>((index, RepositoryIndexBuildMode::Full))
    };

    let index = match RepositoryCodeIndex::load(index_path) {
        Ok(index) => index,
        Err(_) => return full(candidates_path),
    };
    if index.summary.repository_root != canonical_root.to_string_lossy() {
        return full(candidates_path);
    }
    if index.is_stale_generation(parser_generation)
        || index.summary.excluded_patterns != *excluded_patterns
        || index.summary.selected_paths != selection.as_paths().collect::<Vec<_>>()
        || index.summary.selection_policies != *policies
    {
        return full(candidates_path);
    }
    let backends = active_backends(root, excluded_patterns)?;
    let identity = discover_repository_identity(root, excluded_patterns, &backends, selection)?;
    if index.file_contexts.is_empty() && !index.symbols.is_empty() {
        // An index with symbols but no contexts cannot be patched; rebuild.
        return full(candidates_path);
    }
    if identity.commit == index.summary.commit_sha
        && identity.worktree_identity == index.summary.worktree_identity
    {
        return Ok((index, RepositoryIndexBuildMode::Noop));
    }
    let Some(candidates) = load_relation_candidates(candidates_path, parser_generation)? else {
        return full(candidates_path);
    };
    // The porcelain dirty set drives both re-extraction and the changed
    // delta; the delta uses it BEFORE the content-staleness pass extends it,
    // because the delta rule is exactly "porcelain dirty set plus baseline
    // diff", never content-derived files. All sets are scoped to the
    // selection: edits outside it never trigger a rebuild or a delta entry.
    let porcelain_dirty = selection_scoped(discover_dirty_paths(root)?, selection);
    let mut dirty = porcelain_dirty.clone();
    let manifest_edit = |path: &String| {
        path.ends_with(".toml")
            || path.ends_with(".lock")
            || is_backend_manifest_path(path, &backends)
    };
    if dirty.iter().any(manifest_edit) {
        return full(candidates_path);
    }
    let file_set = selection_scoped(discover_file_set(root)?, selection);

    // Deletions that invalidate persisted package records (a Python
    // top-level package init) force a full rebuild, like a dirty manifest.
    if deletions_force_full(
        root,
        &canonical_root,
        &backends,
        &porcelain_dirty,
        &index.file_contexts,
        &file_set,
    ) {
        return full(candidates_path);
    }

    // Content-staleness pass: files whose on-disk content differs from the
    // persisted hashes (staged/committed/reverted edits porcelain misses).
    let stale = discover_stale_content_files(root, &index.file_contexts, &index.symbols)?;
    dirty.extend(stale);

    // Baseline for the persisted changed delta: the commit of the index being
    // replaced, read before the rebuilt index overwrites it.
    let delta_files = selection_scoped(
        compute_delta_files(root, Some(&index.summary.commit_sha), &porcelain_dirty)?,
        selection,
    );

    let mut state = rebuild_working_stores(&index, &candidates);
    let inputs = RebuildInputs {
        root,
        canonical_root,
        excluded_patterns,
        identity: &identity,
        parser_generation,
        file_set,
        dirty,
        delta_files,
        backends,
        selection_paths: selection.as_paths().map(str::to_string).collect(),
        selection_policies: policies.clone(),
        file_gate: FileGate::new(selection.clone(), policies.clone()),
    };
    match complete_incremental(
        &inputs,
        &index,
        &mut state,
        &candidates,
        candidates_path,
        parser_generation,
    )? {
        Some(rebuilt) => Ok(rebuilt),
        None => full(candidates_path),
    }
}

/// Runs the incremental extraction passes over the prepared inputs and
/// persists the rebuilt index and relation candidates. Returns `None` when
/// a newly discovered auto target forces a full rebuild instead.
fn complete_incremental(
    inputs: &RebuildInputs,
    index: &RepositoryCodeIndex,
    state: &mut RebuildState,
    candidates: &[crate::symbols::RelationCandidate],
    candidates_path: &Path,
    parser_generation: &str,
) -> Result<Option<(RepositoryCodeIndex, RepositoryIndexBuildMode)>, CodeIntelError> {
    drop_deleted_files(inputs, state)?;
    reextract_gitignored(inputs, state)?;
    reconcile_dirty_files(inputs, state)?;
    if check_new_auto_targets(inputs, index, state)? {
        return Ok(None);
    }
    let rebuilt = assemble_index(inputs, index, state, candidates)?;
    rebuilt
        .0
        .validate_provenance()
        .map_err(|error| CodeIntelError::Integrity {
            context: "incremental index".to_string(),
            details: error.to_string(),
        })?;
    write_relation_candidates(candidates_path, parser_generation, &rebuilt.1)?;
    Ok(Some((rebuilt.0, RepositoryIndexBuildMode::Incremental)))
}
