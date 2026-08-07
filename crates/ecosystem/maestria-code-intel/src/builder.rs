use super::{CodeIntelError, RepositoryCodeIndex};
use crate::changes::{build_delta, compute_delta_files};
use crate::identity::{discover_dirty_paths, discover_repository_identity};
use crate::metadata::extract_workspace_packages;
use crate::symbols::RelationCandidate;
use crate::symbols::extract_symbols;
use std::collections::BTreeSet;
use std::path::Path;

impl RepositoryCodeIndex {
    /// Build a fresh index for `root` using `parser_generation`.
    pub fn build(
        root: impl AsRef<Path>,
        parser_generation: impl Into<String>,
    ) -> Result<Self, CodeIntelError> {
        Self::build_with_exclusions(root, parser_generation, &[])
    }

    /// Build an index while applying manifest-compatible path exclusions.
    pub fn build_with_exclusions(
        root: impl AsRef<Path>,
        parser_generation: impl Into<String>,
        excluded_patterns: &[String],
    ) -> Result<Self, CodeIntelError> {
        Ok(
            Self::build_with_exclusions_and_candidates(root, parser_generation, excluded_patterns)?
                .0,
        )
    }

    /// Build a fresh index, also returning the full relation candidate list
    /// (persisted to a sidecar by the incremental rebuild).
    pub(crate) fn build_with_exclusions_and_candidates(
        root: impl AsRef<Path>,
        parser_generation: impl Into<String>,
        excluded_patterns: &[String],
    ) -> Result<(Self, Vec<RelationCandidate>), CodeIntelError> {
        let root = root.as_ref();
        let parser_generation = parser_generation.into();
        let initial_identity = discover_repository_identity(root, excluded_patterns)?;
        let mut discovery = extract_workspace_packages(
            Path::new(&initial_identity.root),
            &initial_identity,
            &parser_generation,
            excluded_patterns,
        )?;
        let identity = discover_repository_identity(root, excluded_patterns)?;
        if identity.commit != initial_identity.commit
            || identity.worktree_identity != initial_identity.worktree_identity
        {
            discovery = extract_workspace_packages(
                Path::new(&identity.root),
                &identity,
                &parser_generation,
                excluded_patterns,
            )?;
        }
        let packages = discovery.packages;
        // From-scratch full builds have no prior index to diff against, so the
        // delta is the porcelain dirty set only (git metadata, no content
        // reads); incremental rebuilds add the baseline..HEAD diff.
        let dirty = discover_dirty_paths(root)?;
        let delta_files = compute_delta_files(root, None, &dirty)?;
        let extraction = extract_symbols(
            &packages,
            Path::new(&identity.root),
            &identity,
            &parser_generation,
            excluded_patterns,
        )?;

        let files = extraction
            .symbols
            .iter()
            .map(|symbol| symbol.provenance.file_path.clone())
            .collect::<BTreeSet<_>>();
        Ok((
            Self {
                summary: super::types::CodeIndexSummary {
                    repository_root: identity.root,
                    commit_sha: identity.commit,
                    worktree_identity: identity.worktree_identity,
                    parser_generation: super::types::ParserGeneration::new(parser_generation),
                    package_count: packages.len(),
                    target_count: packages.iter().map(|package| package.targets.len()).sum(),
                    symbol_count: extraction.symbols.len(),
                    file_count: files.len(),
                    packages: packages
                        .iter()
                        .map(|package| package.name.clone())
                        .collect(),
                    excluded_patterns: excluded_patterns.to_vec(),
                    workspace_warnings: discovery.warnings,
                    relation_summary: extraction.relation_summary,
                    changed: build_delta(&delta_files, &extraction.symbols),
                },
                packages,
                symbols: extraction.symbols,
                relations: extraction.relations,
                file_contexts: extraction.file_contexts,
            },
            extraction.candidates,
        ))
    }
}
