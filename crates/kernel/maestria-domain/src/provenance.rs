#![forbid(unsafe_code)]

use crate::types::{ArtifactId, ContentRange, EvidenceId};
use sha2::{Digest, Sha256};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize,
)]
pub enum ParseStatus {
    Parsed,
    Unsupported,
    Failed,
    MetadataOnly,
    NeedsOcr,
    Quarantined,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize,
)]
pub enum RepresentationKind {
    Raw,
    Retrieval,
    Contextual,
    Summary,
    Visual,
}

#[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub struct ParsedRepresentation {
    pub kind: RepresentationKind,
    pub content: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize,
)]
pub enum SourceSpan {
    TextSpan {
        start_line: usize,
        end_line: usize,
    },
    PdfSpan {
        page: usize,
    },
    PdfRegion {
        page: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
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
/// Returns a stable artifact identity for an externally fetched source.
pub fn web_artifact_id_for(url: &str, content_hash: &str) -> ArtifactId {
    let mut hasher = Sha256::new();
    hasher.update(b"maestria:web-artifact\0");
    hasher.update(url.as_bytes());
    hasher.update([0]);
    hasher.update(content_hash.as_bytes());
    let digest = hasher.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    ArtifactId::new(u64::from_be_bytes(id_bytes))
}

/// Returns a stable evidence identity for a fetched web artifact.
pub fn web_evidence_id_for(artifact_id: ArtifactId) -> EvidenceId {
    let mut hasher = Sha256::new();
    hasher.update(b"maestria:web-evidence\0");
    hasher.update(artifact_id.value().to_be_bytes());
    let digest = hasher.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    EvidenceId::new(u64::from_be_bytes(id_bytes))
}

pub fn evidence_id_for(artifact_id: ArtifactId, order: u32) -> EvidenceId {
    EvidenceId::new(
        artifact_id
            .value()
            .wrapping_mul(1_000_003)
            .wrapping_add(u64::from(order))
            .wrapping_add(500_001),
    )
}

pub fn line_range_for_chunk(source: &str, chunk: &str, search_start: &mut usize) -> ContentRange {
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

pub fn excerpt_for(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(240).collect()
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn line_number_at(text: &str, byte_index: usize) -> usize {
    let capped = byte_index.min(text.len());
    text[..capped].bytes().filter(|byte| *byte == b'\n').count() + 1
}
