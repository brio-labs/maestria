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
            return Err(PortError::InvalidInputContext {
                context: "input file is empty",
                source: "file bytes are empty".to_string(),
            });
        }

        let content_hash_str = maestria_domain::content_hash(&file.bytes);
        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInputContext {
            context: "input file is not UTF-8",
            source: err.to_string(),
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
            PortError::InvalidInputContext {
                context: "invalid document tree",
                source: format!("{err:?}"),
            }
        })?;

        let line_count = text.lines().count().max(1);

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
                end_line: line_count,
            },
        };
        let content_hash = ContentHash::new(content_hash_str.clone()).map_err(|err| {
            PortError::InvalidInputContext {
                context: "invalid content hash",
                source: format!("{err:?}"),
            }
        })?;

        let digest = content_hash_str.strip_prefix("sha256:").ok_or_else(|| {
            PortError::InvalidInputContext {
                context: "invalid content hash prefix",
                source: content_hash_str.clone(),
            }
        })?;
        let prefix = digest
            .get(..16)
            .ok_or_else(|| PortError::InvalidInputContext {
                context: "content hash too short for artifact version",
                source: content_hash_str.clone(),
            })?;
        let value =
            u64::from_str_radix(prefix, 16).map_err(|error| PortError::InvalidInputContext {
                context: "invalid content hash digest",
                source: error.to_string(),
            })?;
        let artifact_version_id = ArtifactVersionId::new(value);

        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            artifact_version_id,
            content_hash,
            tree,
            status: ParseStatus::Parsed,
            chunks: vec![chunk],
            cards: Vec::new(),
        })
    }
}
