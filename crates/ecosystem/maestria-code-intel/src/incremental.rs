//! Incremental repository index rebuild: re-parse only files whose extraction
//! inputs changed and patch the persisted index, producing a result exactly
//! equivalent to a full rebuild at the same repository state.
//!
//! - `assemble`: rebuilt index and candidate-list assembly.
//! - `candidates`: relation candidate sidecar persistence.
//! - `reconcile`: deleted, gitignored, dirty, and stale-source passes.
//! - `state`: working stores threaded through the rebuild phases.

use crate::CodeIntelError;
use crate::identity::{discover_dirty_paths, discover_file_set, discover_repository_identity};
use crate::types::RepositoryCodeIndex;
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
use state::{RebuildInputs, rebuild_working_stores};

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

/// Persisted sidecar filename for the relation candidate list.
pub const REPOSITORY_CODE_CANDIDATES_FILENAME: &str = "repository-code-index.candidates.json";

/// Build or incrementally update the repository code index at `index_path`.
///
/// The sidecar at `candidates_path` is written by this function (before
/// returning); the caller saves the index last — the index rename is the
/// commit point, and a crash between the two renames self-heals on the next
/// rebuild. Returns `Noop` without touching either file when the persisted
/// index already matches the current repository identity.
pub fn build_or_update_repository_index(
    index_path: &Path,
    candidates_path: &Path,
    root: &Path,
    parser_generation: &str,
    excluded_patterns: &[String],
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
    {
        return full(candidates_path);
    }
    let identity = discover_repository_identity(root, excluded_patterns)?;
    if index.file_contexts.is_empty() && !index.symbols.is_empty() {
        // Invariant guard: an index with symbols but no per-file contexts
        // cannot be patched incrementally (there is nothing to re-derive or
        // reconcile), so rebuild it now (before the Noop check).
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
    let mut dirty = discover_dirty_paths(root)?;
    if dirty
        .iter()
        .any(|path| path.ends_with(".toml") || path.ends_with(".lock"))
    {
        return full(candidates_path);
    }
    let file_set = discover_file_set(root)?;

    // Content-staleness pass: files whose on-disk content differs from what
    // the previous build extracted (symbols' persisted content hashes). This
    // catches changes porcelain cannot report as worktree edits — staged
    // edits, edits committed after indexing, and edits reverted after being
    // indexed (worktree equals the index blob in all of these, so the dirty
    // set is empty or incomplete while the extracted content is stale).
    let stale = discover_stale_content_files(root, &index.file_contexts, &index.symbols)?;
    dirty.extend(stale);

    let mut state = rebuild_working_stores(&index, &candidates);
    let inputs = RebuildInputs {
        root,
        canonical_root,
        excluded_patterns,
        identity: &identity,
        parser_generation,
        file_set,
        dirty,
    };
    drop_deleted_files(&inputs, &mut state)?;
    reextract_gitignored(&inputs, &mut state)?;
    reconcile_dirty_files(&inputs, &mut state)?;
    if check_new_auto_targets(&inputs, &index, &state)? {
        return full(candidates_path);
    }
    let rebuilt = assemble_index(&inputs, &index, &state, &candidates)?;
    rebuilt
        .0
        .validate_provenance()
        .map_err(|error| CodeIntelError::Integrity {
            context: "incremental index".to_string(),
            details: error.to_string(),
        })?;
    write_relation_candidates(candidates_path, parser_generation, &rebuilt.1)?;
    Ok((rebuilt.0, RepositoryIndexBuildMode::Incremental))
}
