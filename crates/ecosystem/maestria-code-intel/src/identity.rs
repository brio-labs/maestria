//! Repository identity discovery for provenance.

use crate::CodeIntelError;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub(crate) struct RepositoryIdentity {
    pub(crate) root: String,
    pub(crate) commit: crate::types::CommitSha,
    pub(crate) worktree_identity: crate::types::WorktreeIdentity,
}

/// Derive repository identity values used for provenance without reading excluded files.
pub(crate) fn discover_repository_identity(
    root: &Path,
    excluded_patterns: &[String],
) -> Result<RepositoryIdentity, CodeIntelError> {
    let canonical_root = canonical_root(root)?;
    let commit = git_output(root, &["rev-parse", "HEAD"], "git rev-parse HEAD")?;
    let dirty = discover_dirty_paths(root)?;
    let file_set = discover_file_set(root)?;
    let blob_map = git_blob_map(root)?;

    let mut paths: BTreeSet<String> = file_set
        .iter()
        .filter(|line| is_identity_input(Path::new(line), excluded_patterns))
        .cloned()
        .collect();
    collect_rust_paths(root, root, &mut paths, excluded_patterns)?;

    let mut hasher = Sha256::new();
    hasher.update(b"maestria-worktree-identity-v2\0");
    // Pass 1: per-path presence (missing marker vs path record).
    for relative_path in &paths {
        let path = root.join(relative_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"missing\0");
                continue;
            }
            Err(error) => {
                return Err(CodeIntelError::Identity {
                    context: "inspect repository identity file".to_string(),
                    details: format!("{relative_path}: {error}"),
                });
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        hasher.update((relative_path.len() as u64).to_le_bytes());
        hasher.update(relative_path.as_bytes());
    }
    // Pass 2: per-path content record: file marker, path, tag, digest.
    // Tag 0x01 = 32-byte SHA-256 of file content (dirty/untracked/ignored files),
    // tag 0x02 = 20-byte blob SHA-1 from the git index (clean tracked files).
    for relative_path in &paths {
        let path = root.join(relative_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(CodeIntelError::Identity {
                    context: "inspect repository identity file".to_string(),
                    details: format!("{relative_path}: {error}"),
                });
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        hasher.update(b"file\0");
        hasher.update((relative_path.len() as u64).to_le_bytes());
        hasher.update(relative_path.as_bytes());
        if dirty.contains(relative_path) || !blob_map.contains_key(relative_path) {
            let mut file_hasher = Sha256::new();
            let mut file = File::open(&path).map_err(|error| CodeIntelError::Identity {
                context: "open repository identity file".to_string(),
                details: format!("{relative_path}: {error}"),
            })?;
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
                file_hasher.update(&buffer[..read]);
            }
            hasher.update([0x01]);
            hasher.update(file_hasher.finalize());
        } else {
            hasher.update([0x02]);
            hasher.update(&blob_map[relative_path]);
        }
    }

    Ok(RepositoryIdentity {
        root: canonical_root,
        commit: crate::types::CommitSha::new(commit),
        worktree_identity: crate::types::WorktreeIdentity::new(to_hex(&hasher.finalize())),
    })
}

