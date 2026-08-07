//! TypeScript/JavaScript language backend: bounded `package.json`
//! discovery, walk-based source collection, and tokenizer-based symbol
//! extraction. Mirrors the Python backend's module layout.

pub(crate) mod calls;
pub(crate) mod discover;
pub(crate) mod extract;
pub(crate) mod imports;
pub(crate) mod jsx;
pub(crate) mod manifest;
pub(crate) mod statements;
pub(crate) mod tokens;

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::language::typescript::discover::{
    TS_IDENTITY_INPUTS, TS_MANIFEST_NAMES, discover_web_packages,
};
use crate::language::typescript::extract::extract_web_file;
use crate::language::typescript::tokens::{
    TS_SOURCE_EXTENSIONS, is_bench_file, is_test_file, module_path_for_file,
};
use crate::language::{BackendDiscovery, DerivedFileContext, LanguageBackend, LanguageKind};
use crate::provenance::content_hash;
use crate::symbols::RelationCandidate;
use crate::symbols::SymbolExtraction;
use crate::symbols::context::FileContext;
use crate::types::{FileContextRecord, PackageRecord, SymbolMarkers, SymbolRecord};
use crate::walk;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One re-extracted web file: symbols, candidates, and the updated context.
type ReextractedFile = (Vec<SymbolRecord>, Vec<RelationCandidate>, FileContextRecord);

/// Mutable extraction outputs threaded through a package's files.
struct TargetExtractionOutputs<'a> {
    identity: &'a RepositoryIdentity,
    parser_generation: &'a str,
    symbols: &'a mut Vec<SymbolRecord>,
    candidates: &'a mut Vec<RelationCandidate>,
    file_contexts: &'a mut BTreeMap<String, FileContextRecord>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TypeScriptBackend;

impl TypeScriptBackend {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl LanguageBackend for TypeScriptBackend {
    fn kind(&self) -> LanguageKind {
        LanguageKind::TypeScript
    }

    fn detect(&self, root: &Path, excluded_patterns: &[String]) -> Result<bool, CodeIntelError> {
        Ok(!walk::discover_manifests(root, excluded_patterns, &TS_MANIFEST_NAMES)?.is_empty())
    }

    fn discover_packages(
        &self,
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<BackendDiscovery, CodeIntelError> {
        discover_web_packages(root, identity, parser_generation, excluded_patterns)
    }

    fn identity_inputs(&self) -> &'static [&'static str] {
        &TS_IDENTITY_INPUTS
    }

    fn source_extensions(&self) -> &'static [&'static str] {
        &TS_SOURCE_EXTENSIONS
    }

    fn collect_source_files(
        &self,
        root: &Path,
        excluded_patterns: &[String],
    ) -> Result<BTreeSet<String>, CodeIntelError> {
        let mut files = BTreeSet::new();
        walk::collect_source_paths(
            root,
            root,
            &mut files,
            excluded_patterns,
            &TS_SOURCE_EXTENSIONS,
        )?;
        Ok(files)
    }

