//! Bounded `cargo metadata` extraction for workspace packages.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::provenance::content_hash;
use crate::types::{DependencyRecord, PackageRecord, RecordProvenance, SourceRange, TargetRecord};
use crate::walk::{discover_manifests, is_excluded_path};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct RawMetadata {
    packages: Vec<RawPackage>,
    workspace_members: Vec<String>,
    workspace_root: String,
}

#[derive(Debug, Deserialize)]
struct RawPackage {
    id: String,
    name: String,
    version: String,
    manifest_path: String,
    edition: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    source: Option<String>,
    description: Option<String>,
    #[serde(default)]
    features: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    targets: Vec<RawTarget>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawDependencyKind {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct RawDependency {
    name: String,
    package: Option<String>,
    source: Option<String>,
    #[serde(default)]
    req: String,
    kind: Option<RawDependencyKind>,
    #[serde(default)]
    optional: bool,
    #[serde(default, rename = "uses_default_features")]
    uses_default_features: bool,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    registry: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTarget {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
    #[serde(default)]
    crate_types: Vec<String>,
    #[serde(default)]
    src_path: String,
    #[serde(default)]
    required_features: Vec<String>,
    #[serde(default)]
    doctest: bool,
    #[serde(default)]
    doc: bool,
    #[serde(default)]
    test: bool,
    #[serde(default)]
    bench: bool,
}

/// Repository-wide package discovery result: every package across all
/// discovered workspaces plus per-workspace degradation warnings.
pub(crate) struct WorkspaceDiscovery {
    pub packages: Vec<PackageRecord>,
    pub warnings: Vec<String>,
}

/// Discovers every Cargo workspace under `root` and runs
/// `cargo metadata --no-deps --format-version 1 --manifest-path <manifest>`
/// once per distinct workspace root (member manifests are skipped without a
/// cargo invocation once their workspace is indexed).
///
/// A repository without any `Cargo.toml` has no Rust workspace: it yields an
/// empty package list (a valid, fresh code index with no symbols) instead of
/// failing. A root manifest that exists but fails cargo metadata is a real
/// error and propagates; a failing NESTED manifest degrades per-workspace:
/// the workspace is skipped and a warning is recorded, never silently
/// (Rules 24, 50).
pub(crate) fn extract_workspace_packages(
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<WorkspaceDiscovery, CodeIntelError> {
    let root_manifest = root.join("Cargo.toml");
    let mut seen_workspaces: BTreeSet<String> = BTreeSet::new();
    let mut seen_members: BTreeSet<PathBuf> = BTreeSet::new();
    let mut seen_packages: BTreeSet<String> = BTreeSet::new();
    let mut packages: Vec<PackageRecord> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if root_manifest.exists() {
        let metadata = run_cargo_metadata(root, &root_manifest)?;
        seen_workspaces.insert(metadata.workspace_root.clone());
        collect_workspace_packages(
            metadata,
            identity,
            parser_generation,
            excluded_patterns,
            &mut packages,
            &mut seen_packages,
            &mut seen_members,
        )?;
    }

    // Nested manifests: member manifests of an already-indexed workspace are
    // skipped without a cargo invocation; a failing manifest warns and is
    // skipped; a duplicate workspace root is converted once.
    for manifest in discover_manifests(root, excluded_patterns)? {
        if manifest == root_manifest || seen_members.contains(&manifest) {
            continue;
        }
        let metadata = match run_cargo_metadata(root, &manifest) {
            Ok(metadata) => metadata,
            Err(error) => {
                let relative = match manifest.strip_prefix(root) {
                    Ok(path) => path.to_string_lossy().into_owned(),
                    Err(_) => manifest.to_string_lossy().into_owned(),
                };
                warnings.push(format!("workspace at {relative}: {error}"));
                continue;
            }
        };
        if !seen_workspaces.insert(metadata.workspace_root.clone()) {
            continue;
        }
        collect_workspace_packages(
            metadata,
            identity,
            parser_generation,
            excluded_patterns,
            &mut packages,
            &mut seen_packages,
            &mut seen_members,
        )?;
    }

    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(WorkspaceDiscovery { packages, warnings })
}

/// Runs `cargo metadata --no-deps --format-version 1 --manifest-path` for
/// one manifest from the repository root.
fn run_cargo_metadata(root: &Path, manifest_path: &Path) -> Result<RawMetadata, CodeIntelError> {
    let command = "cargo metadata";
    let output = Command::new("cargo")
        .current_dir(root)
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(manifest_path)
        .output()
        .map_err(|error| CodeIntelError::Command {
            command: command.to_string(),
            status: None,
            details: error.to_string(),
        })?;

    if !output.status.success() {
        return Err(CodeIntelError::Command {
            command: command.to_string(),
            status: output.status.code(),
            details: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    serde_json::from_slice::<RawMetadata>(&output.stdout).map_err(|error| CodeIntelError::Parse {
        context: "cargo metadata JSON".to_string(),
        details: error.to_string(),
    })
}

/// Convert one workspace's packages, deduped by `package_id`; every member
/// manifest is recorded so discovery skips it without a cargo invocation.
fn collect_workspace_packages(
    metadata: RawMetadata,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
    packages: &mut Vec<PackageRecord>,
    seen_packages: &mut BTreeSet<String>,
    seen_members: &mut BTreeSet<PathBuf>,
) -> Result<(), CodeIntelError> {
    let workspace_members: BTreeSet<String> = metadata.workspace_members.into_iter().collect();
    for package in metadata.packages.into_iter() {
        if !workspace_members.contains(&package.id) {
            continue;
        }
        if is_excluded_path(Path::new(&package.manifest_path), excluded_patterns) {
            continue;
        }
        let record = convert_package(package, identity, parser_generation, excluded_patterns)?;
        seen_members.insert(PathBuf::from(&record.manifest_path));
        if seen_packages.insert(record.package_id.clone()) {
            packages.push(record);
        }
    }
    Ok(())
}
fn convert_package(
    package: RawPackage,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<PackageRecord, CodeIntelError> {
    let package_provenance =
        metadata_provenance(&package.manifest_path, identity, parser_generation)?;
    let dependencies = package
        .dependencies
        .into_iter()
        .map(|dependency| DependencyRecord {
            name: dependency.name,
            package: dependency.package,
            source: dependency.source,
            version_req: dependency.req,
            kind: match dependency.kind {
                Some(RawDependencyKind::Single(kind)) => vec![kind],
                Some(RawDependencyKind::Multiple(kinds)) => kinds,
                None => Vec::new(),
            },
            optional: dependency.optional,
            uses_default_features: dependency.uses_default_features,
            features: dependency.features,
            target: dependency.target,
            registry: dependency.registry,
            provenance: package_provenance.clone(),
        })
        .collect();

    let targets = package
        .targets
        .into_iter()
        .filter(|target| !is_excluded_path(Path::new(&target.src_path), excluded_patterns))
        .map(|target| TargetRecord {
            name: target.name,
            kind: target.kind,
            crate_types: target.crate_types,
            src_path: target.src_path,
            required_features: target.required_features,
            doctest: target.doctest,
            test: target.test,
            bench: target.bench,
            doc: target.doc,
            provenance: package_provenance.clone(),
        })
        .collect();

    Ok(PackageRecord {
        package_id: package.id,
        name: package.name,
        version: package.version,
        manifest_path: package.manifest_path,
        edition: package.edition,
        authors: package.authors,
        source: package.source,
        description: package.description,
        features: package.features,
        dependencies,
        targets,
        provenance: package_provenance,
    })
}

fn metadata_provenance(
    manifest_path: &str,
    identity: &RepositoryIdentity,
    parser_generation: &str,
) -> Result<RecordProvenance, CodeIntelError> {
    let manifest_path = PathBuf::from(manifest_path);
    let bytes = std::fs::read(&manifest_path).map_err(|error| CodeIntelError::Io {
        operation: "read Cargo manifest for provenance".to_string(),
        path: manifest_path.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let file_path = match manifest_path.strip_prefix(Path::new(&identity.root)) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => manifest_path.to_string_lossy().into_owned(),
    };
    Ok(RecordProvenance {
        repository_root: identity.root.clone(),
        commit_sha: identity.commit.clone(),
        worktree_identity: identity.worktree_identity.clone(),
        content_hash: content_hash(&bytes),
        file_path,
        source_range: SourceRange::new(1, 1)?,
        parser_generation: crate::types::ParserGeneration::new(parser_generation),
    })
}
