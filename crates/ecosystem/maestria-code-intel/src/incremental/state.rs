//! Working stores threaded through the incremental rebuild phases.

use crate::identity::RepositoryIdentity;
use crate::language::LanguageBackend;
use crate::selection::FileGate;
use crate::symbols::RelationCandidate;
use crate::types::{FileContextRecord, RepositoryCodeIndex};
use maestria_index_selection::IndexPolicy;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Immutable inputs shared by every rebuild phase.
pub(crate) struct RebuildInputs<'a> {
    pub(crate) root: &'a Path,
    pub(crate) canonical_root: std::path::PathBuf,
    pub(crate) excluded_patterns: &'a [String],
    pub(crate) identity: &'a RepositoryIdentity,
    pub(crate) parser_generation: &'a str,
    pub(crate) file_set: BTreeSet<String>,
    pub(crate) dirty: BTreeSet<String>,
    /// Changed file set persisted into the rebuilt summary: porcelain dirty
    /// set plus the diff between the replaced index's commit and HEAD.
    pub(crate) delta_files: BTreeSet<String>,
    /// Active language backends used to dispatch per-file re-extraction,
    /// subtree derivation, and the new-file check.
    pub(crate) backends: Vec<Box<dyn LanguageBackend>>,
    /// Repository-relative directories the rebuilt index covers (empty =
    /// whole repository), persisted into the rebuilt summary.
    pub(crate) selection_paths: Vec<String>,
    /// Per-directory policy overrides applied at build time, persisted
    /// into the rebuilt summary.
    pub(crate) selection_policies: BTreeMap<String, IndexPolicy>,
    /// Selection + policies gate applied per file: re-extraction, assembly,
    /// and candidate retention all check it, so records gated out by
    /// size/minified policy changes are dropped on rebuild.
    pub(crate) file_gate: FileGate,
}

/// Working stores threaded through the incremental rebuild phases.
pub(crate) struct RebuildState {
    pub(crate) symbols_by_file: BTreeMap<String, Vec<crate::SymbolRecord>>,
    pub(crate) files_in_order: Vec<String>,
    pub(crate) candidates_by_id_prefix: BTreeMap<String, Vec<RelationCandidate>>,
    pub(crate) candidate_prefixes: Vec<String>,
    pub(crate) contexts: BTreeMap<String, FileContextRecord>,
    pub(crate) processed: BTreeSet<String>,
    pub(crate) dropped_files: BTreeSet<String>,
    /// Files whose candidate groups were replaced by re-extraction.
    pub(crate) replaced_files: BTreeSet<String>,
    pub(crate) new_candidates: Vec<RelationCandidate>,
    pub(crate) appended_files: BTreeSet<String>,
}

/// Build the working stores from the persisted index and sidecar candidates.
pub(crate) fn rebuild_working_stores(
    index: &RepositoryCodeIndex,
    candidates: &[RelationCandidate],
) -> RebuildState {
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
    for candidate in candidates {
        let prefix = candidate_id_prefix(candidate);
        candidate_prefixes.push(prefix.clone());
        candidates_by_id_prefix
            .entry(prefix)
            .or_default()
            .push(candidate.clone());
    }
    RebuildState {
        symbols_by_file,
        files_in_order,
        candidates_by_id_prefix,
        candidate_prefixes,
        contexts: index.file_contexts.clone(),
        processed: BTreeSet::new(),
        dropped_files: BTreeSet::new(),
        replaced_files: BTreeSet::new(),
        new_candidates: Vec::new(),
        appended_files: BTreeSet::new(),
    }
}

impl RebuildState {
    /// Remove a file and every context key whose parent chain reaches it
    /// from the working stores. Every dropped key is marked processed so the
    /// new-file check never re-arms a full rebuild for a reconciled drop.
    pub(crate) fn drop_file(&mut self, key: String) {
        if !self.dropped_files.insert(key.clone()) {
            return;
        }
        self.processed.insert(key.clone());
        self.symbols_by_file.remove(&key);
        self.candidates_by_id_prefix.remove(&key);
        self.contexts.remove(&key);
        // Recursively drop every key whose parent is a key dropped by this
        // call.
        loop {
            let next: Vec<String> = self
                .contexts
                .keys()
                .filter(|candidate| {
                    self.contexts
                        .get(*candidate)
                        .and_then(|context| context.parent.as_ref())
                        .is_some_and(|parent| self.dropped_files.contains(parent))
                })
                .cloned()
                .collect();
            if next.is_empty() {
                break;
            }
            for child in next {
                if !self.dropped_files.insert(child.clone()) {
                    continue;
                }
                self.processed.insert(child.clone());
                self.symbols_by_file.remove(&child);
                self.candidates_by_id_prefix.remove(&child);
                self.contexts.remove(&child);
            }
        }
    }
}

/// Rewrite the repository identity fields of a record to the current identity.
pub(crate) fn rewrite_identity(
    provenance: &mut crate::types::RecordProvenance,
    identity: &RepositoryIdentity,
) {
    provenance.commit_sha = identity.commit.clone();
    provenance.worktree_identity = identity.worktree_identity.clone();
    provenance.repository_root = identity.root.clone();
}

/// Whether `key`'s parent chain (via `contexts[].parent`) reaches `ancestor`.
pub(crate) fn parent_chain_reaches(
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
pub(crate) fn candidate_id_prefix(candidate: &RelationCandidate) -> String {
    let id = match candidate {
        RelationCandidate::Defines {
            target_record_id, ..
        } => target_record_id,
        RelationCandidate::Imports {
            source_record_id, ..
        } => source_record_id,
        RelationCandidate::Calls {
            source_record_id, ..
        } => source_record_id,
        RelationCandidate::Implements {
            source_record_id, ..
        } => source_record_id,
        RelationCandidate::PythonCall {
            source_record_id, ..
        }
        | RelationCandidate::TypeScriptCall {
            source_record_id, ..
        } => source_record_id,
    };
    id.split_once(':')
        .map_or_else(String::new, |(prefix, _)| prefix.to_string())
}
