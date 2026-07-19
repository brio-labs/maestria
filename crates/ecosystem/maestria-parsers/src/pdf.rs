#![forbid(unsafe_code)]

use crate::pdf_layout::{PdfPageLayout, extract_page_layouts};
use maestria_domain::{
    ContentHash, ContentRange, StructureNode, StructureNodeId, StructureNodeType, content_hash,
};
use maestria_ports::{
    DocumentTree, FileHandle, FileMetadata, ParseContext, ParseStatus, ParsedArtifact, ParsedCard,
    ParsedChunk, ParsedRepresentation, Parser, PortError, RepresentationKind, SourceSpan,
};

const PARSER_GENERATION: &str = "pdf-parser-2";
const SCHEMA_GENERATION: &str = "2";
const PAGE_NODE_OFFSET: u64 = 950_000;

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
        let doc =
            lopdf::Document::load_mem(&file.bytes).map_err(|error| PortError::InvalidInput {
                message: format!("PDF parse error: {error}"),
            })?;
        let pages = extract_page_layouts(&doc)?;
        if pages.is_empty() {
            return Err(PortError::InvalidInput {
                message: "PDF has no pages".to_string(),
            });
        }

        let has_text = pages.iter().any(|page| !page.text.is_empty());
        let needs_ocr = pages.iter().any(|page| page.needs_ocr);
        let (tree, parsed_chunks, root_id) = build_tree_and_chunks(context.artifact_id, &pages)?;
        let parsed_cards = if parsed_chunks.iter().any(|chunk| !chunk.text.is_empty()) {
            vec![parsed_card_for(
                context.artifact_id,
                &file.path,
                &parsed_chunks,
                root_id,
            )?]
        } else {
            Vec::new()
        };

        let hash_string = content_hash(&file.bytes);
        let content_hash =
            ContentHash::new(hash_string.clone()).map_err(|error| PortError::InvalidInput {
                message: error.to_string(),
            })?;
        let artifact_version_id =
            crate::chunking::artifact_version_id_for(context.artifact_id, &hash_string);
        let status = if needs_ocr || !has_text {
            ParseStatus::NeedsOcr
        } else {
            ParseStatus::Parsed
        };
        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id,
            content_hash,
            tree,
            status,
            chunks: parsed_chunks,
            cards: parsed_cards,
        })
    }
}

fn text_layout_items(text: &str) -> Vec<(String, StructureNodeType)> {
    let mut items = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let node_type = if line.starts_with("Figure")
                || line.starts_with("Fig.")
                || line.starts_with("Caption:")
            {
                StructureNodeType::FigureCaption
            } else if line.starts_with("Table") {
                StructureNodeType::TableRow
            } else if line.contains('|') {
                StructureNodeType::TableCell
            } else if line.starts_with("Equation")
                || line.contains(" = ")
                || line.contains("^{")
                || line.contains("∑")
            {
                StructureNodeType::Formula
            } else {
                StructureNodeType::Paragraph
            };
            (line.to_string(), node_type)
        })
        .collect::<Vec<_>>();
    if items.is_empty() && !text.trim().is_empty() {
        items.push((text.trim().to_string(), StructureNodeType::Paragraph));
    }
    items
}

fn root_node(
    artifact_id: maestria_domain::ArtifactId,
    pages: &[PdfPageLayout],
) -> Result<StructureNode, PortError> {
    let max_page = pages
        .iter()
        .map(|page| page.page)
        .max()
        .map_or(1, |page| page);
    Ok(StructureNode {
        id: StructureNodeId::new(
            artifact_id
                .value()
                .checked_mul(crate::chunking::ID_STRIDE)
                .ok_or_else(|| PortError::InvalidInput {
                    message: "artifact id cannot be expanded into PDF node ids".to_string(),
                })?,
        ),
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange {
            start: 1,
            end: max_page as usize,
        },
        page: None,
        section_path: vec![],
        parser_generation: PARSER_GENERATION.to_string(),
        schema_generation: SCHEMA_GENERATION.to_string(),
        language: None,
    })
}

fn page_node(
    root_id: StructureNodeId,
    page_order: usize,
    page: &PdfPageLayout,
) -> Result<StructureNode, PortError> {
    let page_node_id = StructureNodeId::new(
        root_id
            .value()
            .checked_add(PAGE_NODE_OFFSET)
            .and_then(|value| value.checked_add(page_order as u64))
            .ok_or_else(|| PortError::InvalidInput {
                message: "PDF page node id overflow".to_string(),
            })?,
    );
    Ok(StructureNode {
        id: page_node_id,
        parent_id: Some(root_id),
        sibling_id: None,
        node_type: StructureNodeType::Section,
        source_range: ContentRange {
            start: page.page as usize,
            end: page.page as usize,
        },
        page: Some(page.page),
        section_path: vec![format!("Page {}", page.page)],
        parser_generation: PARSER_GENERATION.to_string(),
        schema_generation: SCHEMA_GENERATION.to_string(),
        language: None,
    })
}

