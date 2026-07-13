#![forbid(unsafe_code)]

use std::path::Path;

use maestria_domain::{ArtifactId, CardId, ChunkId, CreateCardInput};
use maestria_ports::{
    FileHandle, FileMetadata, ParsedArtifact, ParsedChunk, PortError, SourceSpan,
};

pub(crate) const ID_STRIDE: u64 = 1_000_003;
pub(crate) const CARD_OFFSET: u64 = 900_001;

pub fn chunk_id_for(artifact_id: ArtifactId, chunk_order: usize) -> Result<ChunkId, PortError> {
    if chunk_order as u64 >= ID_STRIDE {
        return Err(PortError::InvalidInput {
            message: format!("chunk order {chunk_order} exceeds parser id stride {ID_STRIDE}"),
        });
    }

    let id = artifact_id
        .value()
        .checked_mul(ID_STRIDE)
        .and_then(|value| value.checked_add(chunk_order as u64))
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| PortError::InvalidInput {
            message: format!(
                "artifact id {} cannot be expanded into deterministic chunk ids",
                artifact_id.value()
            ),
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

pub(crate) fn decode_utf8(bytes: Vec<u8>) -> Result<String, PortError> {
    if bytes.is_empty() {
        return Err(PortError::InvalidInput {
            message: "input file is empty".to_string(),
        });
    }

    String::from_utf8(bytes).map_err(|err| PortError::InvalidInput {
        message: format!("file bytes are not utf8: {err}"),
    })
}

pub(crate) fn parsed_artifact(
    artifact_id: ArtifactId,
    path: &Path,
    chunks_with_spans: Vec<(String, SourceSpan)>,
) -> Result<ParsedArtifact, PortError> {
    if chunks_with_spans.is_empty() {
        return Err(PortError::InvalidInput {
            message: "input file has no textual content".to_string(),
        });
    }

    let mut chunks = Vec::with_capacity(chunks_with_spans.len());
    for (order, (text, source_span)) in chunks_with_spans.into_iter().enumerate() {
        chunks.push(ParsedChunk {
            chunk_id: chunk_id_for(artifact_id, order)?,
            artifact_id,
            text,
            source_span,
        });
    }

    let card = summary_card_for(artifact_id, path, &chunks);
    Ok(ParsedArtifact {
        artifact_id,
        chunks,
        cards: vec![card],
    })
}

pub(crate) fn summary_card_for(
    artifact_id: ArtifactId,
    path: &Path,
    chunks: &[ParsedChunk],
) -> CreateCardInput {
    let first_line = chunks
        .first()
        .and_then(|chunk| chunk.text.lines().find(|line| !line.trim().is_empty()))
        .map(clean_summary_line)
        .filter(|line| !line.is_empty());
    let fallback_title = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_string(),
        None => "artifact".to_string(),
    };
    let title = match first_line {
        Some(line) => line,
        None => fallback_title,
    };
    let unit = if chunks.len() == 1 { "chunk" } else { "chunks" };

    CreateCardInput {
        card_id: card_id_for(artifact_id),
        artifact_id,
        title,
        body: format!(
            "Parsed {} textual {} from {}.",
            chunks.len(),
            unit,
            path.display()
        ),
    }
}

fn clean_summary_line(line: &str) -> String {
    let trimmed = line.trim().trim_start_matches('#').trim();
    trimmed.chars().take(96).collect()
}

pub(crate) fn paragraph_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let mut chunks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut para_start: Option<usize> = None;

    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            if let Some(start) = para_start.take() {
                let joined = current.join("\n").trim().to_string();
                current.clear();
                if !joined.is_empty() {
                    chunks.push((
                        joined,
                        SourceSpan::TextSpan {
                            start_line: start + 1,
                            end_line: line_idx,
                        },
                    ));
                }
            }
        } else {
            if para_start.is_none() {
                para_start = Some(line_idx);
            }
            current.push(line);
        }
    }
    if let Some(start) = para_start.take() {
        let joined = current.join("\n").trim().to_string();
        if !joined.is_empty() {
            let total_lines = text.lines().count();
            chunks.push((
                joined,
                SourceSpan::TextSpan {
                    start_line: start + 1,
                    end_line: total_lines,
                },
            ));
        }
    }

    chunks
}

pub(crate) fn ranges_from_starts(text: &str, starts: Vec<usize>) -> Vec<(String, SourceSpan)> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut chunks = Vec::new();

    if let Some(first_start) = starts.first().copied() {
        push_range(&mut chunks, &lines, 0, first_start);
    }

    for (position, start) in starts.iter().copied().enumerate() {
        let end = match starts.get(position + 1).copied() {
            Some(next_start) => next_start,
            None => lines.len(),
        };
        push_range(&mut chunks, &lines, start, end);
    }

    chunks
}

fn push_range(chunks: &mut Vec<(String, SourceSpan)>, lines: &[&str], start: usize, end: usize) {
    if start >= end {
        return;
    }

    let text = lines[start..end].join("\n").trim().to_string();
    if !text.is_empty() {
        chunks.push((
            text,
            SourceSpan::TextSpan {
                start_line: start + 1,
                end_line: end,
            },
        ));
    }
}
