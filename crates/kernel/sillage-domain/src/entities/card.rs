use crate::ids::{ArtifactId, CardId, StructureNodeId};
use crate::provenance::SourceSpan;
use crate::security::SecurityMetadata;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    pub artifact_id: ArtifactId,
    pub node_id: StructureNodeId,
    pub source_span: SourceSpan,
    pub title: String,
    pub body: String,
    pub security: SecurityMetadata,
}

impl Card {
    pub(crate) fn new(
        id: CardId,
        artifact_id: ArtifactId,
        node_id: StructureNodeId,
        source_span: SourceSpan,
        title: String,
        body: String,
        security: SecurityMetadata,
    ) -> Self {
        Self {
            id,
            artifact_id,
            node_id,
            source_span,
            title,
            body,
            security,
        }
    }
}
