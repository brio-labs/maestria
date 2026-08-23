use crate::ids::{ArtifactId, ChunkId, StructureNodeId};
use crate::provenance::{ParsedRepresentation, SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub id: ChunkId,
    pub artifact_id: ArtifactId,
    pub node_id: StructureNodeId,
    pub source_span: SourceSpan,
    pub representations: Vec<ParsedRepresentation>,
    /// Identity of the representation set (`sha256:` over canonical JSON).
    /// Survives storage round-trips that drop representation contents, so
    /// restart recovery can compare registrations without both sides
    /// holding full contents.
    pub representations_digest: String,
    pub order: u32,
    pub text: String,
}

impl Chunk {
    pub(crate) fn new(
        input: &crate::inputs::RegisterChunkInput,
        representations_digest: String,
    ) -> Self {
        Self {
            id: input.chunk_id,
            artifact_id: input.artifact_id,
            node_id: input.node_id,
            source_span: input.source_span,
            representations: input.representations.clone(),
            representations_digest,
            order: input.order,
            text: input.text.clone(),
        }
    }
}
