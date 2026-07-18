//! Repository identity discovery for provenance.

use crate::CodeIntelError;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub(crate) struct RepositoryIdentity {
    pub(crate) root: String,
    pub(crate) commit: String,
    pub(crate) worktree_identity: String,
}

/// Derive repository identity values used for provenance without reading excluded files.
pub(crate) fn discover_repository_identity(
    root: &Path,
    excluded_patterns: &[String],
) -> Result<RepositoryIdentity, CodeIntelError> {
    let canonical_root = canonical_root(root)?;
    let commit = git_output(root, &["rev-parse", "HEAD"], "git rev-parse HEAD")?;
    let file_listing = git_output(
        root,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
        "git ls-files",
    )?;

    let mut paths: BTreeSet<String> = file_listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| is_identity_input(Path::new(line), excluded_patterns))
        .map(str::to_string)
        .collect();
    collect_rust_paths(root, root, &mut paths, excluded_patterns)?;

    let mut hasher = Sha256::new();
    for relative_path in paths {
        let path = root.join(&relative_path);
        let metadata = fs::symlink_metadata(&path).map_err(|error| CodeIntelError::Identity {
            context: "inspect repository identity file".to_string(),
            details: format!("{relative_path}: {error}"),
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let mut file = File::open(&path).map_err(|error| CodeIntelError::Identity {
            context: "open repository identity file".to_string(),
            details: format!("{relative_path}: {error}"),
        })?;
        hasher.update(relative_path.as_bytes());
        hasher.update(b"\0");
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| CodeIntelError::Identity {
                    context: "read repository identity file".to_string(),
                    details: format!("{relative_path}: {error}"),
                })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        hasher.update(b"\0");
    }

    Ok(RepositoryIdentity {
        root: canonical_root,
        commit,
        worktree_identity: to_hex(&hasher.finalize()),
    })
}

fn collect_rust_paths(
    root: &Path,
    directory: &Path,
    paths: &mut BTreeSet<String>,
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
            && is_identity_input(relative, excluded_patterns)
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

fn is_identity_input(path: &Path, excluded_patterns: &[String]) -> bool {
    let is_source = matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs" | "toml" | "lock")
    );
    is_source && !is_excluded_path(path, excluded_patterns)
}

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

fn git_output(root: &Path, args: &[&str], context: &str) -> Result<String, CodeIntelError> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| CodeIntelError::Command {
            command: "git".to_owned(),
            status: None,
            details: format!("{context}: {error}"),
        })?;

    if !output.status.success() {
        return Err(CodeIntelError::Command {
            command: "git".to_owned(),
            status: output.status.code(),
            details: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let output = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.is_empty() {
        return Err(CodeIntelError::Identity {
            context: context.to_owned(),
            details: "empty output".to_string(),
        });
    }

    Ok(output)
}

fn canonical_root(path: &Path) -> Result<String, CodeIntelError> {
    path.canonicalize()
        .map_err(|error| CodeIntelError::Identity {
            context: "canonicalize root".to_string(),
            details: error.to_string(),
        })
        .map(|root| root.to_string_lossy().into_owned())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}
