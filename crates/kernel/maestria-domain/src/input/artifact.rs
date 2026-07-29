use crate::security::SecurityMetadata;
use crate::types::*;

impl KernelState {
    fn validate_existing_deterministic_evidence_for_chunk(
        &self,
        artifact_id: ArtifactId,
        order: u32,
    ) -> Result<(), DomainError> {
        let evidence_id = crate::provenance::evidence_id_for(artifact_id, order);
        let Some(existing) = self.evidences.get(&evidence_id) else {
            return Ok(());
        };
        if existing.artifact_id != artifact_id {
            return Err(DomainError::MalformedDeterministicEvidence {
                evidence_id,
                reason: "artifact owner does not match chunk owner",
            });
        }
        match &existing.kind {
            EvidenceKind::FileSpan { snapshot, .. }
            | EvidenceKind::PdfSpan { snapshot, .. }
            | EvidenceKind::PdfRegion { snapshot, .. } => {
                let Some(expected_hash) = self
                    .artifacts
                    .get(&artifact_id)
                    .and_then(|artifact| artifact.content_hash.as_deref())
                else {
                    return Err(DomainError::MalformedDeterministicEvidence {
                        evidence_id,
                        reason: "artifact has no content hash for deterministic evidence",
                    });
                };
                if snapshot.content_hash().as_str() == expected_hash {
                    Ok(())
                } else {
                    Err(DomainError::MalformedDeterministicEvidence {
                        evidence_id,
                        reason: "snapshot content hash does not match artifact content_hash",
                    })
                }
            }
            _ => Err(DomainError::MalformedDeterministicEvidence {
                evidence_id,
                reason: "deterministic evidence must be a source-backed span with a snapshot",
            }),
        }
    }
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_register_artifact(
        &mut self,
        input: RegisterArtifactInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::DuplicateId {
                kind: "artifact",
                id: input.artifact_id.value(),
            });
        }
        let security = SecurityMetadata::from_optional(input.security);
        self.artifacts.insert(
            input.artifact_id,
            Artifact::with_title(input.artifact_id, input.title.clone()),
        );
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.security = security.clone();
        }
        Ok(self.emit_event(DomainEvent::ArtifactRegistered {
            artifact_id: input.artifact_id,
            title: input.title,
            security,
        }))
    }

    pub(super) fn handle_register_chunk(
        &mut self,
        input: RegisterChunkInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if self.chunks.contains_key(&input.chunk_id) {
            return Err(DomainError::DuplicateId {
                kind: "chunk",
                id: input.chunk_id.value(),
            });
        }
        if self
            .chunks
            .values()
            .any(|chunk| chunk.artifact_id == input.artifact_id && chunk.order == input.order)
        {
            return Err(DomainError::DuplicateId {
                kind: "chunk_order",
                id: input.chunk_id.value(),
            });
        }
        self.validate_existing_deterministic_evidence_for_chunk(input.artifact_id, input.order)?;

        let chunk = Chunk::new(
            input.chunk_id,
            input.artifact_id,
            input.node_id,
            input.source_span,
            input.representations.clone(),
            input.order,
            input.text.clone(),
        );
        self.chunks.insert(input.chunk_id, chunk);
        self.chunk_nodes.insert(input.chunk_id, input.node_id);
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.chunk_ids.insert(input.chunk_id);
        }

        Ok(self.emit_event(DomainEvent::ChunkRegistered {
            chunk_id: input.chunk_id,
            artifact_id: input.artifact_id,
            node_id: input.node_id,
            source_span: input.source_span,
            representations: input.representations,
            order: input.order,
            text: input.text,
        }))
    }

    // ── Replay apply methods ─────────────────────────────────────

    pub(crate) fn apply_artifact_registered(
        &mut self,
        artifact_id: ArtifactId,
        title: &str,
        security: &SecurityMetadata,
    ) -> Result<(), DomainError> {
        if self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::DuplicateId {
                kind: "artifact",
                id: artifact_id.value(),
            });
        }
        self.artifacts.insert(
            artifact_id,
            Artifact::with_title(artifact_id, title.to_string()),
        );
        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.security = security.clone();
        }
        Ok(())
    }

    pub(crate) fn apply_chunk_registered(
        &mut self,
        input: RegisterChunkInput,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if self.chunks.contains_key(&input.chunk_id) {
            return Err(DomainError::DuplicateId {
                kind: "chunk",
                id: input.chunk_id.value(),
            });
        }
        if self
            .chunks
            .values()
            .any(|chunk| chunk.artifact_id == input.artifact_id && chunk.order == input.order)
        {
            return Err(DomainError::DuplicateId {
                kind: "chunk_order",
                id: input.chunk_id.value(),
            });
        }
        self.validate_existing_deterministic_evidence_for_chunk(input.artifact_id, input.order)?;

        self.chunks.insert(
            input.chunk_id,
            Chunk::new(
                input.chunk_id,
                input.artifact_id,
                input.node_id,
                input.source_span,
                input.representations,
                input.order,
                input.text,
            ),
        );
        self.chunk_nodes.insert(input.chunk_id, input.node_id);
        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.chunk_ids.insert(input.chunk_id);
        }
        if let Some(artifact) = self.artifacts.get(&input.artifact_id)
            && artifact.index_status == IndexStatus::Pending
        {
            self.pending_full_text.insert(input.chunk_id);
        }
        Ok(())
    }

    pub(crate) fn apply_artifact_parsed(
        &mut self,
        artifact_id: ArtifactId,
        status: crate::provenance::ParseStatus,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        if status != crate::provenance::ParseStatus::Parsed {
            self.pending_parsers.remove(&artifact_id);
        }
        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.parse_status = Some(status);
            if status != crate::provenance::ParseStatus::Parsed {
                artifact.index_status = IndexStatus::Unindexed;
            }
        }
        self.parsed_artifact_ids.insert(artifact_id);
        Ok(())
    }

    pub(crate) fn apply_document_tree_captured(
        &mut self,
        artifact_id: ArtifactId,
        artifact_version_id: crate::ids::ArtifactVersionId,
        content_hash: crate::search::ContentHash,
        root_id: crate::ids::StructureNodeId,
        nodes: &[crate::search::StructureNode],
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        let node_ids: std::collections::BTreeSet<_> = nodes.iter().map(|node| node.id).collect();
        if node_ids.len() != nodes.len()
            || nodes.iter().filter(|node| node.parent_id.is_none()).count() != 1
            || !nodes
                .iter()
                .any(|node| node.id == root_id && node.parent_id.is_none())
            || nodes.iter().any(|node| {
                node.parent_id
                    .is_some_and(|parent| !node_ids.contains(&parent))
                    || node
                        .sibling_id
                        .is_some_and(|sibling| !node_ids.contains(&sibling))
            })
        {
            return Err(DomainError::InternalInvariantViolation {
                detail: "document tree event failed structural validation",
            });
        }
        self.artifact_versions
            .insert(artifact_id, artifact_version_id);
        self.artifact_content_hashes
            .insert(artifact_id, content_hash);
        self.document_trees
            .insert(artifact_id, (root_id, nodes.to_vec()));
        Ok(())
    }

    pub(crate) fn apply_parser_started(
        &mut self,
        artifact_id: ArtifactId,
        title: &str,
        source_path: &str,
        content_hash: &str,
        blob_id: BlobId,
    ) {
        // Reconstruct pending-parser metadata so the daemon can find
        // stranded artifacts on restart and re-drive parsing.
        self.pending_parsers.insert(
            artifact_id,
            ParserStarted {
                artifact_id,
                title: title.to_string(),
                source_path: source_path.to_string(),
                content_hash: content_hash.to_string(),
                blob_id,
            },
        );
    }

    pub(crate) fn apply_pending_index(
        &mut self,
        artifact_id: ArtifactId,
        content_hash: &str,
    ) -> Result<(), DomainError> {
        let artifact = self
            .artifacts
            .get_mut(&artifact_id)
            .ok_or(DomainError::MissingArtifact { id: artifact_id })?;
        artifact.content_hash = Some(content_hash.to_string());
        artifact.index_status = IndexStatus::Pending;
        Ok(())
    }

    pub(crate) fn apply_full_text_indexed(
        &mut self,
        artifact_id: ArtifactId,
        chunk_id: ChunkId,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        let chunk = self
            .chunks
            .get(&chunk_id)
            .ok_or(DomainError::MissingChunk { id: chunk_id })?;
        if chunk.artifact_id != artifact_id {
            return Err(DomainError::ArtifactMismatch {
                expected: artifact_id,
                actual: chunk.artifact_id,
            });
        }
        self.pending_full_text.remove(&chunk_id);
        Ok(())
    }

    pub(crate) fn apply_artifact_indexed(
        &mut self,
        artifact_id: ArtifactId,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        let has_pending = self
            .chunks
            .values()
            .any(|c| c.artifact_id == artifact_id && self.pending_full_text.contains(&c.id));
        if has_pending {
            return Err(DomainError::PendingChunksExist { artifact_id });
        }
        if !self.evidence_complete_for(artifact_id) {
            let evidence_id = self
                .chunks
                .values()
                .find(|chunk| chunk.artifact_id == artifact_id)
                .map(|chunk| crate::evidence_id_for(chunk.artifact_id, chunk.order))
                .ok_or(DomainError::EvidenceRequired {
                    kind: "artifact_indexed",
                    id: artifact_id.value(),
                })?;
            if !self.evidences.contains_key(&evidence_id) {
                return Err(DomainError::MissingEvidence { id: evidence_id });
            }
            return Err(DomainError::MalformedDeterministicEvidence {
                evidence_id,
                reason: "ArtifactIndexed requires complete source-backed evidence",
            });
        }
        let artifact = self
            .artifacts
            .get_mut(&artifact_id)
            .ok_or(DomainError::MissingArtifact { id: artifact_id })?;
        artifact.index_status = IndexStatus::Indexed;
        self.pending_parsers.remove(&artifact_id);
        Ok(())
    }
}