fn append_text_chunks(
    artifact_id: maestria_domain::ArtifactId,
    page: &PdfPageLayout,
    page_node_id: StructureNodeId,
    nodes: &mut Vec<StructureNode>,
    parsed_chunks: &mut Vec<ParsedChunk>,
) -> Result<Vec<StructureNodeId>, PortError> {
    let mut child_ids = Vec::new();
    for (text, node_type) in text_layout_items(&page.text) {
        let chunk_id = crate::chunking::chunk_id_for(artifact_id, parsed_chunks.len())?;
        let node_id = StructureNodeId::new(chunk_id.value());
        child_ids.push(node_id);
        nodes.push(StructureNode {
            id: node_id,
            parent_id: Some(page_node_id),
            sibling_id: None,
            node_type,
            source_range: ContentRange {
                start: page.page as usize,
                end: page.page as usize,
            },
            page: Some(page.page),
            section_path: vec![format!("Page {}", page.page)],
            parser_generation: PARSER_GENERATION.to_string(),
            schema_generation: SCHEMA_GENERATION.to_string(),
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
                page: page.page as usize,
            },
        });
    }
    Ok(child_ids)
}

fn append_region_chunks(
    artifact_id: maestria_domain::ArtifactId,
    page: &PdfPageLayout,
    page_node_id: StructureNodeId,
    nodes: &mut Vec<StructureNode>,
    parsed_chunks: &mut Vec<ParsedChunk>,
) -> Result<Vec<StructureNodeId>, PortError> {
    let mut child_ids = Vec::new();
    for region in &page.regions {
        let chunk_id = crate::chunking::chunk_id_for(artifact_id, parsed_chunks.len())?;
        let node_id = StructureNodeId::new(chunk_id.value());
        child_ids.push(node_id);
        nodes.push(StructureNode {
            id: node_id,
            parent_id: Some(page_node_id),
            sibling_id: None,
            node_type: region.node_type.clone(),
            source_range: ContentRange {
                start: page.page as usize,
                end: page.page as usize,
            },
            page: Some(page.page),
            section_path: vec![format!("Page {}", page.page)],
            parser_generation: PARSER_GENERATION.to_string(),
            schema_generation: SCHEMA_GENERATION.to_string(),
            language: None,
        });
        parsed_chunks.push(ParsedChunk {
            chunk_id,
            artifact_id,
            node_id,
            representations: vec![ParsedRepresentation {
                kind: RepresentationKind::Visual,
                content: region.label.clone(),
            }],
            text: String::new(),
            source_span: SourceSpan::PdfRegion {
                page: page.page as usize,
                x: region.x,
                y: region.y,
                width: region.width,
                height: region.height,
            },
        });
    }
    Ok(child_ids)
}

fn link_siblings(nodes: &mut [StructureNode], sibling_ids: &[StructureNodeId]) {
    for pair in sibling_ids.windows(2) {
        if let [current, next] = pair
            && let Some(node) = nodes.iter_mut().find(|node| node.id == *current)
        {
            node.sibling_id = Some(*next);
        }
    }
}

fn build_tree_and_chunks(
    artifact_id: maestria_domain::ArtifactId,
    pages: &[PdfPageLayout],
) -> Result<(DocumentTree, Vec<ParsedChunk>, StructureNodeId), PortError> {
    let root = root_node(artifact_id, pages)?;
    let root_id = root.id;
    let mut nodes = vec![root];
    let mut parsed_chunks = Vec::new();
    let mut page_node_ids = Vec::new();
    for (page_order, page) in pages.iter().enumerate() {
        let page_node = page_node(root_id, page_order, page)?;
        let page_node_id = page_node.id;
        page_node_ids.push(page_node_id);
        nodes.push(page_node);
        let mut child_ids = append_text_chunks(
            artifact_id,
            page,
            page_node_id,
            &mut nodes,
            &mut parsed_chunks,
        )?;
        child_ids.extend(append_region_chunks(
            artifact_id,
            page,
            page_node_id,
            &mut nodes,
            &mut parsed_chunks,
        )?);
        link_siblings(&mut nodes, &child_ids);
    }
    link_siblings(&mut nodes, &page_node_ids);
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
