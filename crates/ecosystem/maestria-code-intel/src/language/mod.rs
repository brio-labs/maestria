//! Language extraction backend boundary.
//!
//! `RepositoryCodeIndex` construction and incremental rebuilds dispatch
//! manifest discovery, source walking, and symbol extraction through
//! [`LanguageBackend`] implementations. Rust is the first backend; Python
//! the second; TypeScript/JavaScript the third (web packages via
//! `package.json`, tokenizer-based TS/JS extraction). Adding a language is
//! purely additive: a new `LanguageKind` variant, a new backend module, and
//! one entry in [`active_backends`].
//!
//! The incremental core stays language-agnostic: the orchestrator
//! (`incremental/`) consumes only per-file records and per-backend
//! re-extraction and subtree derivation, so the rebuild state machine,
//! sidecar, and equivalence invariants are unchanged.
//!
//! Cross-backend composition (discovery, extraction merging, relation
//! resolution) lives in the `compose` submodule.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::symbols::RelationCandidate;
use crate::symbols::SymbolExtraction;
use crate::types::{FileContextRecord, PackageRecord, SymbolRecord};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One re-extracted file: symbols, candidates, and the updated context.
type ReextractedFile = (Vec<SymbolRecord>, Vec<RelationCandidate>, FileContextRecord);

pub(crate) mod compose;
pub(crate) mod python;
pub(crate) mod rust;
pub(crate) mod typescript;

/// Languages with an extraction backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LanguageKind {
    Rust,
    Python,
    TypeScript,
}

/// Re-derived per-file context for incremental subtree derivation.
///
/// The Rust backend re-derives `mod` trees and records the module stack,
/// test/bench flags, and declaring-file parent for every reachable file; the
/// Python and TypeScript backends have no module tree and return only the
/// file itself with its recorded context. `parent` is the relative
/// repository path of the file that declared this file, `None` for target
/// roots.
#[derive(Debug, Clone)]
pub(crate) struct DerivedFileContext {
    pub(crate) stack: Vec<String>,
    pub(crate) is_test: bool,
    pub(crate) is_bench: bool,
    pub(crate) parent: Option<String>,
}

/// Discovery output for one backend: packages plus per-workspace degradation
/// warnings (a nested manifest that failed to parse is skipped and warned,
/// mirroring the Cargo `workspace_warnings` mechanism).
#[derive(Debug, Default)]
pub(crate) struct BackendDiscovery {
    pub(crate) packages: Vec<PackageRecord>,
    pub(crate) warnings: Vec<String>,
}

/// Language backend boundary consumed by the builder and the incremental
/// orchestrator. Implementations are deterministic and never touch the
/// network; discovery never installs or executes anything.
pub(crate) trait LanguageBackend {
    fn kind(&self) -> LanguageKind;

    /// Whether this repository (root or any non-excluded directory) is a
    /// repository for this language. Manifest existence only — no heavy work.
    fn detect(&self, root: &Path, excluded_patterns: &[String]) -> Result<bool, CodeIntelError>;

    /// Discover every distribution (package) this backend owns under `root`,
    /// with per-manifest degradation warnings.
    fn discover_packages(
        &self,
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<BackendDiscovery, CodeIntelError>;

    /// Manifest file NAMES this language contributes to the worktree identity
    /// (matched the same way `is_identity_input` matches them today).
    fn identity_inputs(&self) -> &'static [&'static str];

