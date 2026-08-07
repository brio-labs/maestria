//! Deleted, gitignored, dirty, and stale-source reconciliation passes.

use crate::CodeIntelError;
use crate::provenance::content_hash;
use crate::symbols::RelationCandidate;
use crate::symbols::collect_rust::ModuleContext;
use crate::symbols::context::FileContext;
use crate::symbols::{derive_subtree_contexts, extract, markers};
use crate::types::FileContextRecord;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::state::{RebuildInputs, RebuildState, parent_chain_reaches, rewrite_identity};

/// Deleted-file pass (handles `git rm` staged deletions): drop every context
/// key that is neither tracked nor present on disk.
pub(crate) fn drop_deleted_files(
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<(), CodeIntelError> {
    let deleted: Vec<String> = state
        .contexts
        .keys()
        .filter(|key| !inputs.file_set.contains(*key) && !inputs.root.join(key).exists())
        .cloned()
        .collect();
    for key in deleted {
        state.drop_file(key.clone());
        state.processed.insert(key);
    }
    Ok(())
}

/// Gitignored pass: gitignored files under target roots are extracted by the
/// walk but tracked by no file set; always re-extract.
pub(crate) fn reextract_gitignored(
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<(), CodeIntelError> {
    let gitignored: Vec<String> = state
        .contexts
        .keys()
        .filter(|key| !inputs.file_set.contains(*key))
        .cloned()
        .collect();
    for key in gitignored {
        if state.processed.contains(&key) {
            continue;
        }
        let record = state.contexts[&key].clone();
        let (symbols, extracted_candidates) = reextract_file(
            inputs.root,
            &key,
            &record,
            inputs.identity,
            inputs.parser_generation,
        )?;
        state.candidates_by_id_prefix.remove(&key);
        state.symbols_by_file.insert(key.clone(), symbols);
        if !state.files_in_order.iter().any(|file| file == &key) {
            state.appended_files.insert(key.clone());
        }
        state
            .candidates_by_id_prefix
            .entry(key.clone())
            .or_default()
            .extend(extracted_candidates.iter().cloned());
        state.new_candidates.extend(extracted_candidates);
        state.replaced_files.insert(key.clone());
        state.processed.insert(key);
    }
    Ok(())
}

/// Dirty-file pass: for every dirty `.rs` path, re-derive its module subtree
/// and re-extract exactly the files whose extraction inputs changed.
pub(crate) fn reconcile_dirty_files(
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<(), CodeIntelError> {
    let mut dirty_files: Vec<&String> = inputs
        .dirty
        .iter()
        .filter(|path| path.ends_with(".rs"))
        .collect();
    dirty_files.sort();
    for file in dirty_files {
        if state.processed.contains(file) {
            continue;
        }
        let absolute = inputs.root.join(file);
        if !absolute.exists() {
            state.drop_file(file.clone());
            state.processed.insert(file.clone());
            continue;
        }
        if !state.contexts.contains_key(file) {
            // New file not yet handled; the new-file check decides.
            continue;
        }
        reconcile_file(file, inputs, state)?;
    }
    Ok(())
}

/// Re-derive one dirty file's module subtree, re-extracting changed members
/// in place and dropping modules that are no longer reachable.
fn reconcile_file(
    file: &str,
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<(), CodeIntelError> {
    let absolute = inputs.root.join(file);
    let canonical = absolute
        .canonicalize()
        .map_err(|error| CodeIntelError::Io {
            operation: "canonicalize dirty Rust source".to_string(),
            path: absolute.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
    let record = state.contexts[file].clone();
    let mut out = Vec::new();
    let mut derived = BTreeMap::new();
    let mut derived_parents = BTreeMap::new();
    derive_subtree_contexts(
        &inputs.canonical_root,
        &canonical,
        inputs.excluded_patterns,
        ModuleContext {
            stack: record.stack.clone(),
            is_test: record.is_test,
            is_bench: record.is_bench,
        },
        &mut out,
        &mut derived,
        &mut derived_parents,
    )?;
    // Relative paths reachable from this dirty file's current parse.
    let derived_rels =
        reconcile_derived_children(out, derived, derived_parents, &record, inputs, state)?;
    // Unreachable cleanup — drop files whose parent chain reaches `file` but
    // which are no longer reachable from it. `processed` is not used here: a
    // file re-extracted by the gitignored pass whose `mod` declaration this
    // edit removed must still be dropped.
    let stale: Vec<String> = state
        .contexts
        .keys()
        .filter(|key| {
            key.as_str() != file
                && !derived_rels.contains(*key)
                && parent_chain_reaches(&state.contexts, key, file)
        })
        .cloned()
        .collect();
    for key in stale {
        state.drop_file(key.clone());
        state.processed.insert(key);
    }
    Ok(())
}

/// Re-extract or keep every file in a dirty file's derived module subtree.
/// Returns the relative paths reachable from the current parse.
fn reconcile_derived_children(
    out: Vec<std::path::PathBuf>,
    derived: BTreeMap<std::path::PathBuf, ModuleContext>,
    derived_parents: BTreeMap<std::path::PathBuf, std::path::PathBuf>,
    record: &FileContextRecord,
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<BTreeSet<String>, CodeIntelError> {
    let mut derived_rels = BTreeSet::new();
    for child in out {
        let rel = child
            .strip_prefix(&inputs.canonical_root)
            .map_err(|error| CodeIntelError::Identity {
                context: "derive incremental source path".to_string(),
                details: error.to_string(),
            })?
            .to_string_lossy()
            .into_owned();
        derived_rels.insert(rel.clone());
        let old = state.contexts.get(&rel).cloned();
        let new = &derived[&child];
        let should_reextract = old.is_none()
            || inputs.dirty.contains(&rel)
            || !inputs.file_set.contains(&rel)
            || old.as_ref().is_some_and(|old| {
                old.stack != new.stack || old.is_test != new.is_test || old.is_bench != new.is_bench
            });
        if should_reextract {
            let replacement = FileContextRecord {
                package: old
                    .as_ref()
                    .map_or_else(|| record.package.clone(), |old| old.package.clone()),
                target: old
                    .as_ref()
                    .map_or_else(|| record.target.clone(), |old| old.target.clone()),
                is_test_target: old
                    .as_ref()
                    .map_or(record.is_test_target, |old| old.is_test_target),
                is_bench_target: old
                    .as_ref()
                    .map_or(record.is_bench_target, |old| old.is_bench_target),
                stack: new.stack.clone(),
                is_test: new.is_test,
                is_bench: new.is_bench,
                parent: derived_parents
                    .get(&child)
                    .and_then(|parent| {
                        parent
                            .strip_prefix(&inputs.canonical_root)
                            .ok()
                            .map(|path| path.to_string_lossy().into_owned())
                    })
                    .or_else(|| {
                        // Derivation root (the dirty file itself): its module
                        // parent is outside this subtree and unchanged.
                        old.as_ref().and_then(|old| old.parent.clone())
                    }),
            };
            let (symbols, extracted_candidates) = reextract_file(
                inputs.root,
                &rel,
                &replacement,
                inputs.identity,
                inputs.parser_generation,
            )?;
            state.candidates_by_id_prefix.remove(&rel);
            state.symbols_by_file.insert(rel.clone(), symbols);
            if !state.files_in_order.iter().any(|file| file == &rel) {
                state.appended_files.insert(rel.clone());
            }
            state
                .candidates_by_id_prefix
                .entry(rel.clone())
                .or_default()
                .extend(extracted_candidates.iter().cloned());
            state.new_candidates.extend(extracted_candidates);
            state.replaced_files.insert(rel.clone());
            state.contexts.insert(rel.clone(), replacement);
        } else if let Some(old) = old {
            // Keep: rewrite identity fields, everything else unchanged.
            if let Some(symbols) = state.symbols_by_file.get_mut(&rel) {
                for symbol in symbols.iter_mut() {
                    rewrite_identity(&mut symbol.provenance, inputs.identity);
                }
            }
            state.contexts.insert(rel.clone(), old);
        }
        state.processed.insert(rel);
    }
    Ok(derived_rels)
}

/// Re-extract one file with the exact inputs `extract_target_symbols` would
/// have used for a file with this context record.
pub(crate) fn reextract_file(
    root: &Path,
    rel: &str,
    record: &FileContextRecord,
    identity: &crate::identity::RepositoryIdentity,
    parser_generation: &str,
) -> Result<(Vec<crate::SymbolRecord>, Vec<RelationCandidate>), CodeIntelError> {
    let file = root.join(rel);
    let source_bytes = fs::read(&file).map_err(|error| CodeIntelError::Io {
        operation: "read source file".to_string(),
        path: file.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let source_content_hash = content_hash(&source_bytes);
    let source = String::from_utf8(source_bytes).map_err(|error| CodeIntelError::Parse {
        context: format!("decode Rust source {}", file.display()),
        details: error.to_string(),
    })?;
    let file_context = FileContext {
        package: &record.package,
        target: &record.target,
        relative_path: rel.to_string(),
        content_hash: source_content_hash,
        identity,
        parser_generation,
        file_markers: markers::file_markers(&file, &source),
        is_test_target: record.is_test_target || record.is_test,
        is_bench_target: record.is_bench_target || record.is_bench,
    };
    extract::extract_file_symbols(&source, &file_context, &record.stack)
}

/// Files in `contexts` whose current on-disk content differs from the
/// content hash persisted on their extracted symbols (i.e. whose extraction
/// inputs changed since the previous build). Files with no symbols and files
/// that no longer exist are skipped (their fate is decided by the deleted
/// pass and the dirty pass).
pub(crate) fn discover_stale_content_files(
    root: &Path,
    contexts: &BTreeMap<String, FileContextRecord>,
    symbols: &[crate::SymbolRecord],
) -> Result<BTreeSet<String>, CodeIntelError> {
    let mut symbols_by_file: BTreeMap<&String, &crate::SymbolRecord> = BTreeMap::new();
    for symbol in symbols {
        symbols_by_file
            .entry(&symbol.provenance.file_path)
            .or_insert(symbol);
    }
    let mut stale = BTreeSet::new();
    for key in contexts.keys() {
        let Some(first) = symbols_by_file.get(key) else {
            continue;
        };
        let path = root.join(key);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(CodeIntelError::Io {
                    operation: "read source file for staleness check".to_string(),
                    path: path.to_string_lossy().into_owned(),
                    details: error.to_string(),
                });
            }
        };
        if content_hash(&bytes) != first.provenance.content_hash {
            stale.insert(key.clone());
        }
    }
    Ok(stale)
}
