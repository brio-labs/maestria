use maestria_domain::{
    ArtifactId, ContentRange, StructureNode, StructureNodeId, StructureNodeType,
};
use maestria_ports::{
    DocumentTree, ParsedChunk, ParsedRepresentation, PortError, RepresentationKind, SourceSpan,
};

use crate::chunking::{ID_STRIDE, chunk_id_for};

pub(crate) fn build_tree_and_chunks(
    artifact_id: ArtifactId,
    bytes: &[u8],
    chunks_with_spans: Vec<(String, SourceSpan)>,
    parser_generation: String,
    schema_generation: String,
    language: Option<String>,
) -> Result<(DocumentTree, Vec<ParsedChunk>), PortError> {
    let mut chunks = Vec::with_capacity(chunks_with_spans.len());
    let mut nodes = Vec::with_capacity(chunks_with_spans.len() + 1);
    let root_node_id = StructureNodeId::new(artifact_id.value().wrapping_mul(ID_STRIDE));
    nodes.push(StructureNode {
        id: root_node_id,
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange { start: 1, end: 1 },
        page: None,
        section_path: vec![],
        parser_generation: parser_generation.clone(),
        schema_generation: schema_generation.clone(),
        language: language.clone(),
    });
    for (order, (text, source_span)) in chunks_with_spans.into_iter().enumerate() {
        let chunk_id = chunk_id_for(artifact_id, order)?;
        let node_id = StructureNodeId::new(chunk_id.value());
        let content_range = match source_span {
            SourceSpan::TextSpan {
                start_line,
                end_line,
            } => ContentRange {
                start: start_line,
                end: end_line,
            },
            SourceSpan::PdfSpan { .. } | SourceSpan::PdfRegion { .. } => {
                ContentRange { start: 1, end: 1 }
            }
        };
        let page = match source_span {
            SourceSpan::PdfSpan { page } | SourceSpan::PdfRegion { page, .. } => Some(page as u32),
            _ => None,
        };
        let (node_type, section_path) = node_metadata(&text);
        nodes.push(StructureNode {
            id: node_id,
            parent_id: Some(root_node_id),
            sibling_id: None,
            node_type,
            source_range: content_range,
            page,
            section_path,
            parser_generation: parser_generation.clone(),
            schema_generation: schema_generation.clone(),
            language: language.clone(),
        });
        let raw_content = raw_content_for_span(bytes, &source_span, &text);
        chunks.push(ParsedChunk {
            chunk_id,
            artifact_id,
            node_id,
            text: text.clone(),
            representations: vec![
                ParsedRepresentation {
                    kind: RepresentationKind::Raw,
                    content: raw_content,
                },
                ParsedRepresentation {
                    kind: RepresentationKind::Retrieval,
                    content: text,
                },
            ],
            source_span,
        });
    }
    for index in 1..nodes.len() {
        if index < nodes.len() - 1 {
            nodes[index].sibling_id = Some(nodes[index + 1].id);
        }
    }
    let root_end = match chunks.last() {
        Some(chunk) => match &chunk.source_span {
            SourceSpan::TextSpan { end_line, .. } => *end_line,
            SourceSpan::PdfSpan { page } | SourceSpan::PdfRegion { page, .. } => *page,
        },
        None => 1,
    };
    if let Some(root) = nodes.first_mut() {
        root.source_range.end = root_end;
    }
    let tree = DocumentTree::new(root_node_id, nodes)?;
    Ok((tree, chunks))
}

fn node_metadata(text: &str) -> (StructureNodeType, Vec<String>) {
    let first_line = text.lines().next().map_or("", str::trim);
    let heading_level = first_line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if (1..=6).contains(&heading_level)
        && first_line
            .chars()
            .nth(heading_level)
            .is_some_and(char::is_whitespace)
    {
        return (
            StructureNodeType::Section,
            vec![first_line[heading_level..].trim().to_owned()],
        );
    }
    if first_line.starts_with("```") {
        return (StructureNodeType::Code, Vec::new());
    }
    if first_line.contains('|') {
        return (StructureNodeType::Table, Vec::new());
    }
    (StructureNodeType::Paragraph, Vec::new())
}

fn raw_content_for_span(bytes: &[u8], span: &SourceSpan, fallback: &str) -> String {
    let source = match std::str::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => return fallback.to_owned(),
    };
    let (start, end) = match span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => (*start_line, *end_line),
        SourceSpan::PdfSpan { .. } | SourceSpan::PdfRegion { .. } => {
            return fallback.to_owned();
        }
    };
    let lines: Vec<_> = source.split_inclusive('\n').collect();
    if start == 0 || end < start || end > lines.len() {
        return fallback.to_owned();
    }
    lines[start - 1..end].concat()
}
