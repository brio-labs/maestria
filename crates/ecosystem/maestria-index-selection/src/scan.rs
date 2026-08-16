//! Directory walker, file ingestion policy, and per-directory features.

use anyhow::Result;
use maestria_governance::PrivacyExclusions;
use std::fs;
use std::path::{Path, PathBuf};

/// Whether `path` names a source file eligible for ingestion.
///
/// Union of the CLI and watcher policies: the CLI's `Cargo.toml` special case
/// plus case-insensitive extensions md|markdown|txt|text|rs|toml|json|yaml|
/// yml|pdf.
pub fn is_supported_source_file(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt" | "text" | "rs" | "toml" | "json" | "yaml" | "yml" | "pdf")
    )
}

/// Whether `path` traverses a privacy-excluded component.
///
/// The CLI's hard-coded component exclusions (`.ssh`, `.gnupg`, `node_modules`,
/// `target`, `dist`, `build`, `.env.*` prefixes) are OR-ed with the shared
/// governance privacy exclusions.
pub fn is_privacy_excluded_path(path: &Path) -> bool {
    let default_exclusions = PrivacyExclusions::default();
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".ssh" | ".gnupg" | "node_modules" | "target" | "dist" | "build"
        ) || name.starts_with(".env.")
    }) || default_exclusions.is_excluded(path)
}

/// Whether `root` is the user's home directory itself.
pub fn is_home_root(root: &Path) -> bool {
    root.canonicalize().ok()
        == std::env::var_os("HOME")
            .map(PathBuf::from)
            .and_then(|home| home.canonicalize().ok())
}

/// Collect the supported, non-excluded source files under `path`, in
/// deterministic (sorted) order.
///
/// A direct file target must itself be supported and privacy-clean; a
/// directory target requires `recursive` and is walked with
/// `ignore::WalkBuilder` (hidden, ignore files, gitignore, no symlink
/// following, no symlinked entries).
pub fn collect_files(path: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    if is_privacy_excluded_path(path) {
        return Err(anyhow::anyhow!(
            "index path is excluded by privacy policy: {}",
            path.display()
        ));
    }
    if is_symlink(path)? {
        return Err(anyhow::anyhow!(
            "index path is a symlink and is not indexed: {}",
            path.display()
        ));
    }
    if path.is_file() {
        if !is_supported_source_file(path) {
            return Err(anyhow::anyhow!(
                "unsupported index file type: {}",
                path.display()
            ));
        }
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(anyhow::anyhow!(
            "index path does not exist: {}",
            path.display()
        ));
    }
    if !recursive {
        return Err(anyhow::anyhow!(
            "{} is a directory; pass --recursive to index contained files",
            path.display()
        ));
    }

    let mut files = Vec::new();
    let walker = ignore::WalkBuilder::new(path)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false)
        .build();

    for result in walker {
        let entry = result?;
        let entry_path = entry.path();
        if let Some(error) = entry.error() {
            return Err(anyhow::anyhow!(
                "index traversal failed at {}: {error}",
                entry_path.display()
            ));
        }

        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_symlink())
        {
            continue;
        }
        if is_privacy_excluded_path(entry_path) {
            continue;
        }

        if entry_path.is_file() && is_supported_source_file(entry_path) {
            files.push(entry_path.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

fn is_symlink(path: &Path) -> Result<bool> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

/// Numeric features of a directory's file population, used by the
/// deterministic classification rules.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirFeatures {
    pub file_count: usize,
    pub total_bytes: u64,
    pub max_file_bytes: u64,
    pub mean_bytes: u64,
    pub doc_share: f64,
    pub code_share: f64,
    pub single_ext_share: f64,
    pub minified_share: f64,
}

/// Extension buckets for the doc/code shares.
const DOC_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "text", "pdf"];
const CODE_EXTENSIONS: &[&str] = &["rs", "py", "ts", "tsx"];

/// Compute the numeric features of the directory `_dir` from `files` (all
/// collected files under it).
pub fn dir_features(_dir: &Path, files: &[PathBuf]) -> DirFeatures {
    dir_features_buckets(_dir, files, DOC_EXTENSIONS, CODE_EXTENSIONS)
}

/// [`dir_features`] over explicit doc/code extension buckets, shared with
/// the repository-mode scan (`repo.rs`).
pub(crate) fn dir_features_buckets(
    _dir: &Path,
    files: &[PathBuf],
    doc_extensions: &[&str],
    code_extensions: &[&str],
) -> DirFeatures {
    let mut total_bytes = 0u64;
    let mut max_file_bytes = 0u64;
    let mut doc_count = 0usize;
    let mut code_count = 0usize;
    let mut minified_count = 0usize;
    let mut extension_counts: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for file in files {
        let size = fs::metadata(file).map_or(0, |metadata| metadata.len());
        total_bytes += size;
        max_file_bytes = max_file_bytes.max(size);
        let extension = file
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if let Some(extension) = &extension {
            *extension_counts.entry(extension.clone()).or_insert(0) += 1;
        }
        if extension
            .as_deref()
            .is_some_and(|extension| doc_extensions.contains(&extension))
        {
            doc_count += 1;
        }
        if extension
            .as_deref()
            .is_some_and(|extension| code_extensions.contains(&extension))
        {
            code_count += 1;
        }
        if size >= 256 * 1024 && super::policy::looks_minified(file) {
            minified_count += 1;
        }
    }
    let count = files.len();
    let share = |part: usize| {
        if count == 0 {
            0.0
        } else {
            part as f64 / count as f64
        }
    };
    let single_ext_share = extension_counts.values().copied().max().map_or(0.0, &share);
    DirFeatures {
        file_count: count,
        total_bytes,
        max_file_bytes,
        mean_bytes: if count == 0 {
            0
        } else {
            total_bytes / count as u64
        },
        doc_share: share(doc_count),
        code_share: share(code_count),
        single_ext_share,
        minified_share: share(minified_count),
    }
}
