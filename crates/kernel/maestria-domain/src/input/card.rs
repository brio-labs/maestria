use crate::security::SecurityMetadata;
use crate::types::*;

impl KernelState {
    // ── Handler ──────────────────────────────────────────────────

    pub(super) fn handle_create_card(
        &mut self,
        input: CreateCardInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.cards.contains_key(&input.card_id) {
            return Err(DomainError::DuplicateId {
                kind: "card",
                id: input.card_id.value(),
            });
        }
        if !self.artifacts.contains_key(&input.artifact_id) {
            return Err(DomainError::MissingArtifact {
                id: input.artifact_id,
            });
        }

        let mut security = SecurityMetadata::from_optional(input.security);
        if let Some(artifact) = self.artifacts.get(&input.artifact_id) {
            security = security.taint_from(&artifact.security);
        }
        self.cards.insert(
            input.card_id,
            Card::new(
                input.card_id,
                input.artifact_id,
                input.node_id,
                input.source_span,
                input.title.clone(),
                input.body.clone(),
                security.clone(),
            ),
        );

        if let Some(artifact) = self.artifacts.get_mut(&input.artifact_id) {
            artifact.card_ids.insert(input.card_id);
        }

        Ok(self.emit_event(DomainEvent::CardCreated {
            card_id: input.card_id,
            artifact_id: input.artifact_id,
            node_id: input.node_id,
            source_span: input.source_span,
            title: input.title,
            body: input.body,
            security,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_card_created(
        &mut self,
        card_id: CardId,
        artifact_id: ArtifactId,
        node_id: crate::ids::StructureNodeId,
        source_span: crate::provenance::SourceSpan,
        title: &str,
        body: &str,
        security: &SecurityMetadata,
    ) -> Result<(), DomainError> {
        if !self.artifacts.contains_key(&artifact_id) {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }
        if self.cards.contains_key(&card_id) {
            return Err(DomainError::DuplicateId {
                kind: "card",
                id: card_id.value(),
            });
        }
        self.cards.insert(
            card_id,
            Card::new(
                card_id,
                artifact_id,
                node_id,
                source_span,
                title.to_string(),
                body.to_string(),
                security.clone(),
            ),
        );
        if let Some(artifact) = self.artifacts.get_mut(&artifact_id) {
            artifact.card_ids.insert(card_id);
        }
        Ok(())
    }
}
