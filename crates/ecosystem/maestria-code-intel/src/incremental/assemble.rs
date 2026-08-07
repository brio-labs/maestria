//! Rebuilt index and candidate-list assembly, and the new-file check.

use crate::CodeIntelError;
use crate::language::backend_for_path;
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
    let package_roots: BTreeSet<String> = index
        .packages
        .iter()
        .filter_map(|package| {
            Path::new(&package.manifest_path)
                .parent()
                .and_then(|parent| parent.strip_prefix(&inputs.canonical_root).ok())
                .map(|relative| relative.to_string_lossy().into_owned())
        })
        .collect();
    let mut walk_set = BTreeSet::new();
    for backend in &inputs.backends {
        walk_set.extend(backend.collect_source_files(inputs.root, inputs.excluded_patterns)?);
    }
    for path in inputs.file_set.union(&walk_set) {
        if is_excluded_path(Path::new(path), inputs.excluded_patterns) {
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
    let mut reassembled_candidates = Vec::new();
    for (candidate, prefix) in candidates.iter().zip(&state.candidate_prefixes) {
        if state.dropped_files.contains(prefix) || state.replaced_files.contains(prefix) {
            continue;
        }
        reassembled_candidates.push(candidate.clone());
    }
    reassembled_candidates.extend(state.new_candidates.iter().cloned());
    let relations =
        relation::resolve_relations(inputs.parser_generation, &symbols, &reassembled_candidates);

    let mut packages = index.packages.clone();
    for package in packages.iter_mut() {
        rewrite_identity(&mut package.provenance, inputs.identity);
        for dependency in &mut package.dependencies {
            rewrite_identity(&mut dependency.provenance, inputs.identity);
        }
        for target in &mut package.targets {
            rewrite_identity(&mut target.provenance, inputs.identity);
        }
    }
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
            },
            packages,
            symbols,
            relations,
            file_contexts: state.contexts.clone(),
        },
        reassembled_candidates,
    ))
}
