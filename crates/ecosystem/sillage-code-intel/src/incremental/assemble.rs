//! Rebuilt index and candidate-list assembly, and the new-file check.

use crate::CodeIntelError;
use crate::language::backend_for_path;
use crate::selection::RepositorySelection;
use crate::symbols::relation;
use crate::types::{CodeIndexSummary, ParserGeneration, RepositoryCodeIndex};
use crate::walk::is_excluded_path;
use std::collections::BTreeSet;
use std::path::Path;

use super::state::{RebuildInputs, RebuildState, rewrite_identity};

/// New-file check: only newly added auto-discovery targets can become
/// extractable without a manifest change or an edited parent. Any other
/// source file absent from contexts is unreachable for extraction (a full
/// build does not extract it either). Returns whether a full rebuild is
/// required.
pub(crate) fn check_new_auto_targets(
    inputs: &RebuildInputs,
    index: &RepositoryCodeIndex,
    state: &RebuildState,
) -> Result<bool, CodeIntelError> {
    // Every package's manifest directory, as a repository-relative path.
    // Cargo manifests are absolute (their root manifest strips to the empty
    // path); walk-based manifests (Python, TypeScript) are relative, so an
    // empty parent is mapped to the root explicitly.
    let package_roots: BTreeSet<String> = index
        .packages
        .iter()
        .filter_map(|package| {
            let parent = Path::new(&package.manifest_path).parent();
            match parent.and_then(|parent| parent.strip_prefix(&inputs.canonical_root).ok()) {
                Some(relative) if relative.as_os_str().is_empty() => Some(String::new()),
                Some(relative) => Some(relative.to_string_lossy().into_owned()),
                None if parent.is_some_and(|parent| parent.as_os_str().is_empty()) => {
                    Some(String::new())
                }
                None => None,
            }
        })
        .collect();
    // The persisted selection is the build configuration; a malformed one
    // falls back to whole-repo (conservative: more files can only force a
    // full rebuild, never skip a selected one).
    let selection = match RepositorySelection::try_from(inputs.selection_paths.clone()) {
        Ok(selection) => selection,
        Err(_) => RepositorySelection::everything(),
    };
    let mut walk_set = BTreeSet::new();
    for backend in &inputs.backends {
        walk_set.extend(backend.collect_source_files(
            inputs.root,
            inputs.excluded_patterns,
            Some(&selection),
        )?);
    }
    for path in inputs.file_set.union(&walk_set) {
        if is_excluded_path(Path::new(path), inputs.excluded_patterns) || !selection.contains(path)
        {
            continue;
        }
        let Some(backend) = backend_for_path(&inputs.backends, path) else {
            continue;
        };
        if !state.contexts.contains_key(path)
            && !state.processed.contains(path)
            && backend.is_new_auto_target_root(path, &package_roots)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Assemble the rebuilt index: symbols in original order with replaced files
/// in place and new files appended, candidates re-resolved deterministically,
/// packages and summary rewritten to the current identity. Returns the index
/// and the full reassembled candidate list (kept originals plus newly
/// extracted candidates) for the sidecar.
pub(crate) fn assemble_index(
    inputs: &RebuildInputs,
    index: &RepositoryCodeIndex,
    state: &RebuildState,
    candidates: &[crate::symbols::RelationCandidate],
) -> Result<(RepositoryCodeIndex, Vec<crate::symbols::RelationCandidate>), CodeIntelError> {
    // Rewrite the identity fields of every retained symbol to the current
    // identity (kept files were extracted under the old identity; re-extracted
    // files already carry the new one — idempotent).
    let mut symbols_by_file = state.symbols_by_file.clone();
    for records in symbols_by_file.values_mut() {
        for symbol in records.iter_mut() {
            rewrite_identity(&mut symbol.provenance, inputs.identity);
        }
    }
    let mut symbols = Vec::new();
    for file in &state.files_in_order {
        if let Some(records) = symbols_by_file.get(file) {
            symbols.extend(records.iter().cloned());
        }
    }
    for file in &state.appended_files {
        if let Some(records) = symbols_by_file.get(file) {
            symbols.extend(records.iter().cloned());
        }
    }
    // Records gated out by a policy change are dropped from the rebuilt index.
    let gate_root = Path::new(&inputs.identity.root);
    symbols.retain(|symbol| {
        inputs
            .file_gate
            .allows(gate_root, &symbol.provenance.file_path)
    });
    let mut reassembled_candidates = Vec::new();
    for (candidate, prefix) in candidates.iter().zip(&state.candidate_prefixes) {
        if state.dropped_files.contains(prefix)
            || state.replaced_files.contains(prefix)
            || !inputs.file_gate.allows(gate_root, prefix)
        {
            continue;
        }
        reassembled_candidates.push(candidate.clone());
    }
    reassembled_candidates.extend(state.new_candidates.iter().cloned());
    let relations =
        relation::resolve_relations(inputs.parser_generation, &symbols, &reassembled_candidates);

    let packages = filter_and_rewrite_packages(index.packages.clone(), gate_root, inputs);
    let symbol_files: BTreeSet<&String> = symbols
        .iter()
        .map(|symbol| &symbol.provenance.file_path)
        .collect();
    let changed = crate::changes::build_delta(&inputs.delta_files, &symbols);
    Ok((
        RepositoryCodeIndex {
            summary: CodeIndexSummary {
                repository_root: inputs.identity.root.clone(),
                commit_sha: inputs.identity.commit.clone(),
                worktree_identity: inputs.identity.worktree_identity.clone(),
                parser_generation: ParserGeneration::new(inputs.parser_generation.to_string()),
                package_count: packages.len(),
                target_count: packages.iter().map(|package| package.targets.len()).sum(),
                symbol_count: symbols.len(),
                file_count: symbol_files.len(),
                packages: packages
                    .iter()
                    .map(|package| package.name.clone())
                    .collect(),
                excluded_patterns: inputs.excluded_patterns.to_vec(),
                // Discovery only re-runs on Full builds; on an incremental
                // rebuild the manifest set is unchanged (any dirty
                // manifest/lock already forced Full), so the warnings from
                // the previous discovery pass stay valid.
                workspace_warnings: index.summary.workspace_warnings.clone(),
                relation_summary: relation::relation_status_summary(relations.len()),
                changed,
                selected_paths: inputs.selection_paths.clone(),
                selection_policies: inputs.selection_policies.clone(),
            },
            packages,
            symbols,
            relations,
            file_contexts: state
                .contexts
                .iter()
                .filter(|(key, _)| inputs.file_gate.allows(gate_root, key))
                .map(|(key, record)| (key.clone(), record.clone()))
                .collect(),
        },
        reassembled_candidates,
    ))
}

/// Drops packages whose every target is gated out by the selection and
/// rewrites retained package/dependency/target identities to the current
/// identity (kept packages were extracted under the old identity — the
/// rewrite keeps the rebuilt index fresh-build equal).
fn filter_and_rewrite_packages(
    packages: Vec<crate::types::PackageRecord>,
    gate_root: &Path,
    inputs: &RebuildInputs,
) -> Vec<crate::types::PackageRecord> {
    let mut packages: Vec<_> = packages
        .into_iter()
        .filter_map(|mut package| {
            package.targets.retain(|target| {
                let relative = Path::new(&target.src_path)
                    .strip_prefix(gate_root)
                    .map_or_else(
                        |_| target.src_path.clone(),
                        |relative| relative.to_string_lossy().into_owned(),
                    );
                inputs.file_gate.allows(gate_root, &relative)
            });
            if package.targets.is_empty() {
                None
            } else {
                Some(package)
            }
        })
        .collect();
    for package in packages.iter_mut() {
        rewrite_identity(&mut package.provenance, inputs.identity);
        for dependency in &mut package.dependencies {
            rewrite_identity(&mut dependency.provenance, inputs.identity);
        }
        for target in &mut package.targets {
            rewrite_identity(&mut target.provenance, inputs.identity);
        }
    }
    packages
}
