use crate::pdf_layout::PdfPageLayout;
use maestria_domain::{ContentRange, StructureNode, StructureNodeId, StructureNodeType};
use maestria_ports::{
    DocumentTree, ParsedCard, ParsedChunk, ParsedRepresentation, PortError, RepresentationKind,
    SourceSpan,
};

const PARSER_GENERATION: &str = "pdf-parser-3";
const SCHEMA_GENERATION: &str = "2";
const PAGE_NODE_OFFSET: u64 = 950_000;

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
    let max_page = match pages.iter().map(|page| page.page).max() {
        Some(page) => page,
        None => {
            let _ = ();
            1
        }
    };
    Ok(StructureNode {
        id: StructureNodeId::new(
            artifact_id
                .value()
                .checked_mul(crate::chunking::ID_STRIDE)
                .ok_or_else(|| {
                    PortError::invalid_input(
                        "allocate PDF root node id",
                        "artifact id cannot be expanded into PDF node ids",
                    )
                })?,
        ),
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange::new(1, max_page as usize).map_err(|error| {
            PortError::InvalidInputContext {
                context: "allocate PDF root node content range",
                source: error.to_string(),
            }
        })?,
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
            .ok_or_else(|| {
                PortError::invalid_input("allocate PDF page node id", "PDF page node id overflow")
            })?,
    );
    Ok(StructureNode {
        id: page_node_id,
        parent_id: Some(root_id),
        sibling_id: None,
        node_type: StructureNodeType::Section,
        source_range: ContentRange::new(page.page as usize, page.page as usize).map_err(
            |error| PortError::InvalidInputContext {
                context: "allocate PDF page node content range",
                source: error.to_string(),
            },
        )?,
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
            source_range: ContentRange::new(page.page as usize, page.page as usize).map_err(
                |error| PortError::InvalidInputContext {
                    context: "allocate PDF text chunk node content range",
                    source: error.to_string(),
                },
            )?,
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
            source_range: ContentRange::new(page.page as usize, page.page as usize).map_err(
                |error| PortError::InvalidInputContext {
                    context: "allocate PDF region chunk node content range",
                    source: error.to_string(),
                },
            )?,
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

pub(crate) fn build_tree_and_chunks(
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

pub(crate) fn parsed_card_for(
    artifact_id: maestria_domain::ArtifactId,
    path: &std::path::Path,
    parsed_chunks: &[ParsedChunk],
    root_id: StructureNodeId,
) -> Result<ParsedCard, PortError> {
    let mut card = crate::chunking::summary_card_for(artifact_id, path, parsed_chunks)?;
    let card_source_span = match parsed_chunks.first() {
        Some(chunk) => chunk.source_span.clone(),
        None => {
            return Err(PortError::InvalidInputContext {
                context: "build parsed PDF card",
                source: "parsed PDF has no card evidence span".to_string(),
            });
        }
    };
    card.node_id = root_id;
    card.source_span = crate::chunking::domain_source_span(&card_source_span)?;
    Ok(ParsedCard {
        card,
        node_id: root_id,
        source_span: card_source_span,
    })
}
