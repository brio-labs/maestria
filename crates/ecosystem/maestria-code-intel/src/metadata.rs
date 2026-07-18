//! Bounded `cargo metadata` extraction for workspace packages.

use crate::CodeIntelError;
use crate::identity::{RepositoryIdentity, is_excluded_path};
use crate::types::{DependencyRecord, PackageRecord, RecordProvenance, SourceRange, TargetRecord};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct RawMetadata {
    packages: Vec<RawPackage>,
    workspace_members: Vec<String>,
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

/// Runs `cargo metadata --no-deps --format-version 1` from `root`.
pub(crate) fn extract_workspace_packages(
    root: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> Result<Vec<PackageRecord>, CodeIntelError> {
    let command = "cargo metadata";
    let output = Command::new("cargo")
        .current_dir(root)
        .arg("metadata")
        .arg("--no-deps")
        .arg("--format-version")
        .arg("1")
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

    let metadata = serde_json::from_slice::<RawMetadata>(&output.stdout).map_err(|error| {
        CodeIntelError::Parse {
            context: "cargo metadata JSON".to_string(),
            details: error.to_string(),
        }
    })?;

    let workspace_members: BTreeSet<String> = metadata.workspace_members.into_iter().collect();

    let mut packages: Vec<PackageRecord> = metadata
        .packages
        .into_iter()
        .filter(|package| workspace_members.contains(&package.id))
        .filter(|package| !is_excluded_path(Path::new(&package.manifest_path), excluded_patterns))
        .map(|package| convert_package(package, identity, parser_generation, excluded_patterns))
        .collect();

    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(packages)
}
fn convert_package(
    package: RawPackage,
    identity: &RepositoryIdentity,
    parser_generation: &str,
    excluded_patterns: &[String],
) -> PackageRecord {
    let package_provenance =
        metadata_provenance(&package.manifest_path, identity, parser_generation);
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

    PackageRecord {
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
    }
}

fn metadata_provenance(
    manifest_path: &str,
    identity: &RepositoryIdentity,
    parser_generation: &str,
) -> RecordProvenance {
    let manifest_path = PathBuf::from(manifest_path);
    let file_path = match manifest_path.strip_prefix(Path::new(&identity.root)) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => manifest_path.to_string_lossy().into_owned(),
    };
    RecordProvenance {
        repository_root: identity.root.clone(),
        commit_sha: identity.commit.clone(),
        worktree_identity: identity.worktree_identity.clone(),
        file_path,
        source_range: SourceRange {
            start_line: 1,
            end_line: 1,
        },
        parser_generation: parser_generation.to_string(),
    }
}
