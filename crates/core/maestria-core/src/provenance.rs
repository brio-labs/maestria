use maestria_domain::ArtifactId;
use sha2::{Digest, Sha256};

use std::path::Path;

// Re-export shared pure provenance helpers from the domain kernel.
pub use maestria_domain::content_hash;

pub(crate) fn title_for_path(path: &Path) -> String {
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty());
    match title {
        Some(title) => title.to_string(),
        None => "artifact".to_string(),
    }
}

/// Deterministically produces an [`ArtifactId`] from a file path and content hash.
///
/// Hashes the canonical path string and the hex content hash together using
/// SHA-256. Using the precomputed content hash avoids re-hashing full bytes
/// when the caller already holds the hash (e.g. watcher observation).
pub fn artifact_id_for_content_hash(path: &Path, content_hash: &str) -> ArtifactId {
    let mut hasher = Sha256::new();
    hasher.update(path.display().to_string().as_bytes());
    hasher.update([0]);
    hasher.update(content_hash.as_bytes());
    let digest = hasher.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    ArtifactId::new(non_zero_id(u64::from_be_bytes(id_bytes) % 1_000_000_000))
}

/// Deterministically produces an [`ArtifactId`] from a file path and content.
///
/// The ID is a stable, content-addressed value suitable for deduplication
/// and indexing. It hashes the canonical UTF-8 path representation and raw
/// bytes together using SHA-256, then folds the digest into a `u64`-backed
/// identifier.
pub fn artifact_id_for(path: &Path, bytes: &[u8]) -> ArtifactId {
    let mut hasher = Sha256::new();
    hasher.update(path.display().to_string().as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    ArtifactId::new(non_zero_id(u64::from_be_bytes(id_bytes) % 1_000_000_000))
}

pub(crate) fn non_zero_id(value: u64) -> u64 {
    if value == 0 { 1 } else { value }
}
