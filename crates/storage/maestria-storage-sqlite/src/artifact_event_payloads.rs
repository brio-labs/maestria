use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use super::stored_content::StoredContentHash;
use super::stored_security::StoredSecurityMetadata;
use super::stored_structure::StoredStructureNode;
use maestria_domain::{
    ArtifactId, ArtifactVersionId, BlobId, ChunkId, DomainEvent, StructureNodeId,
};

impl StoredEventPayload {
    pub(crate) fn try_from_domain_artifact(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::ArtifactRegistered {
                artifact_id,
                title,
                security,
            } => Some(Self::ArtifactRegistered {
                artifact_id: artifact_id.value(),
                title: title.clone(),
                security: StoredSecurityMetadata::from_domain(security),
            }),
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                node_id,
                source_span,
                representations,
                order,
                text,
            } => Some(Self::from_chunk_registered(
                *chunk_id,
                *artifact_id,
                *node_id,
                *source_span,
                representations,
                *order,
                text,
            )),
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                node_id,
                source_span,
                title,
                body,
                security,
            } => Some(Self::from_card_created(
                *card_id,
                *artifact_id,
                *node_id,
                *source_span,
                title,
                body,
                security,
            )),
            _ => Self::try_from_domain_artifact_tail(event),
        }
    }

    pub(crate) fn try_into_domain_artifact(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            Self::ArtifactRegistered {
                artifact_id,
                title,
                security,
            } => Ok(DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(artifact_id),
                title,
                security: security
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            Self::ChunkRegistered {
                chunk_id,
                artifact_id,
                node_id,
                source_span,
                representations,
                order,
                text,
            } => Ok(Self::into_chunk_registered(
                chunk_id,
                artifact_id,
                node_id,
                source_span,
                representations,
                order,
                text,
            )?),
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
                source_span: source_span.try_into().map_err(FamilyDecodeError::Invalid)?,
                title,
                body,
                security: security
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            other => Self::try_into_domain_artifact_tail(other),
        }
    }
    fn try_from_domain_artifact_tail(event: &DomainEvent) -> Option<Self> {
        match event {
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
                content_hash: StoredContentHash::from_domain(content_hash),
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
                content_hash: StoredContentHash::from_domain(content_hash),
                root_id: root_id.value(),
                nodes: nodes.iter().map(StoredStructureNode::from_domain).collect(),
            }),
            DomainEvent::ArtifactParsed {
                artifact_id,
                status,
            } => Some(Self::ArtifactParsed {
                artifact_id: artifact_id.value(),
                status: crate::payloads::StoredParseStatus::from_domain(*status),
            }),
            DomainEvent::SearchCompleted { artifact_id } => Some(Self::SearchCompleted {
                artifact_id: artifact_id.value(),
            }),
            DomainEvent::PendingIndex {
                artifact_id,
                content_hash,
            } => Some(Self::PendingIndex {
                artifact_id: artifact_id.value(),
                content_hash: StoredContentHash::from_domain(content_hash),
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
    fn try_into_domain_artifact_tail(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
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
                content_hash: content_hash
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
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
                content_hash: content_hash
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
                root_id: StructureNodeId::new(root_id),
                nodes: nodes
                    .into_iter()
                    .map(StoredStructureNode::try_into_domain)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            Self::ArtifactParsed {
                artifact_id,
                status,
            } => Ok(DomainEvent::ArtifactParsed {
                artifact_id: ArtifactId::new(artifact_id),
                status: status
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            Self::SearchCompleted { artifact_id } => Ok(DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(artifact_id),
            }),
            Self::PendingIndex {
                artifact_id,
                content_hash,
            } => Ok(DomainEvent::PendingIndex {
                artifact_id: ArtifactId::new(artifact_id),
                content_hash: content_hash
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
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
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
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

    fn from_chunk_registered(
        chunk_id: ChunkId,
        artifact_id: ArtifactId,
        node_id: StructureNodeId,
        source_span: maestria_domain::SourceSpan,
        representations: &[maestria_domain::ParsedRepresentation],
        order: u32,
        text: &str,
    ) -> Self {
        Self::ChunkRegistered {
            chunk_id: chunk_id.value(),
            artifact_id: artifact_id.value(),
            node_id: node_id.value(),
            source_span: source_span.into(),
            representations: representations.iter().cloned().map(Into::into).collect(),
            order,
            text: text.to_owned(),
        }
    }

    fn from_card_created(
        card_id: maestria_domain::CardId,
        artifact_id: ArtifactId,
        node_id: StructureNodeId,
        source_span: maestria_domain::SourceSpan,
        title: &str,
        body: &str,
        security: &maestria_domain::SecurityMetadata,
    ) -> Self {
        Self::CardCreated {
            card_id: card_id.value(),
            artifact_id: artifact_id.value(),
            node_id: node_id.value(),
            source_span: source_span.into(),
            title: title.to_string(),
            body: body.to_string(),
            security: StoredSecurityMetadata::from_domain(security),
        }
    }

    fn into_chunk_registered(
        chunk_id: u64,
        artifact_id: u64,
        node_id: u64,
        source_span: crate::payloads::StoredSourceSpan,
        representations: Vec<crate::payloads::StoredParsedRepresentation>,
        order: u32,
        text: String,
    ) -> Result<DomainEvent, FamilyDecodeError> {
        Ok(DomainEvent::ChunkRegistered {
            chunk_id: ChunkId::new(chunk_id),
            artifact_id: ArtifactId::new(artifact_id),
            node_id: StructureNodeId::new(node_id),
            source_span: source_span.try_into().map_err(FamilyDecodeError::Invalid)?,
            representations: representations
                .into_iter()
                .map(|r| r.try_into_domain())
                .collect::<Result<_, _>>()
                .map_err(FamilyDecodeError::Invalid)?,
            order,
            text,
        })
    }
}
