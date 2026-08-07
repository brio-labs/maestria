//! Bounded, offline web package discovery.
//!
//! Reads every `package.json` under the repository root with the shared
//! bounded manifest walk (skipping `node_modules`, `.git`, hidden
//! directories, and privacy-excluded paths). One `PackageRecord` per
//! manifest: name/version from the JSON fields, targets = the `src/` walk
//! plus entry points (`main`/`module`/`exports` leaves) plus `tests`/`e2e`/
//! `benchmarks` directory walks. `workspaces` globs are NOT resolved: the
//! bounded walk finds every member manifest anyway, and packages are
//! deduplicated by manifest path. Lockfiles are never parsed.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::language::BackendDiscovery;
use crate::language::typescript::manifest::{
    WebPackageIdentity, manifest_provenance, read_package_identity,
};
use crate::types::{PackageRecord, RecordProvenance, TargetRecord};
use crate::walk;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) const TS_MANIFEST_NAMES: [&str; 1] = ["package.json"];

/// Manifest and lockfile names this backend contributes to the worktree
/// identity (`all_manifest_names` filters the `.lock` suffixes itself).
pub(crate) const TS_IDENTITY_INPUTS: [&str; 4] = [
    "package.json",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
];

/// Discover every web package under `root`: one `PackageRecord` per
/// `package.json`, with targets = the `src/` walk, resolved entry points,
/// and test/benchmark directory walks. A repo without any `package.json`
/// yields an empty discovery (the backend's `detect` would have returned
/// false). A broken ROOT manifest is a hard typed error; a broken nested
/// manifest degrades with a warning.
pub(crate) fn discover_web_packages(
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<BackendDiscovery, CodeIntelError> {
    let manifests = walk::discover_manifests(root, excluded_patterns, &TS_MANIFEST_NAMES)?;
    let mut packages = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_package_ids = BTreeSet::new();
    for manifest in &manifests {
        let relative_manifest = match manifest.strip_prefix(root) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(_) => manifest.to_string_lossy().into_owned(),
        };
        let is_root = manifest.parent() == Some(root);
        let package_identity = match read_package_identity(manifest) {
            Ok(identity) => identity,
            Err(error) => {
                if is_root {
                    return Err(error);
                }
                warnings.push(format!("web package at {relative_manifest}: {error}"));
                continue;
            }
        };
        let package_id = format!("web-pkg:{relative_manifest}");
        if !seen_package_ids.insert(package_id.clone()) {
            continue;
        }
        let provenance = manifest_provenance(manifest, identity, parser_generation)?;
        let manifest_dir = match manifest.parent() {
            Some(dir) => dir,
            None => root,
        };
        let mut targets = Vec::new();
        collect_web_targets(
            root,
            manifest_dir,
            &package_identity,
            excluded_patterns,
            &mut targets,
            &provenance,
        )?;
        targets.sort_by(|left, right| left.src_path.cmp(&right.src_path));
        packages.push(PackageRecord {
            package_id,
            name: package_identity.name,
            version: package_identity.version,
            manifest_path: relative_manifest,
            edition: None,
            authors: Vec::new(),
            source: None,
            description: None,
            features: std::collections::BTreeMap::new(),
            dependencies: Vec::new(),
            targets,
            provenance,
        });
    }
    packages.sort_by(|left, right| left.package_id.cmp(&right.package_id));
    Ok(BackendDiscovery { packages, warnings })
}

/// Collect the targets of one package: the `src/` directory walk, entry
/// points that exist under the repository root, and `tests`/`e2e`/
/// `benchmarks` directory walks. A package with none of these falls back to
/// its own directory so its sources are still indexed.
fn collect_web_targets(
    root: &Path,
    manifest_dir: &Path,
    identity: &WebPackageIdentity,
    excluded_patterns: &[String],
    targets: &mut Vec<TargetRecord>,
    provenance: &RecordProvenance,
) -> Result<(), CodeIntelError> {
    let mut seen_paths = BTreeSet::new();
    let src_dir = manifest_dir.join("src");
    if src_dir.is_dir() && !walk::is_excluded_path(&src_dir, excluded_patterns) {
        push_target(
            targets,
            &mut seen_paths,
            "src".to_string(),
            "web-src",
            relative_path(root, &src_dir)?,
            provenance,
        );
    }
    for (index, entry) in identity.entry_points.iter().enumerate() {
        let entry_path = manifest_dir.join(entry);
        if !entry_path.exists() {
            continue;
        }
        let Ok(relative) = entry_path.strip_prefix(root) else {
            // Entry points outside the repository root are skipped.
            continue;
        };
        if walk::is_excluded_path(&entry_path, excluded_patterns) {
            continue;
        }
        push_target(
            targets,
            &mut seen_paths,
            format!("entry-{index}"),
            "web-src",
            relative.to_string_lossy().into_owned(),
            provenance,
        );
    }
    for directory in ["tests", "e2e"] {
        let dir = manifest_dir.join(directory);
        if dir.is_dir() && !walk::is_excluded_path(&dir, excluded_patterns) {
            push_target(
                targets,
                &mut seen_paths,
                directory.to_string(),
                "web-test",
                relative_path(root, &dir)?,
                provenance,
            );
        }
    }
    let benchmarks = manifest_dir.join("benchmarks");
    if benchmarks.is_dir() && !walk::is_excluded_path(&benchmarks, excluded_patterns) {
        push_target(
            targets,
            &mut seen_paths,
            "benchmarks".to_string(),
            "web-bench",
            relative_path(root, &benchmarks)?,
            provenance,
        );
    }
    if targets.is_empty() {
        push_target(
            targets,
            &mut seen_paths,
            "package".to_string(),
            "web-src",
            relative_path(root, manifest_dir)?,
            provenance,
        );
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

fn relative_path(root: &Path, path: &Path) -> Result<String, CodeIntelError> {
    match path.strip_prefix(root) {
        Ok(relative) => Ok(relative.to_string_lossy().into_owned()),
        Err(error) => Err(CodeIntelError::Identity {
            context: "derive web target source path".to_string(),
            details: error.to_string(),
        }),
    }
}
