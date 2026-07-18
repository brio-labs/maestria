//! Rust symbol extraction from workspace sources.

use crate::identity::RepositoryIdentity;
use crate::query::execute_query;
use crate::{CodeIntelError, CodeQuery, PackageRecord, QueryResult, SymbolRecord};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

mod collect_rust;
mod context;
mod extract;
mod markers;
mod probe;
mod trait_methods;
mod utils;

/// Extract symbols from all workspace targets.
pub(crate) fn extract_symbols(
    packages: &[PackageRecord],
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<Vec<SymbolRecord>, CodeIntelError> {
    let mut symbols = Vec::new();
    let mut seen_files = BTreeSet::new();
    let canonical_root = root
        .canonicalize()
        .map_err(|error| CodeIntelError::Identity {
            context: "canonicalize repository root for source extraction".to_string(),
            details: error.to_string(),
        })?;

    for package in packages {
        for target in &package.targets {
            let target_path = Path::new(&target.src_path);
            let target_root = if target_path.is_absolute() {
                target_path.to_path_buf()
            } else {
                canonical_root.join(target_path)
            };
            let target_root = target_root
                .canonicalize()
                .map_err(|error| CodeIntelError::Io {
                    operation: "canonicalize Cargo target source".to_string(),
                    path: target_root.to_string_lossy().into_owned(),
                    details: error.to_string(),
                })?;
            if !target_root.starts_with(&canonical_root) {
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
            let root_context = collect_rust::ModuleContext {
                stack: Vec::new(),
                is_test: false,
                is_bench: false,
            };
            collect_rust::collect_rust_files(
                &target_root,
                &canonical_root,
                &mut files,
                excluded_patterns,
                &root_context,
                &mut module_contexts,
            )?;
            for file in files {
                let file = file.canonicalize().map_err(|error| CodeIntelError::Io {
                    operation: "canonicalize Rust source".to_string(),
                    path: file.to_string_lossy().into_owned(),
                    details: error.to_string(),
                })?;
                if !file.starts_with(&canonical_root) || !seen_files.insert(file.clone()) {
                    continue;
                }
                let relative_path = file
                    .strip_prefix(&canonical_root)
                    .map_err(|error| CodeIntelError::Identity {
                        context: "derive source provenance path".to_string(),
                        details: error.to_string(),
                    })?
                    .to_string_lossy()
                    .into_owned();

                let source = fs::read_to_string(&file).map_err(|error| CodeIntelError::Io {
                    operation: "read source file".to_string(),
                    path: file.to_string_lossy().into_owned(),
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
                let file_context = context::FileContext {
                    package: package.name.as_str(),
                    target: target.name.as_str(),
                    relative_path,
                    identity,
                    parser_generation,
                    file_markers: markers::file_markers(&file, &source),
                    is_test_target: target.kind.iter().any(|kind| kind == "test")
                        || module_context.is_test,
                    is_bench_target: target.kind.iter().any(|kind| kind == "bench")
                        || module_context.is_bench,
                };

                let mut extracted =
                    extract::extract_file_symbols(&source, &file_context, &module_context.stack)?;

                symbols.append(&mut extracted);
            }
        }
    }
    Ok(symbols)
}

/// Query extracted symbols.
pub(crate) fn query_symbols(
    symbols: &[SymbolRecord],
    query: CodeQuery,
    limit: usize,
) -> QueryResult {
    execute_query(symbols, query, limit)
}
