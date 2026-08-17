//! Bounded repository file walking for identity and discovery.

use crate::CodeIntelError;
use maestria_governance::PrivacyExclusions;
use std::fs;
use std::path::{Path, PathBuf};

/// Path-component exclusion predicate shared by identity, discovery, and
/// incremental rebuilds. A path is excluded when any component matches the
/// hard-coded walk names (`.git`/`.ssh`/`.gnupg`, `secrets`, `node_modules`,
/// `target`, `dist`, `build`, `.env.*` prefixes), a manifest-provided
/// pattern, or the shared governance [`PrivacyExclusions::default`]
/// machine-state set — the same privacy boundary the generic indexer
/// applies.
pub(crate) fn is_excluded_path(path: &Path, patterns: &[String]) -> bool {
    PrivacyExclusions::default().is_excluded(path)
        || path.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            name == ".git"
                || name == ".ssh"
                || name == ".gnupg"
                || name == "secrets"
                || name == "node_modules"
                || name == "target"
                || name == "dist"
                || name == "build"
                || name.starts_with(".env.")
                || patterns.iter().any(|pattern| {
                    pattern.as_str() == name
                        || (pattern == ".env.*" && name.starts_with(".env."))
                        || (pattern == "*.pem" && name.ends_with(".pem"))
                        || (pattern == "*.key" && name.ends_with(".key"))
                })
        })
}

/// Whether a directory name is skipped by every source walk: `.git`,
/// `target/`, hidden directories, `__pycache__`, and `*.egg-info` build
/// output.
fn is_skipped_directory(name: &str) -> bool {
    name == ".git"
        || name == "target"
        || name.starts_with('.')
        || name == "__pycache__"
        || name.ends_with(".egg-info")
}