    fn extract(
        &self,
        packages: &[PackageRecord],
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<SymbolExtraction, CodeIntelError> {
        extract_web_packages(
            packages,
            root,
            identity,
            parser_generation,
            excluded_patterns,
        )
    }

    fn reextract_file(
        &self,
        root: &Path,
        relative_path: &str,
        record: &FileContextRecord,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        _excluded_patterns: &[String],
    ) -> Result<Option<ReextractedFile>, CodeIntelError> {
        reextract_web_file(root, relative_path, record, identity, parser_generation)
    }

    fn derive_subtree_contexts(
        &self,
        _root: &Path,
        file: &Path,
        record: &FileContextRecord,
        _excluded_patterns: &[String],
    ) -> Result<Vec<(PathBuf, DerivedFileContext)>, CodeIntelError> {
        // Web has no module tree: only the file itself is re-derived, with
        // its recorded context unchanged (mirrors the Python backend).
        let canonical = file.canonicalize().map_err(|error| CodeIntelError::Io {
            operation: "canonicalize dirty web source".to_string(),
            path: file.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        Ok(vec![(
            canonical,
            DerivedFileContext {
                stack: record.stack.clone(),
                is_test: record.is_test,
                is_bench: record.is_bench,
                parent: record.parent.clone(),
            },
        )])
    }

    fn is_new_auto_target_root(
        &self,
        relative_path: &str,
        package_roots: &BTreeSet<String>,
    ) -> bool {
        // A new web source is an auto-discovery target exactly when the
        // extractor would pick it up on a full rebuild: under a package
        // root's `src/` or under a package root's `tests`/`e2e`/`benchmarks`
        // directories. Anything else needs a manifest change, so the new-file
        // check conservatively does not force a full rebuild for it.
        let path = Path::new(relative_path);
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            return false;
        };
        if !TS_SOURCE_EXTENSIONS.contains(&extension) {
            return false;
        }
        let Some(parent) = path.parent() else {
            return false;
        };
        let parent = parent.to_string_lossy().into_owned();
        for root_dir in package_roots {
            let base = if root_dir.is_empty() {
                String::new()
            } else {
                format!("{root_dir}/")
            };
            for directory in ["src", "tests", "e2e", "benchmarks"] {
                let root_path = format!("{base}{directory}");
                if parent == root_path || parent.starts_with(&format!("{root_path}/")) {
                    return true;
                }
            }
        }
        false
    }
}

/// Full extraction pass: walk each package's targets and extract every web
/// source file once (deduplicated by canonical path across targets).
fn extract_web_packages(
    packages: &[PackageRecord],
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<SymbolExtraction, CodeIntelError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| CodeIntelError::Identity {
            context: "canonicalize repository root for web extraction".to_string(),
            details: error.to_string(),
        })?;
    let mut symbols = Vec::new();
    let mut candidates = Vec::new();
    let mut file_contexts = BTreeMap::new();
    let mut seen_files = BTreeSet::new();
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
                    operation: "canonicalize web target source".to_string(),
                    path: target_root.to_string_lossy().into_owned(),
                    details: error.to_string(),
                })?;
            if !target_root.starts_with(&canonical_root) {
                return Err(CodeIntelError::Identity {
                    context: "validate web target source scope".to_string(),
                    details: format!(
                        "target {} points outside repository root: {}",
                        target.name,
                        target_root.display()
                    ),
                });
            }
            let mut files = Vec::new();
            collect_web_files(&target_root, &mut files, excluded_patterns)?;
            for file in files {
                let file = file.canonicalize().map_err(|error| CodeIntelError::Io {
                    operation: "canonicalize web source".to_string(),
                    path: file.to_string_lossy().into_owned(),
                    details: error.to_string(),
                })?;
                if !file.starts_with(&canonical_root) || !seen_files.insert(file.clone()) {
                    continue;
                }
                let mut outputs = TargetExtractionOutputs {
                    identity,
                    parser_generation,
                    symbols: &mut symbols,
                    candidates: &mut candidates,
                    file_contexts: &mut file_contexts,
                };
                extract_web_target_file(&file, &canonical_root, package, target, &mut outputs)?;
            }
        }
    }
    Ok(SymbolExtraction {
        symbols,
        candidates,
        file_contexts,
    })
}

