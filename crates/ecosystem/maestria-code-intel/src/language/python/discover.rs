//! Bounded, offline Python distribution discovery.
//!
//! Reads `pyproject.toml` (PEP 621 `[project]`), `setup.cfg`
//! (`[metadata]`), and `setup.py` (`name=`/`version=` keyword regex) —
//! never executing anything and never installing. A distribution's targets
//! are the top-level packages and modules its layout contains, discovered by
//! a deterministic directory walk.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::language::BackendDiscovery;
use crate::language::python::manifest::{
    manifest_provenance, read_dependencies, read_distribution_identity,
};
use crate::types::{DependencyRecord, PackageRecord, RecordProvenance, TargetRecord};
use crate::walk;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const PYTHON_MANIFEST_NAMES: [&str; 3] = ["pyproject.toml", "setup.cfg", "setup.py"];
const PYTHON_SOURCE_EXTENSIONS: [&str; 1] = ["py"];

/// Discover every Python distribution under `root`: one `PackageRecord` per
/// manifest, with targets = the top-level packages/modules the distribution's
/// layout contains. A repo without any Python manifest yields an empty
/// discovery (the backend's `detect` would have returned false). A broken
/// ROOT manifest is a hard typed error; a broken nested manifest degrades
/// with a warning.
pub(crate) fn discover_python_packages(
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<BackendDiscovery, CodeIntelError> {
    let manifests = walk::discover_manifests(root, excluded_patterns, &PYTHON_MANIFEST_NAMES)?;
    let mut packages = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_package_ids = BTreeSet::new();
    let mut seen_target_paths = BTreeSet::new();
    for manifest in &manifests {
        let relative_manifest = match manifest.strip_prefix(root) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => manifest.to_string_lossy().into_owned(),
        };
        let is_root = manifest.parent() == Some(root);
        let distribution = match read_distribution_identity(manifest) {
            Ok(distribution) => distribution,
            Err(error) => {
                if is_root {
                    return Err(error);
                }
                warnings.push(format!(
                    "python distribution at {relative_manifest}: {error}"
                ));
                continue;
            }
        };
        let package_id = format!("python-dist:{relative_manifest}");
        if !seen_package_ids.insert(package_id.clone()) {
            continue;
        }
        let provenance = manifest_provenance(manifest, identity, parser_generation)?;
        let manifest_dir = match manifest.parent() {
            Some(dir) => dir,
            None => root,
        };
        let mut targets = Vec::new();
        collect_distribution_targets(
            root,
            manifest_dir,
            excluded_patterns,
            &mut targets,
            &mut seen_target_paths,
            &provenance,
        )?;
        targets.sort_by(|left, right| left.src_path.cmp(&right.src_path));
        let dependencies = match read_dependencies(manifest) {
            Ok(dependencies) => dependencies,
            Err(error) => {
                if is_root {
                    return Err(error);
                }
                warnings.push(format!(
                    "python distribution at {relative_manifest}: {error}"
                ));
                Vec::new()
            }
        };
        let dependency_records = dependencies
            .into_iter()
            .map(|dependency| DependencyRecord {
                name: dependency.name,
                package: None,
                source: None,
                version_req: dependency.version_req,
                kind: Vec::new(),
                optional: false,
                uses_default_features: false,
                features: Vec::new(),
                target: None,
                registry: None,
                provenance: provenance.clone(),
            })
            .collect();
        packages.push(PackageRecord {
            package_id,
            name: distribution.name,
            version: distribution.version,
            manifest_path: relative_manifest,
            edition: None,
            authors: Vec::new(),
            source: None,
            description: None,
            features: BTreeMap::new(),
            dependencies: dependency_records,
            targets,
            provenance,
        });
    }
    packages.sort_by(|left, right| left.package_id.cmp(&right.package_id));
    Ok(BackendDiscovery { packages, warnings })
}

/// Collect the targets of one distribution: one `py-module` target per
/// top-level package (an `__init__.py` directory whose parent is not a
/// package) and per top-level module file, plus `py-test`/`py-bench` targets
/// for test/benchmark roots directly under the manifest directory.
fn collect_distribution_targets(
    root: &Path,
    manifest_dir: &Path,
    excluded_patterns: &[String],
    targets: &mut Vec<TargetRecord>,
    seen_target_paths: &mut BTreeSet<String>,
    provenance: &RecordProvenance,
) -> Result<(), CodeIntelError> {
    let mut package_dirs: Vec<PathBuf> = Vec::new();
    collect_package_dirs(root, manifest_dir, excluded_patterns, &mut package_dirs)?;
    let mut module_files: Vec<PathBuf> = Vec::new();
    collect_top_level_modules(manifest_dir, excluded_patterns, &mut module_files)?;

    for dir in package_dirs {
        let relative = match dir.strip_prefix(root) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => dir.to_string_lossy().into_owned(),
        };
        let name = match dir.file_name().and_then(|name| name.to_str()) {
            Some(name) => name.to_string(),
            None => "python-package".to_string(),
        };
        push_target(
            targets,
            seen_target_paths,
            name,
            "py-module",
            relative,
            provenance,
        );
    }
    for file in module_files {
        let relative = match file.strip_prefix(root) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => file.to_string_lossy().into_owned(),
        };
        let name = match file.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) => stem.to_string(),
            None => "python-module".to_string(),
        };
        push_target(
            targets,
            seen_target_paths,
            name,
            "py-module",
            relative,
            provenance,
        );
    }
    collect_test_and_bench_targets(root, manifest_dir, targets, seen_target_paths, provenance);
    // Root-level test modules become individual py-test targets.
    if let Ok(entries) = fs::read_dir(manifest_dir) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("py")
            })
            .collect();
        files.sort();
        for file in files {
            let file_name = file
                .file_name()
                .and_then(|name| name.to_str())
                .map_or("", |name| name);
            if !(file_name.starts_with("test_") || file_name.ends_with("_test.py")) {
                continue;
            }
            let relative = match file.strip_prefix(root) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(_) => file.to_string_lossy().into_owned(),
            };
            let name = match file.file_stem().and_then(|stem| stem.to_str()) {
                Some(stem) => stem.to_string(),
                None => "python-test".to_string(),
            };
            push_target(
                targets,
                seen_target_paths,
                name,
                "py-test",
                relative,
                provenance,
            );
        }
    }
    Ok(())
}

