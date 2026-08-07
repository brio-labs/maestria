//! Rust symbol extraction from workspace sources.
use crate::provenance::content_hash;

use crate::identity::RepositoryIdentity;
use crate::query::execute_query;
use crate::{
    CodeIntelError, CodeQuery, CodeRelationRecord, CodeRelationSummary, FileContextRecord,
    PackageRecord, QueryResult, SymbolRecord,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(crate) mod collect_rust;
pub(crate) mod comments;
mod compound;
pub(crate) mod context;
pub(crate) mod extract;
pub(crate) mod markers;
mod probe;
pub(crate) mod relation;
pub(crate) use relation::RelationCandidate;
mod relation_paths;
mod trait_methods;
mod utils;

/// Everything extracted from repository sources in one pass, including the
/// per-file contexts and relation candidates the incremental rebuild needs.
pub(crate) struct SymbolExtraction {
    pub symbols: Vec<SymbolRecord>,
    pub candidates: Vec<relation::RelationCandidate>,
    pub file_contexts: BTreeMap<String, FileContextRecord>,
    pub relations: Vec<CodeRelationRecord>,
    pub relation_summary: CodeRelationSummary,
}

/// Extract symbols from all workspace targets.
/// Shared per-build extraction inputs threaded through target extraction.
struct TargetExtractionState<'a> {
    canonical_root: &'a Path,
    identity: &'a RepositoryIdentity,
    parser_generation: &'a str,
    excluded_patterns: &'a [String],
    seen_files: &'a mut BTreeSet<std::path::PathBuf>,
    file_contexts: &'a mut BTreeMap<String, FileContextRecord>,
}

