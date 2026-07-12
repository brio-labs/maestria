use maestria_domain::ContentRange;
use maestria_domain::{ArtifactId, EvidenceId};
use maestria_ports::FileMetadata;
use sha2::{Digest, Sha256};

use std::path::Path;

pub(crate) fn file_metadata(path: &Path, size: usize) -> FileMetadata {
    FileMetadata {
        path: path.to_path_buf(),
        size,
        extension: path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase),
    }
}

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

pub(crate) fn evidence_id_for(artifact_id: ArtifactId, order: u32) -> EvidenceId {
    EvidenceId::new(
        artifact_id
            .value()
            .wrapping_mul(1_000_003)
            .wrapping_add(u64::from(order))
            .wrapping_add(500_001),
    )
}

/// Deterministically produces a content-addressed hash string.
///
/// Returns a `"sha256:<hex>"` string suitable for identifying byte content
/// without requiring the full bytes. The output is stable across all hosts
/// and processes.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_digest(&digest))
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn non_zero_id(value: u64) -> u64 {
    if value == 0 { 1 } else { value }
}

pub(crate) fn decode_utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(crate) fn line_range_for_chunk(
    source: &str,
    chunk: &str,
    search_start: &mut usize,
) -> ContentRange {
    let found = source
        .get(*search_start..)
        .and_then(|tail| tail.find(chunk).map(|offset| *search_start + offset))
        .or_else(|| source.find(chunk));
    let (start_byte, end_byte) = match found {
        Some(start) => {
            let end = start.saturating_add(chunk.len());
            *search_start = end;
            (start, end)
        }
        None => {
            let start_line = line_number_at(source, *search_start);
            let line_count = chunk.lines().count().max(1);
            return ContentRange {
                start: start_line,
                end: start_line.saturating_add(line_count).saturating_sub(1),
            };
        }
    };

    ContentRange {
        start: line_number_at(source, start_byte),
        end: line_number_at(source, end_byte.saturating_sub(1))
            .max(line_number_at(source, start_byte)),
    }
}

fn line_number_at(text: &str, byte_index: usize) -> usize {
    let capped = byte_index.min(text.len());
    text[..capped].bytes().filter(|byte| *byte == b'\n').count() + 1
}

pub(crate) fn excerpt_for(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(240).collect()
}
