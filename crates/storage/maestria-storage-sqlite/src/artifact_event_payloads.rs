use super::event_payloads::StoredEventPayload;
use maestria_domain::{
    ArtifactId, ArtifactVersionId, BlobId, ChunkId, DomainEvent, StructureNodeId,
};

impl StoredEventPayload {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn try_from_domain_artifact(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::ArtifactRegistered {
                artifact_id,
                title,
                security,
            } => Some(Self::ArtifactRegistered {
                artifact_id: artifact_id.value(),
                title: title.clone(),
                security: security.clone(),
            }),
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                node_id,
                source_span,
                representations,
                order,
                text,
            } => Some(Self::ChunkRegistered {
                chunk_id: chunk_id.value(),
                artifact_id: artifact_id.value(),
                node_id: node_id.value(),
                source_span: (*source_span).into(),
                representations: representations.iter().cloned().map(Into::into).collect(),
                order: *order,
                text: text.clone(),
            }),
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                node_id,
                source_span,
                title,
                body,
                security,
            } => Some(Self::CardCreated {
                card_id: card_id.value(),
                artifact_id: artifact_id.value(),
                node_id: node_id.value(),
                source_span: (*source_span).into(),
                title: title.clone(),
                body: body.clone(),
                security: security.clone(),
            }),
            DomainEvent::ParserStarted {
                artifact_id,
                title,
                source_path,
                content_hash,
                blob_id,
            } => Some(Self::ParserStarted {
                artifact_id: artifact_id.value(),
                title: title.clone(),
                source_path: source_path.clone(),
                content_hash: content_hash.clone(),
                blob_id: blob_id.value(),
            }),
            DomainEvent::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id,
                content_hash,
                root_id,
                nodes,
            } => Some(Self::DocumentTreeCaptured {
                artifact_id: artifact_id.value(),
                artifact_version_id: artifact_version_id.value(),
                content_hash: content_hash.clone(),
                root_id: root_id.value(),
                nodes: nodes.clone(),
            }),
            DomainEvent::ArtifactParsed {
                artifact_id,
                status,
                chunks_added,
            } => Some(Self::ArtifactParsed {
                artifact_id: artifact_id.value(),
                status: (*status).into(),
                chunks_added: *chunks_added,
            }),
            DomainEvent::SearchCompleted {
                artifact_id,
                cards_added,
            } => Some(Self::SearchCompleted {
                artifact_id: artifact_id.value(),
                cards_added: *cards_added,
            }),
            DomainEvent::PendingIndex {
                artifact_id,
                content_hash,
            } => Some(Self::PendingIndex {
                artifact_id: artifact_id.value(),
                content_hash: content_hash.clone(),
            }),
            DomainEvent::FullTextIndexed {
                artifact_id,
                chunk_id,
            } => Some(Self::FullTextIndexed {
                artifact_id: artifact_id.value(),
                chunk_id: chunk_id.value(),
            }),
            DomainEvent::ArtifactIndexed { artifact_id } => Some(Self::ArtifactIndexed {
                artifact_id: artifact_id.value(),
            }),
            _ => None,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn try_into_domain_artifact(self) -> Result<DomainEvent, Box<Self>> {
        match self {
            Self::ArtifactRegistered {
                artifact_id,
                title,
                security,
            } => Ok(DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(artifact_id),
                title,
                security,
            }),
            Self::ChunkRegistered {
                chunk_id,
                artifact_id,
                node_id,
                source_span,
                representations,
                order,
                text,
            } => Ok(DomainEvent::ChunkRegistered {
                chunk_id: ChunkId::new(chunk_id),
                artifact_id: ArtifactId::new(artifact_id),
                node_id: StructureNodeId::new(node_id),
                source_span: source_span.into(),
                representations: representations.into_iter().map(Into::into).collect(),
                order,
                text,
            }),
            Self::CardCreated {
                card_id,
                artifact_id,
                node_id,
                source_span,
                title,
                body,
                security,
            } => Ok(DomainEvent::CardCreated {
                card_id: maestria_domain::CardId::new(card_id),
                artifact_id: ArtifactId::new(artifact_id),
                node_id: StructureNodeId::new(node_id),
                source_span: source_span.into(),
                title,
                body,
                security,
            }),
            Self::ParserStarted {
                artifact_id,
                title,
                source_path,
                content_hash,
                blob_id,
            } => Ok(DomainEvent::ParserStarted {
                artifact_id: ArtifactId::new(artifact_id),
                title,
                source_path,
                content_hash,
                blob_id: BlobId::new(blob_id),
            }),
            Self::DocumentTreeCaptured {
                artifact_id,
                artifact_version_id,
                content_hash,
                root_id,
                nodes,
            } => Ok(DomainEvent::DocumentTreeCaptured {
                artifact_id: ArtifactId::new(artifact_id),
                artifact_version_id: ArtifactVersionId::new(artifact_version_id),
                content_hash,
                root_id: StructureNodeId::new(root_id),
                nodes,
            }),
            Self::ArtifactParsed {
                artifact_id,
                status,
                chunks_added,
            } => Ok(DomainEvent::ArtifactParsed {
                artifact_id: ArtifactId::new(artifact_id),
                status: status.into(),
                chunks_added,
            }),
            Self::SearchCompleted {
                artifact_id,
                cards_added,
            } => Ok(DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(artifact_id),
                cards_added,
            }),
            Self::PendingIndex {
                artifact_id,
                content_hash,
            } => Ok(DomainEvent::PendingIndex {
                artifact_id: ArtifactId::new(artifact_id),
                content_hash,
            }),
            Self::FullTextIndexed {
                artifact_id,
                chunk_id,
            } => Ok(DomainEvent::FullTextIndexed {
                artifact_id: ArtifactId::new(artifact_id),
                chunk_id: ChunkId::new(chunk_id),
            }),
            Self::ArtifactIndexed { artifact_id } => Ok(DomainEvent::ArtifactIndexed {
                artifact_id: ArtifactId::new(artifact_id),
            }),
            other => Err(Box::new(other)),
        }
    }

    pub(crate) fn try_kind_artifact(&self) -> Option<&'static str> {
        match self {
            Self::ArtifactRegistered { .. } => Some("artifact_registered"),
            Self::ChunkRegistered { .. } => Some("chunk_registered"),
            Self::CardCreated { .. } => Some("card_created"),
            Self::ParserStarted { .. } => Some("parser_started"),
            Self::ArtifactParsed { .. } => Some("artifact_parsed"),
            Self::DocumentTreeCaptured { .. } => Some("document_tree_captured"),
            Self::SearchCompleted { .. } => Some("search_completed"),
            Self::PendingIndex { .. } => Some("pending_index"),
            Self::FullTextIndexed { .. } => Some("full_text_indexed"),
            Self::ArtifactIndexed { .. } => Some("artifact_indexed"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_artifact(&self) -> Option<u64> {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ChunkRegistered { artifact_id, .. }
            | Self::CardCreated { artifact_id, .. }
            | Self::ArtifactParsed { artifact_id, .. }
            | Self::SearchCompleted { artifact_id, .. }
            | Self::PendingIndex { artifact_id, .. }
            | Self::FullTextIndexed { artifact_id, .. }
            | Self::ArtifactIndexed { artifact_id, .. }
            | Self::ParserStarted { artifact_id, .. }
            | Self::DocumentTreeCaptured { artifact_id, .. } => Some(*artifact_id),
            _ => None,
        }
    }
}