/// The `git ls-files --cached --others --exclude-standard` file set: every
/// tracked plus untracked non-ignored path, one per line.
pub(crate) fn discover_file_set(root: &Path) -> Result<BTreeSet<String>, CodeIntelError> {
    let file_listing = git_output(
        root,
        &["ls-files", "--cached", "--others", "--exclude-standard"],
        "git ls-files",
    )?;
    Ok(file_listing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Worktree-dirty paths from `git status --porcelain -z`: any record whose
/// worktree status column differs from a space, plus records whose index
/// status column differs (staged edits/renames/deletes — the worktree content
/// then differs from what the previous build extracted even though porcelain
/// shows no worktree change). Rename/copy entries contribute their target
/// path (and their source path when the record is not fully clean).
pub(crate) fn discover_dirty_paths(root: &Path) -> Result<BTreeSet<String>, CodeIntelError> {
    let output = git_output_allow_empty(
        root,
        &["status", "--porcelain", "-z"],
        "git status --porcelain",
    )?;
    let mut dirty = BTreeSet::new();
    let mut records = output.split('\0');
    while let Some(record) = records.next() {
        let bytes = record.as_bytes();
        if bytes.len() < 3 || bytes[2] != b' ' {
            continue;
        }
        let x = bytes[0];
        let y = bytes[1];
        let path = &record[3..];
        if y != b' ' || x != b' ' {
            dirty.insert(path.to_string());
        }
        if x == b'R' || x == b'C' {
            if let Some(target) = records.next() {
                if y != b' ' || x != b' ' {
                    dirty.insert(target.to_string());
                }
            }
        }
    }
    Ok(dirty)
}

/// Path -> 20-byte blob SHA-1 map from `git ls-files -s` (staged content is
/// exactly the worktree content for clean tracked files).
fn git_blob_map(root: &Path) -> Result<BTreeMap<String, [u8; 20]>, CodeIntelError> {
    let output = git_output_allow_empty(root, &["ls-files", "-s"], "git ls-files -s")?;
    let mut blobs = BTreeMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let Some(sha_hex) = meta.split_ascii_whitespace().nth(1) else {
            continue;
        };
        if sha_hex.len() != 40 {
            continue;
        }
        let Some(sha) = decode_hex_sha(sha_hex.as_bytes()) else {
            continue;
        };
        blobs.insert(path.to_string(), sha);
    }
    Ok(blobs)
}

fn decode_hex_sha(bytes: &[u8]) -> Option<[u8; 20]> {
    let mut sha = [0_u8; 20];
    for (index, byte) in sha.iter_mut().enumerate() {
        *byte = decode_hex_pair(&bytes[index * 2..index * 2 + 2])?;
    }
    Some(sha)
}

fn decode_hex_pair(bytes: &[u8]) -> Option<u8> {
    let high = decode_hex_nibble(bytes[0])?;
    let low = decode_hex_nibble(bytes[1])?;
    Some((high << 4) | low)
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn collect_rust_paths(
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

/// Like [`git_output`] but treats empty stdout as valid and returns the raw
/// (untrimmed) stdout. Used by NUL-delimited calls whose leading-space records
/// and empty output are meaningful (clean worktree, empty index); callers
/// split on `\0` or trim per line themselves.
fn git_output_allow_empty(
    root: &Path,
    args: &[&str],
    context: &str,
) -> Result<String, CodeIntelError> {
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

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn run_git_ok(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?} failed in {root:?}");
    }

    fn init_repo(root: &Path) {
        run_git_ok(root, &["init", "--initial-branch", "main"]);
        run_git_ok(root, &["config", "user.email", "ci@example.com"]);
        run_git_ok(root, &["config", "user.name", "CI"]);
    }

    #[test]
    fn clean_blob_hash_matches_git_hash_object() -> Result<(), CodeIntelError> {
        let root = tempdir().expect("tempdir");
        init_repo(root.path());
        fs::write(root.path().join("a.txt"), "hello blob\n").expect("write file");
        run_git_ok(root.path(), &["add", "a.txt"]);
        run_git_ok(root.path(), &["commit", "-m", "init"]);

        let blobs = git_blob_map(root.path())?;
        let hash = git_output(root.path(), &["hash-object", "a.txt"], "git hash-object")?;
        assert_eq!(to_hex(&blobs["a.txt"]), hash);
        Ok(())
    }

    #[test]
    fn identity_changes_then_restores() -> Result<(), CodeIntelError> {
        let root = tempdir().expect("tempdir");
        init_repo(root.path());
        fs::create_dir_all(root.path().join("src")).expect("create src");
        let source = root.path().join("src/lib.rs");
        fs::write(&source, "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").expect("write source");
        run_git_ok(root.path(), &["add", "."]);
        run_git_ok(root.path(), &["commit", "-m", "init"]);

        let original = discover_repository_identity(root.path(), &[])?.worktree_identity;

        fs::write(&source, "pub fn add(a: i32, b: i32) -> i32 { a - b }\n").expect("modify source");
        let modified = discover_repository_identity(root.path(), &[])?.worktree_identity;
        assert_ne!(original, modified);

        run_git_ok(root.path(), &["checkout", "--", "src/lib.rs"]);
        let restored = discover_repository_identity(root.path(), &[])?.worktree_identity;
        assert_eq!(original, restored);
        Ok(())
    }
}