/// Extract one file under a target and record its per-file context.
fn extract_web_target_file(
    file: &Path,
    canonical_root: &Path,
    package: &PackageRecord,
    target: &crate::types::TargetRecord,
    outputs: &mut TargetExtractionOutputs<'_>,
) -> Result<(), CodeIntelError> {
    let relative_path = file
        .strip_prefix(canonical_root)
        .map_err(|error| CodeIntelError::Identity {
            context: "derive web source provenance path".to_string(),
            details: error.to_string(),
        })?
        .to_string_lossy()
        .into_owned();
    let source_bytes = fs::read(file).map_err(|error| CodeIntelError::Io {
        operation: "read web source file".to_string(),
        path: file.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let source_content_hash = content_hash(&source_bytes);
    let source = String::from_utf8(source_bytes).map_err(|error| CodeIntelError::Parse {
        context: format!("decode web source {}", file.display()),
        details: error.to_string(),
    })?;
    let module_path = module_path_for_file(&relative_path);
    let target_is_test = target.kind.iter().any(|kind| kind == "web-test");
    let target_is_bench = target.kind.iter().any(|kind| kind == "web-bench");
    let file_is_test = target_is_test || is_test_file(&relative_path);
    let file_is_bench = target_is_bench || is_bench_file(&relative_path);
    let file_context = FileContext {
        package: &package.name,
        target: &target.name,
        relative_path: relative_path.clone(),
        content_hash: source_content_hash,
        identity: outputs.identity,
        parser_generation: outputs.parser_generation,
        file_markers: SymbolMarkers::default(),
        is_test_target: file_is_test,
        is_bench_target: file_is_bench,
    };
    let extraction = extract_web_file(&source, &relative_path, &file_context)?;
    outputs.file_contexts.insert(
        relative_path.clone(),
        FileContextRecord {
            package: package.name.clone(),
            target: target.name.clone(),
            is_test_target: target_is_test,
            is_bench_target: target_is_bench,
            stack: module_path.split('/').map(str::to_string).collect(),
            is_test: file_is_test,
            is_bench: file_is_bench,
            parent: None,
        },
    );
    outputs.symbols.extend(extraction.symbols);
    outputs.candidates.extend(extraction.candidates);
    Ok(())
}

/// Per-file re-extraction for the incremental rebuild, using the recorded
/// package/target and the file's own module path.
fn reextract_web_file(
    root: &Path,
    relative_path: &str,
    record: &FileContextRecord,
    identity: &RepositoryIdentity,
    parser_generation: &str,
) -> Result<Option<ReextractedFile>, CodeIntelError> {
    let file = root.join(relative_path);
    let source_bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CodeIntelError::Io {
                operation: "read web source file".to_string(),
                path: file.to_string_lossy().into_owned(),
                details: error.to_string(),
            });
        }
    };
    let source_content_hash = content_hash(&source_bytes);
    let source = String::from_utf8(source_bytes).map_err(|error| CodeIntelError::Parse {
        context: format!("decode web source {}", file.display()),
        details: error.to_string(),
    })?;
    let module_path = module_path_for_file(relative_path);
    let file_is_test = record.is_test_target || is_test_file(relative_path);
    let file_is_bench = record.is_bench_target || is_bench_file(relative_path);
    let file_context = FileContext {
        package: &record.package,
        target: &record.target,
        relative_path: relative_path.to_string(),
        content_hash: source_content_hash,
        identity,
        parser_generation,
        file_markers: SymbolMarkers::default(),
        is_test_target: file_is_test,
        is_bench_target: file_is_bench,
    };
    let extraction = extract_web_file(&source, relative_path, &file_context)?;
    let updated = FileContextRecord {
        package: record.package.clone(),
        target: record.target.clone(),
        is_test_target: record.is_test_target,
        is_bench_target: record.is_bench_target,
        stack: module_path.split('/').map(str::to_string).collect(),
        is_test: file_is_test,
        is_bench: file_is_bench,
        parent: None,
    };
    Ok(Some((extraction.symbols, extraction.candidates, updated)))
}

/// Every web source file under `directory` (recursively), skipping excluded,
/// hidden, and `.git` paths. Symlinks are never followed.
fn collect_web_files(
    directory: &Path,
    out: &mut Vec<PathBuf>,
    excluded_patterns: &[String],
) -> Result<(), CodeIntelError> {
    if walk::is_excluded_path(directory, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| CodeIntelError::Io {
        operation: "inspect web source path".to_string(),
        path: directory.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        let extension = directory
            .extension()
            .and_then(|ext| ext.to_str())
            .map_or("", |ext| ext);
        if TS_SOURCE_EXTENSIONS.contains(&extension) {
            out.push(directory.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| CodeIntelError::Io {
        operation: "read web source directory".to_string(),
        path: directory.to_string_lossy().into_owned(),
        details: error.to_string(),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Io {
            operation: "read web source directory entry".to_string(),
            path: directory.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        let name = child
            .file_name()
            .and_then(|name| name.to_str())
            .map_or("", |name| name);
        if name == ".git" || name.starts_with('.') {
            continue;
        }
        collect_web_files(&child, out, excluded_patterns)?;
    }
    Ok(())
}
