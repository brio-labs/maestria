use crate::{
    DocumentTree, FileHandle, FileMetadata, ParseContext, ParseStatus, ParsedArtifact, ParsedChunk,
    ParsedRepresentation, Parser, PortError, RepresentationKind, SourceSpan,
};
use maestria_domain::ChunkId;
use maestria_domain::{
    ArtifactVersionId, ContentHash, ContentRange, StructureNode, StructureNodeId, StructureNodeType,
};

#[derive(Clone)]
pub struct InMemoryParser;

impl Default for InMemoryParser {
    fn default() -> Self {
        Self
    }
}

impl InMemoryParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for InMemoryParser {
    fn id(&self) -> &'static str {
        "in-memory-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        match file.extension.as_deref() {
            Some(ext) => matches!(ext, "md" | "txt" | "rs" | "toml"),
            None => false,
        }
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        if file.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "input file is empty".to_string(),
            });
        }

        let content_hash_str = maestria_domain::content_hash(&file.bytes);
        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
            message: format!("file bytes are not utf8: {err}"),
        })?;

        let root_node_id = StructureNodeId::new(context.artifact_id.value());
        let root_node = StructureNode {
            id: root_node_id,
            parent_id: None,
            sibling_id: None,
            node_type: StructureNodeType::Document,
            source_range: ContentRange {
                start: 0,
                end: text.len(),
            },
            page: None,
            section_path: vec![],
            parser_generation: "in-memory".to_string(),
            schema_generation: "v1".to_string(),
            language: None,
        };

        let tree = DocumentTree::new(root_node_id, vec![root_node]).map_err(|err| {
            PortError::InvalidInput {
                message: format!("invalid document tree: {:?}", err),
            }
        })?;

        let chunk = ParsedChunk {
            chunk_id: ChunkId::new(context.artifact_id.value()),
            artifact_id: context.artifact_id,
            node_id: root_node_id,
            text: text.clone(),
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
            source_span: SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1,
            },
        };
        let content_hash =
            ContentHash::new(content_hash_str).map_err(|err| PortError::InvalidInput {
                message: format!("invalid content hash: {:?}", err),
            })?;

        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id: ArtifactVersionId::new(context.artifact_id.value()),
            content_hash,
            tree,
            status: ParseStatus::Parsed,
            chunks: vec![chunk],
            cards: Vec::new(),
        })
    }
}
