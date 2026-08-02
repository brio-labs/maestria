//! Source-path identity keys shared by the watcher and recovery staging.
//!
//! A source path's identity key is its canonical form when the file exists;
//! when the path cannot be canonicalized (file just created, just removed, or
//! no longer present), the caller-supplied path is used verbatim. This is the
//! deliberate policy for identity keys: the watcher must track files across
//! appearance/disappearance races, and recovery staging must reconcile stored
//! paths that may no longer exist (R24: the fallback is the modeled policy,
//! not an error discard).

use std::path::Path;

/// Deterministic identity key for a source path (R28: one owner).
pub fn source_key(path: &Path) -> String {
    match path.canonicalize() {
        Ok(path) => path.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Identity key for a stored source path; falls back to the stored string
/// when the path no longer exists on disk (recovery reconciliation).
pub fn stored_source_key(stored: &str) -> String {
    source_key(Path::new(stored))
}
