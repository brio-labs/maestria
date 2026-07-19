use std::collections::BTreeSet;

use maestria_domain::{
    ArtifactId, ArtifactVersionId, ChunkId, ContentHash, CreateCardInput, StructureNode,
    StructureNodeId,
};

use super::traits::{FileHandle, FileMetadata, PortError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseContext {
    pub artifact_id: ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseStatus {
    Parsed,
    Unsupported,
    Failed,
    MetadataOnly,
    NeedsOcr,
    Quarantined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepresentationKind {
    Raw,
    Retrieval,
    Contextual,
    Summary,
    Visual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepresentation {
    pub kind: RepresentationKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTree {
    pub root_id: StructureNodeId,
    pub nodes: Vec<StructureNode>,
}

impl DocumentTree {
    pub fn new(root_id: StructureNodeId, nodes: Vec<StructureNode>) -> Result<Self, PortError> {
        let mut ids = BTreeSet::new();
        for node in &nodes {
            if !ids.insert(node.id) {
                return Err(PortError::InvalidInput {
                    message: "document tree contains duplicate node ids".to_owned(),
                });
            }
        }
        let roots: Vec<_> = nodes
            .iter()
            .filter(|node| node.parent_id.is_none())
            .collect();
        if roots.len() != 1 || roots[0].id != root_id {
            return Err(PortError::InvalidInput {
                message: "document tree must have one declared rooted node".to_owned(),
            });
        }
        for node in &nodes {
            if node.parent_id.is_some_and(|parent| !ids.contains(&parent))
                || node
                    .sibling_id
                    .is_some_and(|sibling| !ids.contains(&sibling))
            {
                return Err(PortError::InvalidInput {
                    message: "document tree contains a dangling link".to_owned(),
                });
            }
        }
        for node in &nodes {
            let mut current = node.id;
            let mut visited = BTreeSet::new();
            while let Some(parent) = nodes.iter().find(|candidate| candidate.id == current) {
                if !visited.insert(current) {
                    return Err(PortError::InvalidInput {
                        message: "document tree contains a parent cycle".to_owned(),
                    });
                }
                match parent.parent_id {
                    Some(next) => current = next,
                    None => break,
                }
            }
        }
        for node in &nodes {
            let mut current = node.id;
            let mut visited = BTreeSet::new();
            while let Some(current_node) = nodes.iter().find(|candidate| candidate.id == current) {
                if !visited.insert(current) {
                    return Err(PortError::InvalidInput {
                        message: "document tree contains a sibling cycle".to_owned(),
                    });
                }
                match current_node.sibling_id {
                    Some(next) => current = next,
                    None => break,
                }
            }
        }
        Ok(Self { root_id, nodes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpan {
    TextSpan {
        start_line: usize,
        end_line: usize,
    },
    PdfSpan {
        page: usize,
    },
    PdfRegion {
        page: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChunk {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub node_id: StructureNodeId,
    pub text: String,
    pub representations: Vec<ParsedRepresentation>,
    pub source_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCard {
    pub card: CreateCardInput,
    pub node_id: StructureNodeId,
    pub source_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArtifact {
    pub artifact_id: ArtifactId,
    pub artifact_version_id: ArtifactVersionId,
    pub content_hash: ContentHash,
    pub tree: DocumentTree,
    pub status: ParseStatus,
    pub chunks: Vec<ParsedChunk>,
    pub cards: Vec<ParsedCard>,
}

pub trait Parser: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, file: &FileMetadata) -> bool;
    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError>;
}