/// Collect every file with one of `extensions` under `directory` as a
/// relative path, skipping `.git`, `target/`, hidden directories,
/// `__pycache__`, `*.egg-info`, and privacy-excluded paths. Symlinks are
/// never followed.
///
/// When `selection` is `Some`, the walk is pruned to it: a directory is
/// entered only when a selected path equals it, lies under it, or contains
/// it, and files are collected only when `selection.contains(relative)`.
pub(crate) fn collect_source_paths(
    root: &Path,
    directory: &Path,
    paths: &mut std::collections::BTreeSet<String>,
    excluded_patterns: &[String],
    selection: Option<&crate::selection::RepositorySelection>,
    extensions: &[&'static str],
) -> Result<(), CodeIntelError> {
    if is_excluded_path(directory, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| CodeIntelError::Identity {
        context: "inspect source directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })?;
    if metadata.is_file() {
        let relative = directory
            .strip_prefix(root)
            .map_err(|error| CodeIntelError::Identity {
                context: "derive source identity path".to_string(),
                details: error.to_string(),
            })?;
        if let Some(selection) = selection
            && !selection.contains(&relative.to_string_lossy())
        {
            return Ok(());
        }
        let extension = directory
            .extension()
            .and_then(|extension| extension.to_str())
            .map_or("", |extension| extension);
        if extensions.contains(&extension) && !is_excluded_path(relative, excluded_patterns) {
            paths.insert(relative.to_string_lossy().into_owned());
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }
    if let Some(selection) = selection {
        let relative = directory.strip_prefix(root).map_or_else(
            |_| String::new(),
            |relative| relative.to_string_lossy().into_owned(),
        );
        if !selection_reaches(selection, &relative) {
            return Ok(());
        }
    }
    for entry in fs::read_dir(directory).map_err(|error| CodeIntelError::Identity {
        context: "read source directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })? {
        let entry = entry.map_err(|error| CodeIntelError::Identity {
            context: "read source directory entry".to_string(),
            details: error.to_string(),
        })?;
        let child = entry.path();
        if child
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_skipped_directory)
        {
            continue;
        }
        collect_source_paths(
            root,
            &child,
            paths,
            excluded_patterns,
            selection,
            extensions,
        )?;
    }
    Ok(())
}

/// Whether the selection covers `relative` or anything under or above it:
/// some selected path equals it, lies below it, or contains it. A directory
/// is entered when it is selected, is an ancestor of a selected path, or is
/// a descendant of one (its files are then still filtered by
/// [`RepositorySelection::contains`]).
fn selection_reaches(selection: &crate::selection::RepositorySelection, relative: &str) -> bool {
    selection.is_whole()
        || selection.as_paths().any(|selected| {
            selected == relative
                || selected.starts_with(&format!("{relative}/"))
                || relative.starts_with(&format!("{selected}/"))
        })
}

/// Bounded manifest discovery: every file named in `manifest_names` under
/// `root` that repository-wide indexing considers, skipping `.git`,
/// `target/`, hidden directories, and privacy-excluded paths. Sorted by path
/// so discovery and identity enumeration are deterministic. Symlinks are
/// never followed.
pub(crate) fn discover_manifests(
    root: &Path,
    excluded_patterns: &[String],
    manifest_names: &[&'static str],
) -> Result<Vec<PathBuf>, CodeIntelError> {
    let mut manifests = Vec::new();
    collect_manifests_in(root, &mut manifests, excluded_patterns, manifest_names)?;
    manifests.sort();
    Ok(manifests)
}

fn collect_manifests_in(
    directory: &Path,
    manifests: &mut Vec<PathBuf>,
    excluded_patterns: &[String],
    manifest_names: &[&'static str],
) -> Result<(), CodeIntelError> {
    if is_excluded_path(directory, excluded_patterns) {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(directory).map_err(|error| CodeIntelError::Identity {
        context: "inspect manifest directory".to_string(),
        details: format!("{}: {error}", directory.display()),
    })?;
    if metadata.is_file() {
        if directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| manifest_names.contains(&name))
        {
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
            .is_some_and(is_skipped_directory)
        {
            continue;
        }
        collect_manifests_in(&child, manifests, excluded_patterns, manifest_names)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const NO_PATTERNS: &[String] = &[];

    fn patterns(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| entry.to_string()).collect()
    }

    #[test]
    fn is_excluded_path_applies_default_privacy_set() {
        // Machine-state and credential-shaped names from
        // `PrivacyExclusions::default()`, with no manifest patterns.
        for path in [
            "vendor/mod.rs",
            "credentials/keys.rs",
            "credential/token.rs",
            "tokens/secrets.rs",
            "password/db.rs",
            ".config/maestria/keys.rs",
            "id_rsa",
            "secret_key.pem",
            "src/keys.pem",
            "src/certs/key.pfx",
            "src/creds.env",
        ] {
            assert!(
                is_excluded_path(Path::new(path), NO_PATTERNS),
                "{path} should be excluded by the default privacy set"
            );
        }
    }

    #[test]
    fn is_excluded_path_keeps_regular_source_paths() {
        for path in [
            "src/lib.rs",
            "src/main.rs",
            "crates/one/src/lib.rs",
            "vendor-dump/asset.rs",
            "vendorish/mod.rs",
            "secret_notes/readme.rs",
        ] {
            assert!(
                !is_excluded_path(Path::new(path), NO_PATTERNS),
                "{path} should not be excluded"
            );
        }
    }

    #[test]
    fn is_excluded_path_applies_hardcoded_and_manifest_patterns() {
        // Hard-coded walk names apply without manifest patterns.
        for path in [
            ".ssh/keys.rs",
            "node_modules/pkg/lib.rs",
            "target/debug/out.rs",
        ] {
            assert!(
                is_excluded_path(Path::new(path), NO_PATTERNS),
                "{path} should be excluded by the hard-coded walk set"
            );
        }
        // Manifest-provided patterns still apply on top of the defaults.
        let manifest = patterns(&[".env.*", "custom-secret", "*.pem"]);
        for path in ["config/custom-secret/keys.rs", "keys.pem"] {
            assert!(
                is_excluded_path(Path::new(path), &manifest),
                "{path} should be excluded by the manifest pattern"
            );
        }
        assert!(!is_excluded_path(Path::new("src/lib.rs"), &manifest));
    }

    #[test]
    fn collect_source_paths_skips_default_privacy_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let src = root.join("src");
        let vendor = root.join("vendor");
        let credentials = root.join("credentials");
        fs::create_dir_all(&src)?;
        fs::create_dir_all(&vendor)?;
        fs::create_dir_all(&credentials)?;
        fs::write(src.join("lib.rs"), "pub fn lib() {}\n")?;
        fs::write(vendor.join("dep.rs"), "pub fn dep() {}\n")?;
        fs::write(credentials.join("leak.rs"), "pub fn leak() {}\n")?;

        let mut paths = BTreeSet::new();
        collect_source_paths(root, root, &mut paths, NO_PATTERNS, None, &["rs"])?;

        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            vec!["src/lib.rs".to_string()],
            "only the non-excluded source should be collected"
        );
        Ok(())
    }

    #[test]
    fn discover_manifests_skips_default_privacy_directories()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let root = temp.path();
        let vendor = root.join("vendor");
        fs::create_dir_all(&vendor)?;
        fs::write(root.join("Cargo.toml"), "[workspace]\n")?;
        fs::write(vendor.join("Cargo.toml"), "[package]\n")?;

        let manifests = discover_manifests(root, NO_PATTERNS, &["Cargo.toml"])?;

        assert_eq!(
            manifests,
            vec![root.join("Cargo.toml")],
            "the vendor manifest must not be discovered"
        );
        Ok(())
    }
}
