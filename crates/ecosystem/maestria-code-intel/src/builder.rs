use super::{CodeIntelError, RepositoryCodeIndex};
use crate::changes::{build_delta, compute_delta_files};
use crate::identity::{discover_dirty_paths, discover_repository_identity};
use crate::incremental::candidate_id_prefix;
use crate::language::active_backends;
use crate::language::compose::{
    discover_all_packages, merge_extractions, resolve_merged_relations,
};
use crate::selection::{FileGate, RepositorySelection};
use crate::symbols::RelationCandidate;
use maestria_index_selection::IndexPolicy;
use std::collections::{BTreeMap, BTreeSet};
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
        Ok(Self::build_with_exclusions_and_candidates(
            root,
            parser_generation,
            excluded_patterns,
            &RepositorySelection::everything(),
            &BTreeMap::new(),
        )?
        .0)
    }

    /// Build a fresh index, also returning the full relation candidate list
    /// (persisted to a sidecar by the incremental rebuild). Only paths under
    /// `selection` are indexed, with per-directory `policies` applied by the
    /// [`FileGate`].
    pub(crate) fn build_with_exclusions_and_candidates(
        root: impl AsRef<Path>,
        parser_generation: impl Into<String>,
        excluded_patterns: &[String],
        selection: &RepositorySelection,
        policies: &BTreeMap<String, IndexPolicy>,
    ) -> Result<(Self, Vec<RelationCandidate>), CodeIntelError> {
        let root = root.as_ref();
        let parser_generation = parser_generation.into();
        let backends = active_backends(root, excluded_patterns)?;
        let initial_identity =
            discover_repository_identity(root, excluded_patterns, &backends, selection)?;
        let mut discovery = discover_all_packages(
            &backends,
            Path::new(&initial_identity.root),
            &initial_identity,
            &parser_generation,
            excluded_patterns,
        )?;
        let identity = discover_repository_identity(root, excluded_patterns, &backends, selection)?;
        if identity.commit != initial_identity.commit
            || identity.worktree_identity != initial_identity.worktree_identity
        {
            discovery = discover_all_packages(
                &backends,
                Path::new(&identity.root),
                &identity,
                &parser_generation,
                excluded_patterns,
            )?;
        }
        // Packages and targets outside the selection never participate:
        // whole-package drops and per-target drops happen before any parse
        // (bulk skip), and the FileGate additionally applies the
        // per-directory size/minified policies.
        let gate = FileGate::new(selection.clone(), policies.clone());
        let packages = Self::filter_selected_packages(discovery.packages, root, selection, &gate);
        // From-scratch full builds have no prior index to diff against, so the
        // delta is the porcelain dirty set only (git metadata, no content
        // reads); incremental rebuilds add the baseline..HEAD diff. The sets
        // are scoped to the selection.
        let dirty: BTreeSet<String> = discover_dirty_paths(root)?
            .into_iter()
            .filter(|path| selection.contains(path))
            .collect();
        let delta_files: BTreeSet<String> = compute_delta_files(root, None, &dirty)?
            .into_iter()
            .filter(|path| selection.contains(path))
            .collect();

        // Per-backend extraction merged into one index. Relations are
        // re-resolved from the merged candidate set so the deterministic
        // global ordering matches the incremental path exactly.
        let mut extractions = Vec::new();
        for backend in &backends {
            let backend_packages: Vec<_> = packages
                .iter()
                .filter(|package| backend.owns_package(package))
                .cloned()
                .collect();
            extractions.push(backend.extract(
                &backend_packages,
                Path::new(&identity.root),
                &identity,
                &parser_generation,
                excluded_patterns,
            )?);
        }
        let (mut symbols, mut candidates, mut file_contexts) = merge_extractions(extractions);
        // Defense filter: no record, context, or candidate survives outside
        // the selection or gated out by a policy. Relations are re-resolved
        // from the surviving candidate set, so they only ever connect
        // records that exist (R49/50).
        symbols.retain(|symbol| gate.allows(root, &symbol.provenance.file_path));
        file_contexts.retain(|key, _| gate.allows(root, key));
        candidates.retain(|candidate| gate.allows(root, &candidate_id_prefix(candidate)));
        let (relations, relation_summary) =
            resolve_merged_relations(&parser_generation, &symbols, &candidates);

        let files = symbols
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
                    symbol_count: symbols.len(),
                    file_count: files.len(),
                    packages: packages
                        .iter()
                        .map(|package| package.name.clone())
                        .collect(),
                    excluded_patterns: excluded_patterns.to_vec(),
                    workspace_warnings: discovery.warnings,
                    relation_summary,
                    changed: build_delta(&delta_files, &symbols),
                    selected_paths: selection.as_paths().map(str::to_string).collect(),
                    selection_policies: policies.clone(),
                },
                packages,
                symbols,
                relations,
                file_contexts,
            },
            candidates,
        ))
    }

    /// Drop packages and targets outside the selection (whole-package and
    /// per-target drops happen before any parse — bulk skip), with the
    /// `FileGate` applying the per-directory size/minified policies.
    fn filter_selected_packages(
        packages: Vec<crate::types::PackageRecord>,
        root: &Path,
        selection: &RepositorySelection,
        gate: &FileGate,
    ) -> Vec<crate::types::PackageRecord> {
        packages
            .into_iter()
            .filter_map(|mut package| {
                // Cargo metadata reports absolute target paths; walk-based
                // backends (Python, TypeScript) report relative ones.
                package.targets.retain(|target| {
                    let relative = Path::new(&target.src_path).strip_prefix(root).map_or_else(
                        |_| target.src_path.clone(),
                        |relative| relative.to_string_lossy().into_owned(),
                    );
                    selection.contains(&relative) && gate.allows(root, &relative)
                });
                if package.targets.is_empty() {
                    None
                } else {
                    Some(package)
                }
            })
            .collect()
    }
}
