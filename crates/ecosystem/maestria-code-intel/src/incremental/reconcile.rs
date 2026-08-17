//! Deleted, gitignored, dirty, and stale-source reconciliation passes.

use crate::CodeIntelError;
use crate::language::{DerivedFileContext, backend_for_path};
use crate::symbols::RelationCandidate;
use crate::types::FileContextRecord;
use maestria_domain::content_hash;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::state::{RebuildInputs, RebuildState, parent_chain_reaches, rewrite_identity};

/// One re-extracted file: symbols plus relation candidates.
type ReextractedFile = (Vec<crate::SymbolRecord>, Vec<RelationCandidate>);

/// Whether a repository-relative path is a source file of any active backend.
fn is_backend_source_file(path: &str, inputs: &RebuildInputs) -> bool {
    backend_for_path(&inputs.backends, path).is_some()
}

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
        let reextracted = reextract_via_backend(inputs, &key, &record)?;
        let Some((symbols, extracted_candidates)) = reextracted else {
            state.drop_file(key.clone());
            state.processed.insert(key.clone());
            continue;
        };
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

/// Dirty-file pass: for every dirty source path (any active backend), re-derive
/// its subtree and re-extract exactly the files whose extraction inputs changed.
pub(crate) fn reconcile_dirty_files(
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<(), CodeIntelError> {
    let mut dirty_files: Vec<&String> = inputs
        .dirty
        .iter()
        .filter(|path| is_backend_source_file(path, inputs))
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

/// Re-derive one dirty file's subtree through its language backend,
/// re-extracting changed members in place and dropping files that are no
/// longer reachable.
fn reconcile_file(
    file: &str,
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<(), CodeIntelError> {
    let absolute = inputs.root.join(file);
    let canonical = absolute
        .canonicalize()
        .map_err(|error| CodeIntelError::Io {
            operation: "canonicalize dirty source".to_string(),
            path: absolute.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
    let record = state.contexts[file].clone();
    let Some(backend) = backend_for_path(&inputs.backends, file) else {
        return Ok(());
    };
    let derived = backend.derive_subtree_contexts(
        &inputs.canonical_root,
        &canonical,
        &record,
        inputs.excluded_patterns,
    )?;
    // Relative paths reachable from this dirty file's current parse.
    let derived_rels = reconcile_derived_children(derived, &record, inputs, state)?;
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

/// Re-extract or keep every file in a dirty file's derived subtree. Returns
/// the relative paths reachable from the current parse.
fn reconcile_derived_children(
    derived: Vec<(std::path::PathBuf, DerivedFileContext)>,
    record: &FileContextRecord,
    inputs: &RebuildInputs,
    state: &mut RebuildState,
) -> Result<BTreeSet<String>, CodeIntelError> {
    let mut derived_rels = BTreeSet::new();
    for (child, new_context) in derived {
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
        let should_reextract = old.is_none()
            || inputs.dirty.contains(&rel)
            || !inputs.file_set.contains(&rel)
            || old.as_ref().is_some_and(|old| {
                old.stack != new_context.stack
                    || old.is_test != new_context.is_test
                    || old.is_bench != new_context.is_bench
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
                stack: new_context.stack.clone(),
                is_test: new_context.is_test,
                is_bench: new_context.is_bench,
                parent: new_context.parent.clone(),
            };
            let reextracted = reextract_via_backend(inputs, &rel, &replacement)?;
            let Some((symbols, extracted_candidates)) = reextracted else {
                state.drop_file(rel.clone());
                continue;
            };
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

/// Re-extract one file through its language backend, mapping a `None`
/// (no-longer-extractable) result to a dropped file. A file gated out by
/// the selection's policies (grown past `max_file_bytes`, become minified)
/// is also dropped — the incremental fix path for policy changes.
fn reextract_via_backend(
    inputs: &RebuildInputs,
    rel: &str,
    record: &FileContextRecord,
) -> Result<Option<ReextractedFile>, CodeIntelError> {
    if !inputs.file_gate.allows(inputs.root, rel) {
        return Ok(None);
    }
    let Some(backend) = backend_for_path(&inputs.backends, rel) else {
        return Ok(None);
    };
    let Some(reextracted) = backend.reextract_file(
        inputs.root,
        rel,
        record,
        inputs.identity,
        inputs.parser_generation,
        inputs.excluded_patterns,
    )?
    else {
        return Ok(None);
    };
    Ok(Some((reextracted.0, reextracted.1)))
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
