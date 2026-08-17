//! Rust language backend: Cargo workspace discovery and syn-based symbol
//! extraction, adapted from the pre-backend-boundary `metadata` and `symbols`
//! modules without changing their behavior.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::language::{BackendDiscovery, DerivedFileContext, LanguageBackend, LanguageKind};
use crate::metadata::extract_workspace_packages;
use crate::symbols::collect_rust::ModuleContext;
use crate::symbols::context::FileContext;
use crate::symbols::{RelationCandidate, SymbolExtraction, extract, markers};
use crate::types::{FileContextRecord, PackageRecord, SymbolRecord};
use crate::walk;
use maestria_domain::content_hash;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const RUST_MANIFEST_NAMES: [&str; 2] = ["Cargo.toml", "Cargo.lock"];
const RUST_SOURCE_EXTENSIONS: [&str; 1] = ["rs"];

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RustBackend;

impl RustBackend {
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Whether any non-excluded directory holds a `Cargo.toml`.
    fn has_manifest(root: &Path, excluded_patterns: &[String]) -> Result<bool, CodeIntelError> {
        Ok(!walk::discover_manifests(root, excluded_patterns, &["Cargo.toml"])?.is_empty())
    }
}

impl LanguageBackend for RustBackend {
    fn kind(&self) -> LanguageKind {
        LanguageKind::Rust
    }

    fn detect(&self, root: &Path, excluded_patterns: &[String]) -> Result<bool, CodeIntelError> {
        Self::has_manifest(root, excluded_patterns)
    }

    fn discover_packages(
        &self,
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<BackendDiscovery, CodeIntelError> {
        let discovery =
            extract_workspace_packages(root, identity, parser_generation, excluded_patterns)?;
        Ok(BackendDiscovery {
            packages: discovery.packages,
            warnings: discovery.warnings,
        })
    }

    fn identity_inputs(&self) -> &'static [&'static str] {
        &RUST_MANIFEST_NAMES
    }

    fn source_extensions(&self) -> &'static [&'static str] {
        &RUST_SOURCE_EXTENSIONS
    }

    fn collect_source_files(
        &self,
        root: &Path,
        excluded_patterns: &[String],
        selection: Option<&crate::selection::RepositorySelection>,
    ) -> Result<BTreeSet<String>, CodeIntelError> {
        let mut files = BTreeSet::new();
        walk::collect_source_paths(
            root,
            root,
            &mut files,
            excluded_patterns,
            selection,
            &RUST_SOURCE_EXTENSIONS,
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
        crate::symbols::extract_symbols(
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
        let file = root.join(relative_path);
        let source_bytes = match fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(CodeIntelError::Io {
                    operation: "read source file".to_string(),
                    path: file.to_string_lossy().into_owned(),
                    details: error.to_string(),
                });
            }
        };
        let source_content_hash = content_hash(&source_bytes);
        let source = String::from_utf8(source_bytes).map_err(|error| CodeIntelError::Parse {
            context: format!("decode Rust source {}", file.display()),
            details: error.to_string(),
        })?;
        let file_context = FileContext {
            package: &record.package,
            target: &record.target,
            relative_path: relative_path.to_string(),
            content_hash: source_content_hash,
            identity,
            parser_generation,
            file_markers: markers::file_markers(&file, &source),
            is_test_target: record.is_test_target || record.is_test,
            is_bench_target: record.is_bench_target || record.is_bench,
        };
        let (symbols, candidates) =
            extract::extract_file_symbols(&source, &file_context, &record.stack)?;
        Ok(Some((symbols, candidates, record.clone())))
    }

    fn derive_subtree_contexts(
        &self,
        root: &Path,
        file: &Path,
        record: &FileContextRecord,
        excluded_patterns: &[String],
    ) -> Result<Vec<(PathBuf, DerivedFileContext)>, CodeIntelError> {
        let mut out = Vec::new();
        let mut derived = std::collections::BTreeMap::new();
        let mut derived_parents = std::collections::BTreeMap::new();
        crate::symbols::derive_subtree_contexts(
            root,
            file,
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
        let canonical = file.canonicalize().map_err(|error| CodeIntelError::Io {
            operation: "canonicalize dirty Rust source".to_string(),
            path: file.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let mut contexts = Vec::new();
        for path in out {
            let context = &derived[&path];
            let parent = derived_parents
                .get(&path)
                .and_then(|parent| {
                    parent
                        .strip_prefix(root)
                        .ok()
                        .map(|relative| relative.to_string_lossy().into_owned())
                })
                .or_else(|| {
                    if path == canonical {
                        record.parent.clone()
                    } else {
                        None
                    }
                });
            contexts.push((
                path,
                DerivedFileContext {
                    stack: context.stack.clone(),
                    is_test: context.is_test,
                    is_bench: context.is_bench,
                    parent,
                },
            ));
        }
        Ok(contexts)
    }

    fn is_new_auto_target_root(
        &self,
        relative_path: &str,
        package_roots: &BTreeSet<String>,
    ) -> bool {
        let path = Path::new(relative_path);
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        if file_name == "mod.rs" {
            // cargo never auto-discovers `mod.rs` as a target.
            return false;
        }
        let Some(parent) = path
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
        else {
            return false;
        };
        let grandparent = path
            .parent()
            .and_then(|parent| parent.parent())
            .map(|parent| parent.to_string_lossy().into_owned());
        for root_dir in package_roots {
            let base = if root_dir.is_empty() {
                String::new()
            } else {
                format!("{root_dir}/")
            };
            if file_name == "build.rs" && parent == *root_dir {
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
                if file_name == "main.rs" && grandparent.as_deref() == Some(multi_root.as_str()) {
                    return true;
                }
            }
        }
        false
    }
}
