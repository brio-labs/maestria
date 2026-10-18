#![forbid(unsafe_code)]

use sillage_domain::{ArtifactId, CardId, ChunkId};
use sillage_ports::{FileHandle, FileMetadata, PortError, SourceSpan};

pub(crate) const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

pub(crate) const CODE_EXTENSIONS: &[&str] = &["rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs"];
pub(crate) const DOC_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];

fn is_text_byte(byte: u8) -> bool {
    byte >= 0x20 || matches!(byte, b'\n' | b'\r' | b'\t' | 0x0b | 0x0c)
}

pub(crate) fn looks_like_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.contains(&0) {
        return false;
    }
    let suspicious = sample.iter().filter(|byte| !is_text_byte(**byte)).count();
    suspicious * 100 <= sample.len() * 5
}

pub(crate) const ID_STRIDE: u64 = 1_000_003;
pub(crate) const CARD_OFFSET: u64 = 900_001;

pub fn chunk_id_for(artifact_id: ArtifactId, chunk_order: usize) -> Result<ChunkId, PortError> {
    if chunk_order as u64 >= ID_STRIDE {
        return Err(PortError::InvalidInputContext {
            context: "chunk order exceeds parser id stride",
            source: chunk_order.to_string(),
        });
    }

    let id = artifact_id
        .value()
        .checked_mul(ID_STRIDE)
        .and_then(|value| value.checked_add(chunk_order as u64))
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            PortError::invalid_input(
                "artifact id cannot be expanded into deterministic chunk ids",
                artifact_id.value().to_string(),
            )
        })?;
    Ok(ChunkId::new(id))
}

pub fn card_id_for(artifact_id: ArtifactId) -> CardId {
    CardId::new(
        artifact_id
            .value()
            .wrapping_mul(ID_STRIDE)
            .wrapping_add(CARD_OFFSET),
    )
}

pub(crate) fn metadata_for_handle(file: &FileHandle) -> FileMetadata {
    FileMetadata {
        path: file.path.clone(),
        size: file.bytes.len(),
        extension: file
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase),
    }
}

pub(crate) fn extension_is(file: &FileMetadata, accepted: &[&str]) -> bool {
    file.extension.as_deref().is_some_and(|extension| {
        accepted
            .iter()
            .any(|accepted| extension.eq_ignore_ascii_case(accepted))
    })
}

pub(crate) fn decode_utf8(bytes: &[u8]) -> Result<&str, PortError> {
    if bytes.is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "decode file bytes",
            source: "input file is empty".to_string(),
        });
    }

    std::str::from_utf8(bytes)
        .map_err(|err| PortError::invalid_input("file bytes are not utf8", err.to_string()))
}

pub(crate) fn paragraph_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let mut chunks = Vec::new();
    let mut para_start_line: Option<usize> = None;
    let mut para_start_byte = 0usize;
    let mut last_non_empty_end_byte = 0usize;
    let mut total_lines = 0usize;
    let mut current_byte = 0usize;

    for (line_idx, line) in text.lines().enumerate() {
        total_lines = line_idx + 1;
        let line_len = line.len();
        if line.trim().is_empty() {
            if let Some(start_line) = para_start_line.take() {
                let slice = text[para_start_byte..last_non_empty_end_byte].trim();
                if !slice.is_empty() {
                    chunks.push((
                        slice.to_string(),
                        SourceSpan::TextSpan {
                            start_line: start_line + 1,
                            end_line: line_idx,
                        },
                    ));
                }
            }
        } else {
            if para_start_line.is_none() {
                para_start_line = Some(line_idx);
                para_start_byte = current_byte;
            }
            last_non_empty_end_byte = current_byte + line_len;
        }
        current_byte += line_len;
        if current_byte < text.len() && text.as_bytes()[current_byte] == b'\r' {
            current_byte += 1;
        }
        if current_byte < text.len() && text.as_bytes()[current_byte] == b'\n' {
            current_byte += 1;
        }
    }
    if let Some(start_line) = para_start_line.take() {
        let slice = text[para_start_byte..last_non_empty_end_byte].trim();
        if !slice.is_empty() {
            chunks.push((
                slice.to_string(),
                SourceSpan::TextSpan {
                    start_line: start_line + 1,
                    end_line: total_lines,
                },
            ));
        }
    }

    chunks
}