    /// Source file extensions this backend extracts.
    fn source_extensions(&self) -> &'static [&'static str];

    /// Every source file under `root` this backend would extract, as
    /// relative paths (exclusion-aware; used by identity and the new-file
    /// check).
    fn collect_source_files(
        &self,
        root: &Path,
        excluded_patterns: &[String],
    ) -> Result<BTreeSet<String>, CodeIntelError>;

    /// Full extraction pass for this backend's packages.
    fn extract(
        &self,
        packages: &[PackageRecord],
        root: &Path,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<SymbolExtraction, CodeIntelError>;

    /// Per-file re-extraction for incremental reconcile. `None` means the
    /// file is no longer extractable and the caller drops it.
    fn reextract_file(
        &self,
        root: &Path,
        relative_path: &str,
        record: &FileContextRecord,
        identity: &RepositoryIdentity,
        parser_generation: &str,
        excluded_patterns: &[String],
    ) -> Result<Option<ReextractedFile>, CodeIntelError>;

    /// Re-derive the files whose extraction inputs change when `file`
    /// changes, with their derived contexts. The returned paths are absolute
    /// canonical paths; the first entry is `file` itself.
    fn derive_subtree_contexts(
        &self,
        root: &Path,
        file: &Path,
        record: &FileContextRecord,
        excluded_patterns: &[String],
    ) -> Result<Vec<(PathBuf, DerivedFileContext)>, CodeIntelError>;

    /// Whether a not-yet-indexed source path is a plausible new auto-discovered
    /// target (extractable without any manifest change). Used by the
    /// new-file check to decide a full rebuild.
    fn is_new_auto_target_root(
        &self,
        relative_path: &str,
        package_roots: &BTreeSet<String>,
    ) -> bool;

    /// Whether deleting `relative_path` invalidates persisted package
    /// records (so the rebuild must be full, like a manifest change).
    /// Default false: deleting a Rust module file keeps package records; a
    /// Python top-level package init removes a discovered target.
    fn deleted_file_requires_full(&self, root: &Path, relative_path: &str) -> bool {
        let _ = (root, relative_path);
        false
    }

    /// Whether `package` was discovered by this backend (manifest-name based).
    fn owns_package(&self, package: &PackageRecord) -> bool {
        let name = Path::new(&package.manifest_path)
            .file_name()
            .and_then(|name| name.to_str());
        name.is_some_and(|name| self.identity_inputs().contains(&name))
    }
}

/// Every backend whose `detect()` succeeds, in deterministic order (Rust,
/// then Python, then TypeScript — enforced by the `LanguageKind` ordering).
pub(crate) fn active_backends(
    root: &Path,
    excluded_patterns: &[String],
) -> Result<Vec<Box<dyn LanguageBackend>>, CodeIntelError> {
    let candidates: [Box<dyn LanguageBackend>; 3] = [
        Box::new(rust::RustBackend::new()),
        Box::new(python::PythonBackend::new()),
        Box::new(typescript::TypeScriptBackend::new()),
    ];
    let mut active = Vec::new();
    for backend in candidates {
        if backend.detect(root, excluded_patterns)? {
            active.push(backend);
        }
    }
    active.sort_by_key(|backend| backend.kind());
    Ok(active)
}

/// The backend owning a relative source path, by source extension.
pub(crate) fn backend_for_path<'a>(
    backends: &'a [Box<dyn LanguageBackend>],
    relative_path: &str,
) -> Option<&'a dyn LanguageBackend> {
    let extension = Path::new(relative_path)
        .extension()
        .and_then(|ext| ext.to_str());
    backends
        .iter()
        .find(|backend| {
            backend
                .source_extensions()
                .iter()
                .any(|accepted| extension == Some(*accepted))
        })
        .map(|backend| backend.as_ref())
}

/// Whether a repository-relative path is a backend manifest file name
/// (editing it invalidates discovery and forces a full rebuild).
pub(crate) fn is_backend_manifest_path(path: &str, backends: &[Box<dyn LanguageBackend>]) -> bool {
    let name = Path::new(path).file_name().and_then(|name| name.to_str());
    name.is_some_and(|name| {
        backends
            .iter()
            .any(|backend| backend.identity_inputs().contains(&name))
    })
}

/// Source extensions of every known language backend. Worktree identity and
/// source walks always cover these, even when no manifest activates the
/// backend (a repository with `.rs` sources but no `Cargo.toml` still has
/// those sources participate in the identity digest).
pub(crate) const KNOWN_SOURCE_EXTENSIONS: [&str; 8] =
    ["rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// Union of every active backend's manifest file names (excluding lock-file
/// siblings, which are derived from manifests, not discovered).
pub(crate) fn all_manifest_names(backends: &[Box<dyn LanguageBackend>]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for backend in backends {
        names.extend(
            backend
                .identity_inputs()
                .iter()
                .copied()
                .filter(|name| !name.ends_with(".lock")),
        );
    }
    names
}
