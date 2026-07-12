#![forbid(unsafe_code)]

use crate::types::{ArtifactId, ContentRange, EvidenceId};
use sha2::{Digest, Sha256};

/// Deterministically produces a content-addressed hash string.
///
/// Returns a `"sha256:<hex>"` string suitable for identifying byte content
/// without requiring the full bytes. The output is stable across all hosts
/// and processes.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_digest(&digest))
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

pub fn line_range_for_chunk(
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
