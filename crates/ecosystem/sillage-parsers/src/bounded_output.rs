#![forbid(unsafe_code)]

use std::ops::Range;
use std::path::Path;

use sha2::{Digest, Sha256};
use sillage_domain::{ArtifactId, ContentHash, CreateCardInput};
use sillage_ports::{
    BoundedParseError, DocumentTree, FileHandleRef, ParseBudget, ParseContext, ParsedArtifact,
    ParsedCard, ParsedChunk, ParsedRepresentation, PortError,
};

use crate::artifact_builder::artifact_version_id_for;
use crate::bounded_markdown::{self, TextChunkRange};
use crate::tree_builder::domain_source_span;

const COPY_SEGMENT_BYTES: usize = 128;
const HASH_PREFIX: &str = "sha256:";
const HASH_OUTPUT_BYTES: usize = HASH_PREFIX.len() + 64;
const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

pub(crate) fn parsed_artifact_from_ranges_bounded(
    file: FileHandleRef<'_>,
    context: ParseContext,
    chunk_ranges: Vec<TextChunkRange>,
    parser_generation: &str,
    schema_generation: &str,
    language: Option<&str>,
    budget: &mut ParseBudget<'_>,
) -> Result<ParsedArtifact, BoundedParseError> {
    if chunk_ranges.is_empty() {
        return Err(PortError::invalid_input(
            "build parsed artifact",
            "input file has no textual content",
        )
        .into());
    }
    let artifact_id = context.artifact_id;
    let chunk_count = chunk_ranges.len();
    let node_count = chunk_count.checked_add(1).ok_or_else(|| {
        PortError::invalid_input("allocate bounded parse tree", "node count overflow")
    })?;
    let representation_count = chunk_count.checked_mul(2).ok_or_else(|| {
        PortError::invalid_input("allocate bounded chunks", "representation count overflow")
    })?;
    budget.charge_output(std::mem::size_of::<ParsedArtifact>())?;
    budget.charge_output(std::mem::size_of::<DocumentTree>())?;
    charge_array::<ParsedChunk>(chunk_count, budget)?;
    charge_array::<sillage_domain::StructureNode>(node_count, budget)?;
    charge_array::<ParsedRepresentation>(representation_count, budget)?;

    let (tree, chunks) = crate::tree_builder::build_tree_and_chunks_from_ranges_bounded(
        artifact_id,
        file.bytes,
        chunk_ranges,
        parser_generation,
        schema_generation,
        language,
        budget,
    )?;
    let first_chunk = chunks.first().ok_or_else(|| {
        PortError::invalid_input(
            "build parsed artifact card",
            "parsed artifact has no card evidence span",
        )
    })?;
    let card_source_span = first_chunk.source_span.clone();
    let mut card = summary_card_for_bounded(artifact_id, file.path, &chunks, budget)?;
    card.node_id = tree.root_id();
    card.source_span = domain_source_span(&card_source_span)?;
    let parsed_card = ParsedCard {
        card,
        node_id: tree.root_id(),
        source_span: card_source_span,
    };
    budget.charge_output(std::mem::size_of::<ParsedCard>())?;
    let cards = vec![parsed_card];

    let content_hash = content_hash_bounded(file.bytes, budget)?;
    let artifact_version_id = artifact_version_id_for(&content_hash)?;
    budget.checkpoint()?;
    Ok(ParsedArtifact {
        artifact_id,
        artifact_version_id,
        content_hash,
        tree,
        status: sillage_ports::ParseStatus::Parsed,
        chunks,
        cards,
    })
}

fn charge_array<T>(count: usize, budget: &mut ParseBudget<'_>) -> Result<(), BoundedParseError> {
    let bytes = std::mem::size_of::<T>().checked_mul(count).ok_or_else(|| {
        PortError::invalid_input("admit bounded parser output", "array size overflow")
    })?;
    budget.charge_output(bytes)?;
    Ok(())
}

