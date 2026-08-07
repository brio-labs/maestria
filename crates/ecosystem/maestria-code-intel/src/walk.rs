//! Bounded repository file walking for identity and discovery.

use crate::CodeIntelError;
use std::fs;
use std::path::{Path, PathBuf};

/// Path-component exclusion predicate shared by identity, discovery, and
/// incremental rebuilds: `.git`/`.ssh`/`.gnupg`, `secrets`, `node_modules`,
/// `target`, `dist`, `build`, and manifest-compatible privacy patterns.
pub(crate) fn is_excluded_path(path: &Path, patterns: &[String]) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == ".git"
            || name == ".ssh"
            || name == ".gnupg"
            || name == "secrets"
            || name == "node_modules"
            || name == "target"
            || name == "dist"
            || name == "build"
            || patterns.iter().any(|pattern| {
                pattern.as_str() == name
                    || (pattern == ".env.*" && name.starts_with(".env."))
                    || (pattern == "*.pem" && name.ends_with(".pem"))
                    || (pattern == "*.key" && name.ends_with(".key"))
            })
    })
}

/// Collect every `.rs` file under `directory` as a relative path, skipping
/// `.git`, `target/`, and privacy-excluded paths. Symlinks are never
/// followed.
pub(crate) fn collect_rust_paths(
    root: &Path,
    directory: &Path,
    paths: &mut std::collections::BTreeSet<String>,
    excluded_patterns: &[String],
) -> Result<(), CodeIntelError> {
    if is_excluded_path(directory, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| CodeIntelError::Identity {
        context: "inspect Rust source directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })?;
    if metadata.is_file() {
        let relative = directory
            .strip_prefix(root)
            .map_err(|error| CodeIntelError::Identity {
                context: "derive Rust source identity path".to_string(),
                details: error.to_string(),
            })?;
        if directory
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("rs")
            && !is_excluded_path(relative, excluded_patterns)
        {
            paths.insert(relative.to_string_lossy().into_owned());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| CodeIntelError::Identity {
        context: "read Rust source directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Identity {
            context: "read Rust source directory entry".to_string(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        if child
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == ".git" || name == "target")
        {
            continue;
        }
        collect_rust_paths(root, &child, paths, excluded_patterns)?;
    }
    Ok(())
}

/// Bounded Cargo manifest discovery: every `Cargo.toml` under `root` that
/// repository-wide indexing considers, skipping `.git`, `target/`, hidden
/// directories, and privacy-excluded paths. Sorted by path so discovery and
/// identity enumeration are deterministic. Symlinks are never followed.
pub(crate) fn discover_manifests(
    root: &Path,
    excluded_patterns: &[String],
) -> Result<Vec<PathBuf>, CodeIntelError> {
    let mut manifests = Vec::new();
    collect_manifests_in(root, &mut manifests, excluded_patterns)?;
    manifests.sort();
    Ok(manifests)
}

fn collect_manifests_in(
    directory: &Path,
    manifests: &mut Vec<PathBuf>,
    excluded_patterns: &[String],
) -> Result<(), CodeIntelError> {
    if is_excluded_path(directory, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| CodeIntelError::Identity {
        context: "inspect manifest directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })?;
    if metadata.is_file() {
        if directory.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
            manifests.push(directory.to_path_buf());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| CodeIntelError::Identity {
        context: "read manifest directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Identity {
            context: "read manifest directory entry".to_string(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        if child
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "target" || name.starts_with('.'))
        {
            continue;
        }
        collect_manifests_in(&child, manifests, excluded_patterns)?;
    }
    Ok(())
}
