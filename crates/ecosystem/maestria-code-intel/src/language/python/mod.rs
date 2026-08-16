//! Python language backend: bounded manifest discovery, walk-based source
//! collection, and tokenizer-based symbol extraction.

pub(crate) mod discover;
pub(crate) mod extract;
pub(crate) mod manifest;
pub(crate) mod statements;
pub(crate) mod tokens;

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::language::python::discover::{
    PYTHON_MANIFEST_NAMES, collect_python_source_files, discover_python_packages, is_package_dir,
};
use crate::language::python::extract::extract_python_file;
use crate::language::python::tokens::{is_bench_file, is_test_file, module_path_for_file};
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

const PYTHON_SOURCE_EXTENSIONS: [&str; 1] = ["py"];

/// One re-extracted Python file: symbols, candidates, and the updated context.
type ReextractedFile = (Vec<SymbolRecord>, Vec<RelationCandidate>, FileContextRecord);

/// Mutable extraction outputs threaded through a target's files.
struct TargetExtractionOutputs<'a> {
    identity: &'a RepositoryIdentity,
    parser_generation: &'a str,
    symbols: &'a mut Vec<SymbolRecord>,
    candidates: &'a mut Vec<RelationCandidate>,
    file_contexts: &'a mut BTreeMap<String, FileContextRecord>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PythonBackend;

impl PythonBackend {
    pub(crate) const fn new() -> Self {
        Self
    }
}

impl LanguageBackend for PythonBackend {
    fn kind(&self) -> LanguageKind {
        LanguageKind::Python
    }

    fn detect(&self, root: &Path, excluded_patterns: &[String]) -> Result<bool, CodeIntelError> {
        Ok(!walk::discover_manifests(root, excluded_patterns, &PYTHON_MANIFEST_NAMES)?.is_empty())
    }

    fn discover_packages(
        &self,
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<BackendDiscovery, CodeIntelError> {
        discover_python_packages(root, identity, parser_generation, excluded_patterns)
    }

    fn identity_inputs(&self) -> &'static [&'static str] {
        &PYTHON_MANIFEST_NAMES
    }