pub(crate) fn copy_bytes_range_bounded(
    source: &[u8],
    range: Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<String, BoundedParseError> {
    if range.start > range.end || range.end > source.len() {
        return Err(PortError::invalid_input(
            "copy bounded parser text",
            "source range is outside the input",
        )
        .into());
    }
    budget.charge_output(range.len())?;
    budget.checkpoint()?;
    let mut output = String::new();
    output
        .try_reserve_exact(range.len())
        .map_err(|error| PortError::internal("reserve bounded parser text", error.to_string()))?;
    let mut offset = range.start;
    while offset < range.end {
        budget.checkpoint()?;
        let end = bounded_markdown::utf8_segment_end(source, offset, range.end, budget)?;
        let segment_len = (end - offset) as u64;
        budget.charge_work(segment_len)?;
        let segment = std::str::from_utf8(&source[offset..end]).map_err(|error| {
            PortError::invalid_input("copy bounded parser text", error.to_string())
        })?;
        budget.charge_work(segment_len)?;
        output.push_str(segment);
        offset = end;
    }
    Ok(output)
}

pub(crate) fn copy_string_bounded(
    source: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<String, BoundedParseError> {
    copy_bytes_range_bounded(source.as_bytes(), 0..source.len(), budget)
}

pub(crate) fn node_metadata_bounded(
    text: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<(sillage_domain::StructureNodeType, Vec<String>), BoundedParseError> {
    let bytes = text.as_bytes();
    let mut offset = 0;
    let first_line = bounded_markdown::next_source_line(bytes, &mut offset, 1, budget)?
        .map_or(0..0, |line| line.text);
    let first_line = bounded_markdown::trim_range(bytes, first_line, budget)?;
    let mut cursor = first_line.start;
    let mut heading_level = 0;
    while cursor < first_line.end {
        budget.charge_work(1)?;
        if bytes[cursor] != b'#' {
            break;
        }
        heading_level += 1;
        cursor += 1;
    }
    if (1..=6).contains(&heading_level) && cursor < first_line.end {
        let (next, _) = bounded_markdown::next_char(bytes, cursor, first_line.end, budget)?;
        if next.is_whitespace() {
            let label_range = bounded_markdown::trim_range(bytes, cursor..first_line.end, budget)?;
            let label = copy_bytes_range_bounded(bytes, label_range, budget)?;
            budget.charge_output(std::mem::size_of::<String>())?;
            let section_path = vec![label];
            return Ok((sillage_domain::StructureNodeType::Section, section_path));
        }
    }

    let first_line_len = first_line.end - first_line.start;
    if first_line_len >= 3 {
        budget.charge_work(3)?;
        if &bytes[first_line.start..first_line.start + 3] == b"```" {
            return Ok((sillage_domain::StructureNodeType::Code, Vec::new()));
        }
    }
    let mut cursor = first_line.start;
    while cursor < first_line.end {
        budget.charge_work(1)?;
        if bytes[cursor] == b'|' {
            return Ok((sillage_domain::StructureNodeType::Table, Vec::new()));
        }
        cursor += 1;
    }
    Ok((sillage_domain::StructureNodeType::Paragraph, Vec::new()))
}

pub(crate) fn summary_card_for_bounded(
    artifact_id: ArtifactId,
    path: &Path,
    chunks: &[ParsedChunk],
    budget: &mut ParseBudget<'_>,
) -> Result<CreateCardInput, BoundedParseError> {
    crate::bounded_summary::summary_card_for_bounded(artifact_id, path, chunks, budget)
}

fn content_hash_bounded(
    bytes: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<ContentHash, BoundedParseError> {
    budget.charge_output(HASH_OUTPUT_BYTES)?;
    let mut hasher = Sha256::new();
    for segment in bytes.chunks(COPY_SEGMENT_BYTES) {
        budget.checkpoint()?;
        budget.charge_work(segment.len() as u64)?;
        hasher.update(segment);
    }
    let digest = hasher.finalize();
    let mut output = String::with_capacity(HASH_OUTPUT_BYTES);
    budget.charge_work(HASH_PREFIX.len() as u64)?;
    output.push_str(HASH_PREFIX);
    for byte in digest {
        budget.charge_work(2)?;
        output.push(char::from(HEX_CHARS[(byte >> 4) as usize]));
        output.push(char::from(HEX_CHARS[(byte & 0x0f) as usize]));
    }
    budget.charge_work(HASH_OUTPUT_BYTES as u64)?;
    ContentHash::new(output).map_err(|error| {
        BoundedParseError::Parse(PortError::invalid_input(
            "invalid content hash",
            format!("{error:?}"),
        ))
    })
}