fn push_target(
    targets: &mut Vec<TargetRecord>,
    seen: &mut BTreeSet<String>,
    name: String,
    kind: &'static str,
    src_path: String,
    provenance: &RecordProvenance,
) {
    if !seen.insert(src_path.clone()) {
        return;
    }
    targets.push(TargetRecord {
        name,
        kind: vec![kind.to_string()],
        crate_types: Vec::new(),
        src_path,
        required_features: Vec::new(),
        doctest: false,
        test: false,
        bench: false,
        doc: false,
        provenance: provenance.clone(),
    });
}

/// `py-test`/`py-bench` targets for test/benchmark roots directly under the
/// manifest directory.
fn collect_test_and_bench_targets(
    root: &Path,
    manifest_dir: &Path,
    targets: &mut Vec<TargetRecord>,
    seen_target_paths: &mut BTreeSet<String>,
    provenance: &RecordProvenance,
) {
    for directory in ["tests", "test"] {
        let dir = manifest_dir.join(directory);
        if dir.is_dir() {
            let relative = match dir.strip_prefix(root) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(_) => dir.to_string_lossy().into_owned(),
            };
            push_target(
                targets,
                seen_target_paths,
                directory.to_string(),
                "py-test",
                relative,
                provenance,
            );
        }
    }
    for directory in ["benchmarks", "bench"] {
        let dir = manifest_dir.join(directory);
        if dir.is_dir() {
            let relative = match dir.strip_prefix(root) {
                Ok(path) => path.to_string_lossy().into_owned(),
                Err(_) => dir.to_string_lossy().into_owned(),
            };
            push_target(
                targets,
                seen_target_paths,
                directory.to_string(),
                "py-bench",
                relative,
                provenance,
            );
        }
    }
}

/// Every top-level package directory under `base`: an `__init__.py`
/// directory whose parent is not itself a package. Test/benchmark subtrees,
/// hidden directories, and excluded paths are skipped.
fn collect_package_dirs(
    root: &Path,
    base: &Path,
    excluded_patterns: &[String],
    out: &mut Vec<PathBuf>,
) -> Result<(), CodeIntelError> {
    if walk::is_excluded_path(base, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(base).map_err(|error| CodeIntelError::Io {
        operation: "inspect python package directory".to_string(),
        path: base.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    if !metadata.is_dir() {
        return Ok(());
    }
    // The manifest directory itself is never a package target (its walk is
    // the distribution root, not an importable package).
    if base != root && is_package_dir(base) && !parent_is_package(base) {
        out.push(base.to_path_buf());
        return Ok(());
    }
    for entry in fs::read_dir(base).map_err(|error| CodeIntelError::Io {
        operation: "read python package directory".to_string(),
        path: base.to_string_lossy().into_owned(),
        details: error.to_string(),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Io {
            operation: "read python package directory entry".to_string(),
            path: base.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        let name = child
            .file_name()
            .and_then(|name| name.to_str())
            .map_or("", |name| name);
        if name.starts_with('.')
            || name == "__pycache__"
            || name.ends_with(".egg-info")
            || matches!(name, "tests" | "test" | "benchmarks" | "bench")
        {
            continue;
        }
        collect_package_dirs(root, &child, excluded_patterns, out)?;
    }
    Ok(())
}

fn parent_is_package(dir: &Path) -> bool {
    dir.parent().is_some_and(is_package_dir)
}

pub(crate) fn is_package_dir(dir: &Path) -> bool {
    dir.join("__init__.py").is_file()
}

/// Top-level module files directly under `base`: non-test `.py` files whose
/// parent is the manifest directory itself.
fn collect_top_level_modules(
    base: &Path,
    excluded_patterns: &[String],
    out: &mut Vec<PathBuf>,
) -> Result<(), CodeIntelError> {
    if walk::is_excluded_path(base, excluded_patterns) {
        return Ok(());
    }
    let entries = fs::read_dir(base).map_err(|error| CodeIntelError::Io {
        operation: "read python module directory".to_string(),
        path: base.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| CodeIntelError::Io {
            operation: "read python module directory entry".to_string(),
            path: base.to_string_lossy().into_owned(),
            details: error.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or("", |name| name);
        if file_name.starts_with('.') || file_name == "__init__.py" {
            continue;
        }
        if PYTHON_MANIFEST_NAMES.contains(&file_name) {
            continue;
        }
        if file_name.starts_with("test_") || file_name.ends_with("_test.py") {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("py") {
            files.push(path);
        }
    }
    files.sort();
    out.extend(files);
    Ok(())
}

pub(crate) fn collect_python_source_files(
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
        &PYTHON_SOURCE_EXTENSIONS,
    )?;
    Ok(files)
}