    fn source_extensions(&self) -> &'static [&'static str] {
        &PYTHON_SOURCE_EXTENSIONS
    }

    fn collect_source_files(
        &self,
        root: &Path,
        excluded_patterns: &[String],
        selection: Option<&crate::selection::RepositorySelection>,
    ) -> Result<BTreeSet<String>, CodeIntelError> {
        collect_python_source_files(root, excluded_patterns, selection)
    }

    fn extract(
        &self,
        packages: &[PackageRecord],
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<SymbolExtraction, CodeIntelError> {
        extract_python_packages(
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
    ) -> Result<
        Option<(Vec<SymbolRecord>, Vec<RelationCandidate>, FileContextRecord)>,
        CodeIntelError,
    > {
        reextract_python_file(root, relative_path, record, identity, parser_generation)
    }

    fn derive_subtree_contexts(
        &self,
        _root: &Path,
        file: &Path,
        record: &FileContextRecord,
        _excluded_patterns: &[String],
    ) -> Result<Vec<(PathBuf, DerivedFileContext)>, CodeIntelError> {
        // Python has no module tree: only the file itself is re-derived, with
        // its recorded context unchanged. Package-root shifts (which would
        // re-path siblings) always coincide with target-set changes and are
        // handled by the full-rebuild trigger on init deletions.
        let canonical = file.canonicalize().map_err(|error| CodeIntelError::Io {
            operation: "canonicalize dirty Python source".to_string(),
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

    fn deleted_file_requires_full(&self, root: &Path, relative_path: &str) -> bool {
        // Deleting a TOP-LEVEL package init removes a discovered target (and
        // can shift the package root), so the persisted package records are
        // stale until a full rebuild — exactly like a manifest change. A
        // nested init deletion leaves targets and module paths unchanged and
        // reconciles incrementally.
        if !relative_path.ends_with("__init__.py") {
            return false;
        }
        let path = Path::new(relative_path);
        let Some(package_dir) = path.parent() else {
            return false;
        };
        let Some(parent_of_package) = package_dir.parent() else {
            return false;
        };
        !is_package_dir(&root.join(parent_of_package))
    }

    fn is_new_auto_target_root(
        &self,
        relative_path: &str,
        _package_roots: &BTreeSet<String>,
    ) -> bool {
        // Python extraction walks whole package/test/benchmark trees, so any
        // new `.py` file is extractable without a manifest change; only a
        // full rebuild picks it up, so every new `.py` forces Full.
        Path::new(relative_path)
            .extension()
            .and_then(|ext| ext.to_str())
            == Some("py")
    }
}

/// Full extraction pass: walk each target's source tree and extract every
/// `.py` file once (deduplicated by canonical path across targets).
fn extract_python_packages(
    packages: &[PackageRecord],
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<SymbolExtraction, CodeIntelError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|error| CodeIntelError::Identity {
            context: "canonicalize repository root for python extraction".to_string(),
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
                    operation: "canonicalize python target source".to_string(),
                    path: target_root.to_string_lossy().into_owned(),
                    details: error.to_string(),
                })?;
            if !target_root.starts_with(&canonical_root) {
                return Err(CodeIntelError::Identity {
                    context: "validate python target source scope".to_string(),
                    details: format!(
                        "target {} points outside repository root: {}",
                        target.name,
                        target_root.display()
                    ),
                });
            }
            let mut files = Vec::new();
            collect_python_files(&target_root, &mut files, excluded_patterns)?;
            for file in files {
                let file = file.canonicalize().map_err(|error| CodeIntelError::Io {
                    operation: "canonicalize python source".to_string(),
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
                extract_python_target_file(&file, &canonical_root, package, target, &mut outputs)?;
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
fn extract_python_target_file(
    file: &Path,
    canonical_root: &Path,
    package: &PackageRecord,
    target: &crate::types::TargetRecord,
    outputs: &mut TargetExtractionOutputs<'_>,
) -> Result<(), CodeIntelError> {
    let relative_path = file
        .strip_prefix(canonical_root)
        .map_err(|error| CodeIntelError::Identity {
            context: "derive python source provenance path".to_string(),
            details: error.to_string(),
        })?
        .to_string_lossy()
        .into_owned();
    let source_bytes = fs::read(file).map_err(|error| CodeIntelError::Io {
        operation: "read python source file".to_string(),
        path: file.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let source_content_hash = content_hash(&source_bytes);
    let source = String::from_utf8(source_bytes).map_err(|error| CodeIntelError::Parse {
        context: format!("decode python source {}", file.display()),
        details: error.to_string(),
    })?;
    let module_path = module_path_for_file(canonical_root, &relative_path);
    let target_is_test = target.kind.iter().any(|kind| kind == "py-test");
    let target_is_bench = target.kind.iter().any(|kind| kind == "py-bench");
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
    let extraction = extract_python_file(&source, &relative_path, &module_path, &file_context)?;
    outputs.file_contexts.insert(
        relative_path.clone(),
        FileContextRecord {
            package: package.name.clone(),
            target: target.name.clone(),
            is_test_target: target_is_test,
            is_bench_target: target_is_bench,
            stack: module_path.split('.').map(str::to_string).collect(),
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
fn reextract_python_file(
    root: &Path,
    relative_path: &str,
    record: &FileContextRecord,
    identity: &RepositoryIdentity,
    parser_generation: &str,
) -> Result<Option<ReextractedFile>, CodeIntelError> {
    if !relative_path.ends_with(".py") {
        return Ok(None);
    }
    let file = root.join(relative_path);
    let source_bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(CodeIntelError::Io {
                operation: "read python source file".to_string(),
                path: file.to_string_lossy().into_owned(),
                details: error.to_string(),
            });
        }
    };
    let source_content_hash = content_hash(&source_bytes);
    let source = String::from_utf8(source_bytes).map_err(|error| CodeIntelError::Parse {
        context: format!("decode python source {}", file.display()),
        details: error.to_string(),
    })?;
    let module_path = module_path_for_file(root, relative_path);
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
    let extraction = extract_python_file(&source, relative_path, &module_path, &file_context)?;
    let updated = FileContextRecord {
        package: record.package.clone(),
        target: record.target.clone(),
        is_test_target: record.is_test_target,
        is_bench_target: record.is_bench_target,
        stack: module_path.split('.').map(str::to_string).collect(),
        is_test: file_is_test,
        is_bench: file_is_bench,
        parent: None,
    };
    Ok(Some((extraction.symbols, extraction.candidates, updated)))
}

/// Every `.py` file under `directory` (recursively), skipping excluded,
/// hidden, `__pycache__`, and `*.egg-info` paths. Symlinks are never
/// followed.
fn collect_python_files(
    directory: &Path,
    out: &mut Vec<PathBuf>,
    excluded_patterns: &[String],
) -> Result<(), CodeIntelError> {
    if walk::is_excluded_path(directory, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| CodeIntelError::Io {
        operation: "inspect python source path".to_string(),
        path: directory.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_file() {
        if directory.extension().and_then(|ext| ext.to_str()) == Some("py") {
            out.push(directory.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| CodeIntelError::Io {
        operation: "read python source directory".to_string(),
        path: directory.to_string_lossy().into_owned(),
        details: error.to_string(),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Io {
            operation: "read python source directory entry".to_string(),
            path: directory.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        let name = child
            .file_name()
            .and_then(|name| name.to_str())
            .map_or("", |name| name);
        if name == ".git"
            || name.starts_with('.')
            || name == "__pycache__"
            || name.ends_with(".egg-info")
        {
            continue;
        }
        collect_python_files(&child, out, excluded_patterns)?;
    }
    Ok(())
}