pub(crate) fn extract_symbols(
    packages: &[PackageRecord],
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<SymbolExtraction, CodeIntelError> {
    let mut symbols = Vec::new();
    let mut relation_candidates = Vec::new();
    let mut file_contexts = BTreeMap::new();
    let mut seen_files = BTreeSet::new();
    let canonical_root = root
        .canonicalize()
        .map_err(|error| CodeIntelError::Identity {
            context: "canonicalize repository root for source extraction".to_string(),
            details: error.to_string(),
        })?;
    let mut state = TargetExtractionState {
        canonical_root: &canonical_root,
        identity,
        parser_generation,
        excluded_patterns,
        seen_files: &mut seen_files,
        file_contexts: &mut file_contexts,
    };

    for package in packages {
        for target in &package.targets {
            let (mut target_symbols, mut target_candidates) =
                extract_target_symbols(package.name.as_str(), target, &mut state)?;
            symbols.append(&mut target_symbols);
            relation_candidates.append(&mut target_candidates);
        }
    }

    let relations = relation::resolve_relations(parser_generation, &symbols, &relation_candidates);
    let relation_summary = relation::relation_status_summary(relations.len());
    Ok(SymbolExtraction {
        symbols,
        candidates: relation_candidates,
        file_contexts,
        relations,
        relation_summary,
    })
}

fn extract_target_symbols(
    package_name: &str,
    target: &crate::TargetRecord,
    state: &mut TargetExtractionState<'_>,
) -> Result<(Vec<SymbolRecord>, Vec<relation::RelationCandidate>), CodeIntelError> {
    let canonical_root = state.canonical_root;
    let target_path = Path::new(&target.src_path);
    let target_root = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        canonical_root.join(target_path)
    };
    let target_root = target_root
        .canonicalize()
        .map_err(|error| CodeIntelError::Io {
            operation: "canonicalize cargo target source".to_string(),
            path: target_root.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
    if !target_root.starts_with(canonical_root) {
        return Err(CodeIntelError::Identity {
            context: "validate Cargo target source scope".to_string(),
            details: format!(
                "target {} points outside repository root: {}",
                target.name,
                target_root.display()
            ),
        });
    }

    let mut files = Vec::new();
    let mut module_contexts = BTreeMap::new();
    let mut parents = BTreeMap::new();
    let root_context = collect_rust::ModuleContext {
        stack: Vec::new(),
        is_test: false,
        is_bench: false,
    };
    collect_rust::collect_rust_files(
        &target_root,
        canonical_root,
        &mut files,
        state.excluded_patterns,
        &root_context,
        &mut module_contexts,
        &mut parents,
    )?;

    let mut symbols = Vec::new();
    let mut relation_candidates = Vec::new();
    for file in files {
        let file = file.canonicalize().map_err(|error| CodeIntelError::Io {
            operation: "canonicalize Rust source".to_string(),
            path: file.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        if !file.starts_with(canonical_root) || !state.seen_files.insert(file.clone()) {
            continue;
        }
        let (mut extracted_symbols, mut extracted_relation_candidates) = extract_file(
            file,
            package_name,
            target,
            state,
            &parents,
            &module_contexts,
        )?;
        symbols.append(&mut extracted_symbols);
        relation_candidates.append(&mut extracted_relation_candidates);
    }
    Ok((symbols, relation_candidates))
}

/// Extract one canonical source file with its module context and record its
/// per-file extraction context.
fn extract_file(
    file: std::path::PathBuf,
    package_name: &str,
    target: &crate::TargetRecord,
    state: &mut TargetExtractionState<'_>,
    parents: &BTreeMap<std::path::PathBuf, std::path::PathBuf>,
    module_contexts: &BTreeMap<std::path::PathBuf, collect_rust::ModuleContext>,
) -> Result<(Vec<SymbolRecord>, Vec<relation::RelationCandidate>), CodeIntelError> {
    let canonical_root = state.canonical_root;
    let relative_path = file
        .strip_prefix(canonical_root)
        .map_err(|error| CodeIntelError::Identity {
            context: "derive source provenance path".to_string(),
            details: error.to_string(),
        })?
        .to_string_lossy()
        .into_owned();
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
    let module_context = match module_contexts.get(&file) {
        Some(context) => context.clone(),
        None => collect_rust::ModuleContext {
            stack: Vec::new(),
            is_test: false,
            is_bench: false,
        },
    };
    state.file_contexts.insert(
        relative_path.clone(),
        FileContextRecord {
            package: package_name.to_string(),
            target: target.name.clone(),
            is_test_target: target.kind.iter().any(|kind| kind == "test"),
            is_bench_target: target.kind.iter().any(|kind| kind == "bench"),
            stack: module_context.stack.clone(),
            is_test: module_context.is_test,
            is_bench: module_context.is_bench,
            parent: parents.get(&file).and_then(|parent| {
                parent
                    .strip_prefix(canonical_root)
                    .ok()
                    .map(|path| path.to_string_lossy().into_owned())
            }),
        },
    );
    let file_context = context::FileContext {
        package: package_name,
        target: target.name.as_str(),
        relative_path,
        content_hash: source_content_hash,
        identity: state.identity,
        parser_generation: state.parser_generation,
        file_markers: markers::file_markers(&file, &source),
        is_test_target: target.kind.iter().any(|kind| kind == "test") || module_context.is_test,
        is_bench_target: target.kind.iter().any(|kind| kind == "bench") || module_context.is_bench,
    };
    extract::extract_file_symbols(&source, &file_context, &module_context.stack)
}

/// Re-derive module contexts for a dirty file and every module reachable from
/// it (`mod` discovery), recording contexts and parent links. Used by the
/// incremental rebuild to decide which files need re-extraction and to drop
/// modules that are no longer reachable.
pub(crate) fn derive_subtree_contexts(
    root: &Path,
    file: &Path,
    excluded_patterns: &[String],
    start: collect_rust::ModuleContext,
    out: &mut Vec<std::path::PathBuf>,
    contexts: &mut BTreeMap<std::path::PathBuf, collect_rust::ModuleContext>,
    parents: &mut BTreeMap<std::path::PathBuf, std::path::PathBuf>,
) -> Result<(), CodeIntelError> {
    collect_rust::collect_source_and_modules(
        file,
        root,
        out,
        excluded_patterns,
        &start,
        contexts,
        parents,
    )
}

/// Query extracted symbols. `changed_files` is the changed file set for
/// `CodeQuery::Changed`; other queries ignore it.
pub(crate) fn query_symbols<E, F>(
    symbols: &[SymbolRecord],
    query: CodeQuery,
    limit: usize,
    changed_files: Option<&std::collections::BTreeSet<String>>,
    authorize: &mut F,
) -> Result<QueryResult, E>
where
    F: FnMut(&SymbolRecord) -> Result<bool, E>,
{
    execute_query(symbols, query, limit, changed_files, authorize)
}
