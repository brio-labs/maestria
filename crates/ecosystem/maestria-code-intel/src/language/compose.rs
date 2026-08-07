//! Cross-backend composition: discovery, extraction, and relation
//! resolution over every active language backend.
//!
//! The backend boundary types and the `LanguageBackend` trait live in the
//! parent module; this module owns the composition of backend outputs into
//! one backend-neutral repository extraction, kept separate so the boundary
//! declarations stay reviewable and the composition stays independently
//! testable (R17).

use super::{
    BackendDiscovery, LanguageBackend, RelationCandidate, RepositoryIdentity, SymbolExtraction,
};
use crate::CodeIntelError;
use crate::types::{FileContextRecord, SymbolRecord};
use std::collections::BTreeSet;
use std::path::Path;

/// Discover packages from every active backend, deduped by package id,
/// backend order first (Rust, then Python, then TypeScript).
pub(crate) fn discover_all_packages(
    backends: &[Box<dyn LanguageBackend>],
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<BackendDiscovery, CodeIntelError> {
    let mut discovery = BackendDiscovery::default();
    let mut seen_ids = BTreeSet::new();
    for backend in backends {
        let found =
            backend.discover_packages(root, identity, parser_generation, excluded_patterns)?;
        for package in found.packages {
            if seen_ids.insert(package.package_id.clone()) {
                discovery.packages.push(package);
            }
        }
        discovery.warnings.extend(found.warnings);
    }
    Ok(discovery)
}

/// Merge per-backend extractions into one backend-neutral extraction.
/// Symbols are deduped by record id (a file cannot belong to two backends;
/// if it does, the first wins), file contexts by path. Relations are NOT
/// merged from the per-backend results — the caller re-resolves the merged
/// candidate set so the deterministic global ordering matches the
/// incremental path.
pub(crate) fn merge_extractions(
    extractions: Vec<SymbolExtraction>,
) -> (
    Vec<SymbolRecord>,
    Vec<RelationCandidate>,
    std::collections::BTreeMap<String, FileContextRecord>,
) {
    let mut symbols = Vec::new();
    let mut candidates = Vec::new();
    let mut file_contexts = std::collections::BTreeMap::new();
    let mut seen_symbols = BTreeSet::new();
    for extraction in extractions {
        for symbol in extraction.symbols {
            if seen_symbols.insert(symbol.record_id.clone()) {
                symbols.push(symbol);
            }
        }
        candidates.extend(extraction.candidates);
        for (path, record) in extraction.file_contexts {
            file_contexts.entry(path).or_insert(record);
        }
    }
    (symbols, candidates, file_contexts)
}

/// Resolve the merged candidate set exactly like the incremental path does,
/// returning the global deterministic relation ordering plus its summary.
pub(crate) fn resolve_merged_relations(
    parser_generation: &str,
    symbols: &[SymbolRecord],
    candidates: &[RelationCandidate],
) -> (
    Vec<crate::types::CodeRelationRecord>,
    crate::types::CodeRelationSummary,
) {
    let relations =
        crate::symbols::relation::resolve_relations(parser_generation, symbols, candidates);
    let relation_summary = crate::symbols::relation::relation_status_summary(relations.len());
    (relations, relation_summary)
}
