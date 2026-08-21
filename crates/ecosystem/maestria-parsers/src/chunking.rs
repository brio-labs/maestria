#![forbid(unsafe_code)]

use std::path::Path;

use maestria_domain::{
    ArtifactId, ArtifactVersionId, CardId, ChunkId, ContentHash, CreateCardInput,
    SourceSpan as DomainSourceSpan, SourceSpanError, StructureNodeId,
};
use maestria_ports::{
    FileHandle, FileMetadata, ParsedArtifact, ParsedCard, ParsedChunk, PortError, SourceSpan,
};
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

pub(crate) fn decode_utf8(bytes: Vec<u8>) -> Result<String, PortError> {
    if bytes.is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "decode file bytes",
            source: "input file is empty".to_string(),
        });
    }

    String::from_utf8(bytes)
        .map_err(|err| PortError::invalid_input("file bytes are not utf8", err.to_string()))
}

pub(crate) fn parsed_artifact(
    artifact_id: ArtifactId,
    path: &Path,
    bytes: &[u8],
    chunks_with_spans: Vec<(String, SourceSpan)>,
    parser_generation: String,
    schema_generation: String,
    language: Option<String>,
) -> Result<ParsedArtifact, PortError> {
    if chunks_with_spans.is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "build parsed artifact",
            source: "input file has no textual content".to_string(),
        });
    }

    let (tree, chunks) = crate::tree_builder::build_tree_and_chunks(
        artifact_id,
        bytes,
        chunks_with_spans,
        parser_generation,
        schema_generation,
        language,
    )?;
    let card_source_span = match chunks.first() {
        Some(chunk) => chunk.source_span.clone(),
        None => {
            return Err(PortError::InvalidInputContext {
                context: "build parsed artifact card",
                source: "parsed artifact has no card evidence span".to_string(),
            });
        }
    };
    let mut card = summary_card_for(artifact_id, path, &chunks)?;
    card.node_id = tree.root_id();
    card.source_span = domain_source_span(&card_source_span)?;
    let parsed_card = ParsedCard {
        card,
        node_id: tree.root_id(),
        source_span: card_source_span,
    };
    let hash_string = maestria_domain::content_hash(bytes);
    let content_hash = maestria_domain::ContentHash::new(hash_string).map_err(|e| {
        PortError::InvalidInputContext {
            context: "invalid content hash",
            source: format!("{e:?}"),
        }
    })?;
    let artifact_version_id = artifact_version_id_for(&content_hash)?;
    Ok(ParsedArtifact {
        artifact_id,
        artifact_version_id,
        content_hash,
        tree,
        status: maestria_ports::ParseStatus::Parsed,
        chunks,
        cards: vec![parsed_card],
    })
}

/// Derives the content-addressed artifact version identity from a validated
/// content hash.
///
/// Delegates to [`ContentHash::version_id`] — the single domain-owned
/// derivation (R28) — mapping the compatibility error to the parser error
/// type.
pub(crate) fn artifact_version_id_for(
    content_hash: &ContentHash,
) -> Result<ArtifactVersionId, PortError> {
    content_hash
        .version_id()
        .map_err(|error| PortError::InvalidInputContext {
            context: "derive artifact version identity",
            source: format!("{error}"),
        })
}

pub(crate) fn summary_card_for(
    artifact_id: ArtifactId,
    path: &Path,
    chunks: &[ParsedChunk],
) -> Result<CreateCardInput, PortError> {
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

    let (node_id, source_span) = match chunks.first() {
        Some(chunk) => (chunk.node_id, domain_source_span(&chunk.source_span)?),
        None => (
            StructureNodeId::new(artifact_id.value()),
            DomainSourceSpan::text_span(1, 1).map_err(span_error)?,
        ),
    };
    Ok(CreateCardInput {
        card_id: card_id_for(artifact_id),
        artifact_id,
        node_id,
        source_span,
        title,
        body: format!(
            "Parsed {} textual {} from {}.",
            chunks.len(),
            unit,
            path.display()
        ),
        security: None,
    })
}

fn clean_summary_line(line: &str) -> String {
    let trimmed = line.trim().trim_start_matches('#').trim();
    trimmed.chars().take(96).collect()
}

pub(crate) fn domain_source_span(span: &SourceSpan) -> Result<DomainSourceSpan, PortError> {
    match span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => DomainSourceSpan::text_span(*start_line, *end_line).map_err(span_error),
        SourceSpan::PdfSpan { page } => DomainSourceSpan::pdf_span(*page).map_err(span_error),
        SourceSpan::PdfRegion {
            page,
            x,
            y,
            width,
            height,
        } => DomainSourceSpan::pdf_region(*page, *x, *y, *width, *height).map_err(span_error),
    }
}

fn span_error(error: SourceSpanError) -> PortError {
    PortError::InvalidInputContext {
        context: "convert chunk source span",
        source: error.to_string(),
    }
}

pub(crate) fn paragraph_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let mut chunks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut para_start: Option<usize> = None;
    let mut total_lines = 0usize;

    for (line_idx, line) in text.lines().enumerate() {
        total_lines = line_idx + 1;
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
        impl maestria_ports::Parser for $name {
            fn id(&self) -> &'static str {
                $id
            }
            fn supports(&self, file: &maestria_ports::FileMetadata) -> bool {
                $supports(file)
            }
            fn parse(
                &self,
                file: maestria_ports::FileHandle,
                context: maestria_ports::ParseContext,
            ) -> Result<maestria_ports::ParsedArtifact, maestria_ports::PortError> {
                $precheck(&file.bytes)?;
                let text = $crate::chunking::decode_utf8(file.bytes.clone())?;
                let chunks = $chunk_fn(&text);
                $crate::chunking::parsed_artifact(
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
