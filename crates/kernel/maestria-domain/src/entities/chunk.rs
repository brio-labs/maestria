use crate::ids::{ArtifactId, ChunkId, StructureNodeId};
use crate::provenance::{ParsedRepresentation, SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: ChunkId,
    pub artifact_id: ArtifactId,
    pub node_id: StructureNodeId,
    pub source_span: SourceSpan,
    pub representations: Vec<ParsedRepresentation>,
    pub order: u32,
    pub text: String,
}

impl Chunk {
    pub(crate) fn new(
        id: ChunkId,
        artifact_id: ArtifactId,
        node_id: StructureNodeId,
        source_span: SourceSpan,
        representations: Vec<ParsedRepresentation>,
        order: u32,
        text: String,
    ) -> Self {
        Self {
            id,
            artifact_id,
            node_id,
            source_span,
            representations,
            order,
            text,
        }
    }
}
