use maestria_domain::ContentRange;
use maestria_domain::{ArtifactId, EvidenceId};
use maestria_ports::FileMetadata;

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

pub(crate) fn artifact_id_for(path: &Path, bytes: &[u8]) -> ArtifactId {
    let mut hash = Fnv64::new();
    hash.update(path.display().to_string().as_bytes());
    hash.update(&[0]);
    hash.update(bytes);
    ArtifactId::new(non_zero_id(hash.finish() % 1_000_000_000))
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

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let mut hash = Fnv64::new();
    hash.update(bytes);
    format!("fnv64:{:016x}", hash.finish())
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

struct Fnv64(u64);

impl Fnv64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    const fn new() -> Self {
        Self(Self::OFFSET)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    const fn finish(&self) -> u64 {
        self.0
    }
}