fn line_start_offsets(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    starts.push(0);
    for (i, byte) in text.as_bytes().iter().enumerate() {
        if *byte == b'\n' && i + 1 < text.len() {
            starts.push(i + 1);
        }
    }
    starts
}

pub(crate) fn ranges_from_starts(text: &str, starts: Vec<usize>) -> Vec<(String, SourceSpan)> {
    let line_starts = line_start_offsets(text);
    let mut chunks = Vec::new();

    if let Some(first_start) = starts.first().copied() {
        push_range(&mut chunks, text, &line_starts, 0, first_start);
    }

    for (position, start) in starts.iter().copied().enumerate() {
        let end = match starts.get(position + 1).copied() {
            Some(next_start) => next_start,
            None => line_starts.len(),
        };
        push_range(&mut chunks, text, &line_starts, start, end);
    }

    chunks
}

fn push_range(
    chunks: &mut Vec<(String, SourceSpan)>,
    text: &str,
    line_starts: &[usize],
    start: usize,
    end: usize,
) {
    if start >= end || start >= line_starts.len() {
        return;
    }

    let start_byte = line_starts[start];
    let end_byte = if end < line_starts.len() {
        line_starts[end]
    } else {
        text.len()
    };

    let slice = text[start_byte..end_byte].trim();
    if !slice.is_empty() {
        chunks.push((
            slice.to_string(),
            SourceSpan::TextSpan {
                start_line: start + 1,
                end_line: end,
            },
        ));
    }
}

pub(crate) fn structural_chunks(
    text: &str,
    is_pending: fn(&str) -> bool,
    is_start: fn(&str) -> bool,
    is_comment: fn(&str) -> bool,
) -> Vec<(String, SourceSpan)> {
    let mut starts = Vec::new();
    let mut pending: Option<usize> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if is_pending(trimmed) {
            pending.get_or_insert(index);
            continue;
        }
        if is_start(trimmed) {
            let start = match pending.take() {
                Some(start) => start,
                None => index,
            };
            starts.push(start);
        } else if !trimmed.is_empty() && !is_comment(trimmed) {
            pending = None;
        }
    }
    if starts.is_empty() {
        return paragraph_chunks(text);
    }
    starts.sort_unstable();
    starts.dedup();
    ranges_from_starts(text, starts)
}

#[macro_export]
macro_rules! text_parser {
    (
        $name:ident, $id:expr, $supports:expr, $parser_gen:expr,
        $schema_gen:expr, $lang:expr, $chunk_fn:expr
    ) => {
        $crate::text_parser!(
            $name,
            $id,
            $supports,
            $parser_gen,
            $schema_gen,
            $lang,
            $chunk_fn,
            |_: &[u8]| Ok(())
        );
    };
    (
        $name:ident, $id:expr, $supports:expr, $parser_gen:expr,
        $schema_gen:expr, $lang:expr, $chunk_fn:expr, $precheck:expr
    ) => {
        #[derive(Debug, Clone, Copy, Default)]
        pub struct $name;
        impl $name {
            pub const fn new() -> Self {
                Self
            }
        }
        impl sillage_ports::Parser for $name {
            fn id(&self) -> &'static str {
                $id
            }
            fn supports(&self, file: &sillage_ports::FileMetadata) -> bool {
                $supports(file)
            }
            fn parse(
                &self,
                file: sillage_ports::FileHandle,
                context: sillage_ports::ParseContext,
            ) -> Result<sillage_ports::ParsedArtifact, sillage_ports::PortError> {
                $precheck(&file.bytes)?;
                let text = $crate::chunking::decode_utf8(&file.bytes)?;
                let chunks = $chunk_fn(&text);
                $crate::artifact_builder::parsed_artifact(
                    context.artifact_id,
                    &file.path,
                    &file.bytes,
                    chunks,
                    $parser_gen.to_string(),
                    $schema_gen.to_string(),
                    Some($lang.to_string()),
                )
            }
        }
    };
}
