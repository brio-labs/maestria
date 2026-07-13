use crate::types::*;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_start_full_text_index(
        &mut self,
        input: &StartFullTextIndex,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        // Crash-after-evidence recovery: all FullTextIndexed events replayed
        // but ArtifactIndexed lost. If artifact is still Pending and no chunks
        // remain unindexed, terminalize now — but only when evidence coverage
        // is complete (one evidence per chunk). Otherwise leave Pending so
        // retry/resume can regenerate evidence.
        let mut generated = Vec::new();
        if let Some(artifact) = self.artifacts.get(&input.artifact_id)
            && artifact.index_status == IndexStatus::Pending
        {
            let has_pending = self.chunks.values().any(|c| {
                c.artifact_id == input.artifact_id && self.pending_full_text.contains(&c.id)
            });
            if !has_pending && self.evidence_complete_for(input.artifact_id) {
                if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                    artifact.index_status = IndexStatus::Indexed;
                }
                generated.push(self.emit_event(DomainEvent::ArtifactIndexed {
                    artifact_id: input.artifact_id,
                }));
                self.pending_parsers.remove(&input.artifact_id);
            }
        }
        Ok(generated)
    }

    pub(super) fn handle_full_text_index_completed(
        &mut self,
        input: FullTextIndexCompleted,
    ) -> Result<Vec<DomainEventEnvelope>, DomainError> {
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }
        if !self.chunks.contains_key(&input.chunk_id) {
            return Err(DomainError::MissingChunk { id: input.chunk_id });
        }
        let chunk = self
            .chunks
            .get(&input.chunk_id)
            .ok_or(DomainError::MissingChunk { id: input.chunk_id })?;
        if chunk.artifact_id != input.artifact_id {
            return Err(DomainError::ArtifactMismatch {
                expected: input.artifact_id,
                actual: chunk.artifact_id,
            });
        }

        let mut generated = Vec::new();

        if self.pending_full_text.remove(&input.chunk_id) {
            generated.push(self.emit_event(DomainEvent::FullTextIndexed {
                artifact_id: input.artifact_id,
                chunk_id: input.chunk_id,
            }));

            let all_done = !self.chunks.values().any(|c| {
                c.artifact_id == input.artifact_id && self.pending_full_text.contains(&c.id)
            });

            if all_done && self.evidence_complete_for(input.artifact_id) {
                if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
                    artifact.index_status = IndexStatus::Indexed;
                }
                generated.push(self.emit_event(DomainEvent::ArtifactIndexed {
                    artifact_id: input.artifact_id,
                }));
                // Terminal indexing frees the pending parser entry so a crash
                // after ArtifactIndexed does not re-trigger parsing on resume.
                self.pending_parsers.remove(&input.artifact_id);
            }
        }

        Ok(generated)
    }
}
