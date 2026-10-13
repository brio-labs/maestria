#![forbid(unsafe_code)]

use std::path::Path;

use sillage_domain::{
    ArtifactId, ArtifactVersionId, ContentHash, CreateCardInput, SourceSpan as DomainSourceSpan,
    StructureNodeId,
};
use sillage_ports::{ParsedArtifact, ParsedCard, ParsedChunk, PortError, SourceSpan};

use crate::chunking::card_id_for;
use crate::tree_builder::{domain_source_span, span_error};

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
    let hash_string = sillage_domain::content_hash(bytes);
    let content_hash = sillage_domain::ContentHash::new(hash_string).map_err(|e| {
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
        status: sillage_ports::ParseStatus::Parsed,
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
