#![forbid(unsafe_code)]

use maestria_domain::{
    ContentHash, ContentRange, StructureNode, StructureNodeId, StructureNodeType, content_hash,
};
use maestria_ports::{
    DocumentTree, FileHandle, FileMetadata, ParseContext, ParseStatus, ParsedArtifact, ParsedCard,
    ParsedChunk, ParsedRepresentation, Parser, PortError, RepresentationKind, SourceSpan,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct PdfParser;

impl PdfParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for PdfParser {
    fn id(&self) -> &'static str {
        "pdf-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.extension
            .as_deref()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let doc = lopdf::Document::load_mem(&file.bytes).map_err(|e| PortError::InvalidInput {
            message: format!("PDF parse error: {e}"),
        })?;
        let chunks = extract_page_chunks(&doc)?;
        let (tree, parsed_chunks, root_id) = build_tree_and_chunks(context.artifact_id, chunks)?;
        let parsed_card =
            parsed_card_for(context.artifact_id, &file.path, &parsed_chunks, root_id)?;

        let hash_string = content_hash(&file.bytes);
        let content_hash =
            ContentHash::new(hash_string.clone()).map_err(|e| PortError::InvalidInput {
                message: e.to_string(),
            })?;
        let artifact_version_id =
            crate::chunking::artifact_version_id_for(context.artifact_id, &hash_string);
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id,
            content_hash,
            tree,
            status: ParseStatus::Parsed,
            chunks: parsed_chunks,
            cards: vec![parsed_card],
        })
    }
}

fn extract_page_chunks(doc: &lopdf::Document) -> Result<Vec<(String, u32)>, PortError> {
    let page_nums: Vec<_> = doc.get_pages().keys().copied().collect();
    let mut chunks = Vec::new();
    for page_num in page_nums {
        let text = doc
            .extract_text(&[page_num])
            .map_err(|error| PortError::InvalidInput {
                message: format!("PDF page {page_num} text extraction failed: {error}"),
            })?;
        let trimmed = text.trim().to_string();
        if !trimmed.is_empty() {
            chunks.push((trimmed, page_num));
        }
    }
    if chunks.is_empty() {
        return Err(PortError::InvalidInput {
            message: "PDF has no extractable text".to_string(),
        });
    }
    Ok(chunks)
}

fn build_tree_and_chunks(
    artifact_id: maestria_domain::ArtifactId,
    chunks: Vec<(String, u32)>,
) -> Result<(DocumentTree, Vec<ParsedChunk>, StructureNodeId), PortError> {
    let root_id =
        StructureNodeId::new(artifact_id.value().wrapping_mul(crate::chunking::ID_STRIDE));
    let max_page = chunks
        .iter()
        .map(|(_, page)| *page)
        .max()
        .map_or(1, |page| page);
    let root_node = StructureNode {
        id: root_id,
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange {
            start: 1,
            end: max_page as usize,
        },
        page: None,
        section_path: vec![],
        parser_generation: "pdf-parser-1".to_string(),
        schema_generation: "1".to_string(),
        language: None,
    };
    let mut nodes = vec![root_node];
    let mut parsed_chunks = Vec::with_capacity(chunks.len());
    for (order, (text, page_num)) in chunks.into_iter().enumerate() {
        let chunk_id = crate::chunking::chunk_id_for(artifact_id, order)?;
        let node_id = StructureNodeId::new(chunk_id.value());
        nodes.push(StructureNode {
            id: node_id,
            parent_id: Some(root_id),
            sibling_id: None,
            node_type: StructureNodeType::Paragraph,
            source_range: ContentRange {
                start: page_num as usize,
                end: page_num as usize,
            },
            page: Some(page_num),
            section_path: vec![],
            parser_generation: "pdf-parser-1".to_string(),
            schema_generation: "1".to_string(),
            language: None,
        });
        parsed_chunks.push(ParsedChunk {
            chunk_id,
            artifact_id,
            node_id,
            representations: vec![
                ParsedRepresentation {
                    kind: RepresentationKind::Raw,
                    content: text.clone(),
                },
                ParsedRepresentation {
                    kind: RepresentationKind::Retrieval,
                    content: text.clone(),
                },
            ],
            text,
            source_span: SourceSpan::PdfSpan {
                page: page_num as usize,
            },
        });
    }
    for index in 1..nodes.len().saturating_sub(1) {
        nodes[index].sibling_id = Some(nodes[index + 1].id);
    }
    let tree = DocumentTree::new(root_id, nodes)?;
    Ok((tree, parsed_chunks, root_id))
}

fn parsed_card_for(
    artifact_id: maestria_domain::ArtifactId,
    path: &std::path::Path,
    parsed_chunks: &[ParsedChunk],
    root_id: StructureNodeId,
) -> Result<ParsedCard, PortError> {
    let mut card = crate::chunking::summary_card_for(artifact_id, path, parsed_chunks);
    let card_source_span = match parsed_chunks.first() {
        Some(chunk) => chunk.source_span.clone(),
        None => {
            return Err(PortError::InvalidInput {
                message: "parsed PDF has no card evidence span".to_string(),
            });
        }
    };
    card.node_id = root_id;
    card.source_span = crate::chunking::domain_source_span(&card_source_span);
    Ok(ParsedCard {
        card,
        node_id: root_id,
        source_span: card_source_span,
    })
}
