//! Incremental repository index rebuild: re-parse only files whose extraction
//! inputs changed and patch the persisted index, producing a result exactly
//! equivalent to a full rebuild at the same repository state.

use crate::identity::{
    RepositoryIdentity, collect_rust_paths, discover_dirty_paths, discover_file_set,
    discover_repository_identity, is_excluded_path,
};
use crate::provenance::content_hash;
use crate::symbols::RelationCandidate;
use crate::symbols::collect_rust::ModuleContext;
use crate::symbols::context::FileContext;
use crate::symbols::{derive_subtree_contexts, extract, markers, relation};
use crate::types::{
    CodeIndexSummary, FileContextRecord, ParserGeneration, RepositoryCodeIndex,
};
use crate::CodeIntelError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

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

/// Sidecar payload: the full relation candidate set for one parser generation.
/// Candidates live outside the index so the daemon/query path never parses them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepositoryRelationCandidates {
    pub parser_generation: String,
    pub candidates: Vec<RelationCandidate>,
}

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
    if dirty.iter().any(|path| path.ends_with(".toml") || path.ends_with(".lock")) {
        return full(candidates_path);
    }
    let file_set = discover_file_set(root)?;

    // Working stores (Step 12).
    let mut symbols_by_file: BTreeMap<String, Vec<crate::SymbolRecord>> = BTreeMap::new();
    let mut files_in_order: Vec<String> = Vec::new();
    for symbol in &index.symbols {
        if !symbols_by_file.contains_key(&symbol.provenance.file_path) {
            files_in_order.push(symbol.provenance.file_path.clone());
        }
        symbols_by_file
            .entry(symbol.provenance.file_path.clone())
            .or_default()
            .push(symbol.clone());
    }
    let mut candidates_by_id_prefix: BTreeMap<String, Vec<RelationCandidate>> = BTreeMap::new();
    let mut candidate_prefixes: Vec<String> = Vec::new();
    for candidate in &candidates {
        let prefix = candidate_id_prefix(candidate);
        candidate_prefixes.push(prefix.clone());
        candidates_by_id_prefix.entry(prefix).or_default().push(candidate.clone());
    }
    let mut contexts = index.file_contexts.clone();
    let mut processed: BTreeSet<String> = BTreeSet::new();
    let mut dropped_files: BTreeSet<String> = BTreeSet::new();
    // Files whose candidate groups were replaced by re-extraction.
    let mut replaced_files: BTreeSet<String> = BTreeSet::new();
    let mut new_candidates: Vec<RelationCandidate> = Vec::new();
    let mut appended_files: BTreeSet<String> = BTreeSet::new();

    // Step 13: deleted-file pass (handles `git rm` staged deletions).
    let deleted: Vec<String> = contexts
        .keys()
        .filter(|key| !file_set.contains(*key) && !root.join(key).exists())
        .cloned()
        .collect();
    for key in deleted {
        drop_file(
            &mut contexts,
            &mut symbols_by_file,
            &mut candidates_by_id_prefix,
            &mut dropped_files,
            key.clone(),
        );
        processed.insert(key);
    }

    // Step 14: gitignored pass — gitignored files under target roots are
    // extracted by the walk but tracked by no file set; always re-extract.
    let gitignored: Vec<String> = contexts
        .keys()
        .filter(|key| !file_set.contains(*key))
        .cloned()
        .collect();
    for key in gitignored {
        if processed.contains(&key) {
            continue;
        }
        let record = contexts[&key].clone();
        let (symbols, extracted_candidates) =
            reextract_file(root, &key, &record, &identity, parser_generation)?;
        let prefix = key.clone();
        candidates_by_id_prefix.remove(&prefix);
        symbols_by_file.insert(key.clone(), symbols);
        if !files_in_order.iter().any(|file| file == &key) {
            appended_files.insert(key.clone());
        }
        candidates_by_id_prefix
            .entry(prefix.clone())
            .or_default()
            .extend(extracted_candidates.iter().cloned());
        new_candidates.extend(extracted_candidates);
        replaced_files.insert(key.clone());
        processed.insert(key);
    }

    // Content-staleness pass: files whose on-disk content differs from what
    // the previous build extracted (symbols' persisted content hashes). This
    // catches changes porcelain cannot report as worktree edits — staged
    // edits, edits committed after indexing, and edits reverted after being
    // indexed (worktree equals the index blob in all of these, so the dirty
    // set is empty or incomplete while the extracted content is stale).
    // Re-extraction is exact: `content_hash` is the whole-file SHA-256 used
    // at extraction time.
    let stale = discover_stale_content_files(root, &contexts, &symbols_by_file)?;
    if !stale.is_empty() {
        dirty.extend(stale);
    }

    // Step 15: dirty-file pass.
    let mut dirty_files: Vec<&String> = dirty.iter().filter(|path| path.ends_with(".rs")).collect();
    dirty_files.sort();
    for file in dirty_files {
        if processed.contains(file) {
            continue;
        }
        let absolute = root.join(file);
        if !absolute.exists() {
            drop_file(
                &mut contexts,
                &mut symbols_by_file,
                &mut candidates_by_id_prefix,
                &mut dropped_files,
                file.clone(),
            );
            processed.insert(file.clone());
            continue;
        }
        let Some(record) = contexts.get(file).cloned() else {
            // New file not yet handled; Step 16 decides.
            continue;
        };
        let canonical = absolute.canonicalize().map_err(|error| CodeIntelError::Io {
            operation: "canonicalize dirty Rust source".to_string(),
            path: absolute.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let mut out = Vec::new();
        let mut derived = BTreeMap::new();
        let mut derived_parents = BTreeMap::new();
        derive_subtree_contexts(
            &canonical_root,
            &canonical,
            excluded_patterns,
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
        let mut derived_rels = BTreeSet::new();
        for child in out {
            let rel = child
                .strip_prefix(&canonical_root)
                .map_err(|error| CodeIntelError::Identity {
                    context: "derive incremental source path".to_string(),
                    details: error.to_string(),
                })?
                .to_string_lossy()
                .into_owned();
            derived_rels.insert(rel.clone());
            let old = contexts.get(&rel).cloned();
            let new = &derived[&child];
            let should_reextract = old.is_none()
                || dirty.contains(&rel)
                || !file_set.contains(&rel)
                || old.as_ref().is_some_and(|old| {
                    old.stack != new.stack
                        || old.is_test != new.is_test
                        || old.is_bench != new.is_bench
                });
            if should_reextract {
                let replacement = FileContextRecord {
                    package: old
                        .as_ref()
                        .map(|old| old.package.clone())
                        .unwrap_or_else(|| record.package.clone()),
                    target: old
                        .as_ref()
                        .map(|old| old.target.clone())
                        .unwrap_or_else(|| record.target.clone()),
                    is_test_target: old
                        .as_ref()
                        .map(|old| old.is_test_target)
                        .unwrap_or(record.is_test_target),
                    is_bench_target: old
                        .as_ref()
                        .map(|old| old.is_bench_target)
                        .unwrap_or(record.is_bench_target),
                    stack: new.stack.clone(),
                    is_test: new.is_test,
                    is_bench: new.is_bench,
                    parent: derived_parents.get(&child).and_then(|parent| {
                        parent.strip_prefix(&canonical_root).ok().map(|path| {
                            path.to_string_lossy().into_owned()
                        })
                    }).or_else(|| {
                        // Derivation root (the dirty file itself): its module
                        // parent is outside this subtree and unchanged.
                        old.as_ref().and_then(|old| old.parent.clone())
                    }),
                };
                let (symbols, extracted_candidates) = reextract_file(
                    root,
                    &rel,
                    &replacement,
                    &identity,
                    parser_generation,
                )?;
                candidates_by_id_prefix.remove(&rel);
                symbols_by_file.insert(rel.clone(), symbols);
                if !files_in_order.iter().any(|file| file == &rel) {
                    appended_files.insert(rel.clone());
                }
                candidates_by_id_prefix
                    .entry(rel.clone())
                    .or_default()
                    .extend(extracted_candidates.iter().cloned());
                new_candidates.extend(extracted_candidates);
                replaced_files.insert(rel.clone());
                contexts.insert(rel.clone(), replacement);
            } else if let Some(old) = old {
                // Keep: rewrite identity fields, everything else unchanged.
                if let Some(symbols) = symbols_by_file.get_mut(&rel) {
                    for symbol in symbols.iter_mut() {
                        symbol.provenance.commit_sha = identity.commit.clone();
                        symbol.provenance.worktree_identity = identity.worktree_identity.clone();
                        symbol.provenance.repository_root = identity.root.clone();
                    }
                }
                contexts.insert(rel.clone(), old);
            }
            processed.insert(rel);
        }
        // Step 15c: unreachable cleanup — drop files whose parent chain
        // reaches `file` but which are no longer reachable from it. `processed`
        // is not used here: a file re-extracted by the gitignored pass (step
        // 14) whose `mod` declaration this edit removed must still be dropped.
        let stale: Vec<String> = contexts
            .keys()
            .filter(|key| {
                key.as_str() != file.as_str()
                    && !derived_rels.contains(*key)
                    && parent_chain_reaches(&contexts, key, file)
            })
            .cloned()
            .collect();
        for key in stale {
            drop_file(
                &mut contexts,
                &mut symbols_by_file,
                &mut candidates_by_id_prefix,
                &mut dropped_files,
                key.clone(),
            );
            processed.insert(key);
        }
    }

    // Step 16: new-file check. A full rebuild extracts only cargo target
    // roots and their module closures. The dirty pass re-derives the closure
    // of every edited file, so the only files that can become extractable
    // without a manifest change (handled by the toml check) or an edited
    // parent are newly added cargo auto-discovery targets (tests, benches,
    // examples, src/bin, build scripts, src/lib|main) inside member packages.
    // Any other `.rs` file absent from contexts is unreachable for extraction
    // (a full build does not extract it either) and cannot change the index.
    let package_roots: BTreeSet<String> = index
        .packages
        .iter()
        .filter_map(|package| {
            Path::new(&package.manifest_path)
                .parent()
                .and_then(|parent| parent.strip_prefix(&canonical_root).ok())
                .map(|relative| relative.to_string_lossy().into_owned())
        })
        .collect();
    let mut walk_set = BTreeSet::new();
    collect_rust_paths(root, root, &mut walk_set, excluded_patterns)?;
    for path in file_set.union(&walk_set) {
        if !path.ends_with(".rs") || is_excluded_path(Path::new(path), excluded_patterns) {
            continue;
        }
        if !contexts.contains_key(path) && !processed.contains(path) {
            if is_new_auto_target_root(Path::new(path), &package_roots) {
                return full(candidates_path);
            }
        }
    }

    // Step 17: assemble. First, rewrite the identity fields of every retained
    // symbol to the current identity (kept files were extracted under the old
    // identity; re-extracted files already carry the new one — idempotent).
    for records in symbols_by_file.values_mut() {
        for symbol in records.iter_mut() {
            rewrite_identity(&mut symbol.provenance, &identity);
        }
    }
    let mut symbols = Vec::new();
    for file in &files_in_order {
        if let Some(records) = symbols_by_file.get(file) {
            symbols.extend(records.iter().cloned());
        }
    }
    for file in &appended_files {
        if let Some(records) = symbols_by_file.get(file) {
            symbols.extend(records.iter().cloned());
        }
    }
    let mut reassembled_candidates = Vec::new();
    for (candidate, prefix) in candidates.iter().zip(&candidate_prefixes) {
        if dropped_files.contains(prefix) || replaced_files.contains(prefix) {
            continue;
        }
        reassembled_candidates.push(candidate.clone());
    }
    reassembled_candidates.extend(new_candidates);
    let relations =
        relation::resolve_relations(parser_generation, &symbols, &reassembled_candidates);

    let mut packages = index.packages.clone();
    for package in packages.iter_mut() {
        rewrite_identity(&mut package.provenance, &identity);
        for dependency in &mut package.dependencies {
            rewrite_identity(&mut dependency.provenance, &identity);
        }
        for target in &mut package.targets {
            rewrite_identity(&mut target.provenance, &identity);
        }
    }
    let symbol_files: BTreeSet<&String> = symbols
        .iter()
        .map(|symbol| &symbol.provenance.file_path)
        .collect();
    let rebuilt = RepositoryCodeIndex {
        summary: CodeIndexSummary {
            repository_root: identity.root.clone(),
            commit_sha: identity.commit.clone(),
            worktree_identity: identity.worktree_identity.clone(),
            parser_generation: ParserGeneration::new(parser_generation.to_string()),
            package_count: packages.len(),
            target_count: packages.iter().map(|package| package.targets.len()).sum(),
            symbol_count: symbols.len(),
            file_count: symbol_files.len(),
            packages: packages
                .iter()
                .map(|package| package.name.clone())
                .collect(),
            excluded_patterns: excluded_patterns.to_vec(),
            relation_summary: relation::relation_status_summary(relations.len()),
        },
        packages,
        symbols,
        relations,
        file_contexts: contexts,
    };
    rebuilt
        .validate_provenance()
        .map_err(|error| CodeIntelError::Integrity {
            context: "incremental index".to_string(),
            details: error.to_string(),
        })?;
    write_relation_candidates(candidates_path, parser_generation, &reassembled_candidates)?;
    Ok((rebuilt, RepositoryIndexBuildMode::Incremental))
}

/// Rewrite the repository identity fields of a record to the current identity.
fn rewrite_identity(
    provenance: &mut crate::types::RecordProvenance,
    identity: &RepositoryIdentity,
) {
    provenance.commit_sha = identity.commit.clone();
    provenance.worktree_identity = identity.worktree_identity.clone();
    provenance.repository_root = identity.root.clone();
}

/// Re-extract one file with the exact inputs `extract_target_symbols` would
/// have used for a file with this context record.
fn reextract_file(
    root: &Path,
    rel: &str,
    record: &FileContextRecord,
    identity: &RepositoryIdentity,
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
fn discover_stale_content_files(
    root: &Path,
    contexts: &BTreeMap<String, FileContextRecord>,
    symbols_by_file: &BTreeMap<String, Vec<crate::SymbolRecord>>,
) -> Result<BTreeSet<String>, CodeIntelError> {
    let mut stale = BTreeSet::new();
    for key in contexts.keys() {
        let Some(symbols) = symbols_by_file.get(key) else {
            continue;
        };
        let Some(first) = symbols.first() else {
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

/// Whether a not-yet-indexed `.rs` path is a plausible new cargo
/// auto-discovery target root: a file cargo turns into a target without any
/// manifest change, inside a member package. `package_roots` are the relative
/// repository paths of the package manifest directories from the loaded index.
fn is_new_auto_target_root(path: &Path, package_roots: &BTreeSet<String>) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if file_name == "mod.rs" {
        // cargo never auto-discovers `mod.rs` as a target.
        return false;
    }
    let parent = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned());
    let Some(parent) = parent else {
        return false;
    };
    let grandparent = path
        .parent()
        .and_then(|parent| parent.parent())
        .map(|parent| parent.to_string_lossy().into_owned());
    for root in package_roots {
        let base = if root.is_empty() {
            String::new()
        } else {
            format!("{root}/")
        };
        if file_name == "build.rs" && parent == *root {
            return true;
        }
        if matches!(file_name, "lib.rs" | "main.rs") && parent == format!("{base}src") {
            return true;
        }
        if parent == format!("{base}src/bin") {
            return true;
        }
        for directory in ["tests", "benches", "examples"] {
            if parent == format!("{base}{directory}") {
                return true;
            }
            // Multi-file target: `<dir>/<name>/main.rs`.
            let multi_root = format!("{base}{directory}");
            if file_name == "main.rs"
                && grandparent.as_deref() == Some(multi_root.as_str())
            {
                return true;
            }
        }
    }
    false
}

/// Remove a file and every context key whose parent chain reaches it from the
/// working stores.
fn drop_file(
    contexts: &mut BTreeMap<String, FileContextRecord>,
    symbols_by_file: &mut BTreeMap<String, Vec<crate::SymbolRecord>>,
    candidates_by_id_prefix: &mut BTreeMap<String, Vec<RelationCandidate>>,
    dropped: &mut BTreeSet<String>,
    key: String,
) {
    if !dropped.insert(key.clone()) {
        return;
    }
    symbols_by_file.remove(&key);
    candidates_by_id_prefix.remove(&key);
    contexts.remove(&key);
    // Recursively drop every key whose parent is a key dropped by this call.
    loop {
        let next: Vec<String> = contexts
            .keys()
            .filter(|candidate| {
                contexts
                    .get(*candidate)
                    .and_then(|context| context.parent.as_ref())
                    .is_some_and(|parent| dropped.contains(parent))
            })
            .cloned()
            .collect();
        if next.is_empty() {
            break;
        }
        for child in next {
            if !dropped.insert(child.clone()) {
                continue;
            }
            symbols_by_file.remove(&child);
            candidates_by_id_prefix.remove(&child);
            contexts.remove(&child);
        }
    }
}

/// Whether `key`'s parent chain (via `contexts[].parent`) reaches `ancestor`.
fn parent_chain_reaches(
    contexts: &BTreeMap<String, FileContextRecord>,
    key: &str,
    ancestor: &str,
) -> bool {
    let mut current = key.to_string();
    loop {
        let Some(parent) = contexts
            .get(&current)
            .and_then(|context| context.parent.clone())
        else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        current = parent;
    }
}

/// Path prefix of the record id identifying the file a candidate came from.
fn candidate_id_prefix(candidate: &RelationCandidate) -> String {
    let id = match candidate {
        RelationCandidate::Defines { target_record_id, .. } => target_record_id,
        RelationCandidate::Imports { source_record_id, .. } => source_record_id,
        RelationCandidate::Calls { source_record_id, .. } => source_record_id,
        RelationCandidate::Implements { source_record_id, .. } => source_record_id,
    };
    id.split_once(':')
        .map(|(prefix, _)| prefix.to_string())
        .unwrap_or_default()
}

/// Write the relation candidates sidecar atomically (tmp + rename).
pub(crate) fn write_relation_candidates(
    path: &Path,
    generation: &str,
    candidates: &[RelationCandidate],
) -> Result<(), CodeIntelError> {
    let payload = RepositoryRelationCandidates {
        parser_generation: generation.to_string(),
        candidates: candidates.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|error| CodeIntelError::Persist {
        context: "serialize relation candidates".to_string(),
        details: error.to_string(),
    })?;
    let temporary = path.with_extension("candidates.tmp");
    fs::write(&temporary, bytes).map_err(|error| CodeIntelError::Persist {
        context: "write temporary relation candidates file".to_string(),
        details: format!("{temporary:?}: {error}"),
    })?;
    fs::rename(&temporary, path).map_err(|error| CodeIntelError::Persist {
        context: "atomically replace relation candidates file".to_string(),
        details: format!("{path:?}: {error}"),
    })
}

/// Load the relation candidates sidecar; `None` when missing or built under a
/// different parser generation, error on corrupt JSON.
pub(crate) fn load_relation_candidates(
    path: &Path,
    expected_generation: &str,
) -> Result<Option<Vec<RelationCandidate>>, CodeIntelError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CodeIntelError::Persist {
                context: "read relation candidates file".to_string(),
                details: format!("{path:?}: {error}"),
            });
        }
    };
    let payload: RepositoryRelationCandidates =
        serde_json::from_slice(&bytes).map_err(|error| CodeIntelError::Persist {
            context: "deserialize relation candidates".to_string(),
            details: error.to_string(),
        })?;
    if payload.parser_generation != expected_generation {
        return Ok(None);
    }
    Ok(Some(payload.candidates))
}
