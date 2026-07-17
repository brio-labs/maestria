use crate::security::SecurityMetadata;
use crate::types::*;

pub(crate) struct ApplyRelationCreatedArgs<'a> {
    pub relation_id: RelationId,
    pub source: RelationEndpoint,
    pub kind: RelationKind,
    pub target: RelationEndpoint,
    pub evidence_id: Option<EvidenceId>,
    pub confidence_milli: u16,
    pub security: &'a SecurityMetadata,
}

impl KernelState {
    // ── Handler ──────────────────────────────────────────────────

    pub(super) fn handle_create_relation(
        &mut self,
        input: CreateRelationInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if input.confidence_milli > 1000 {
            return Err(DomainError::InvalidConfidence {
                max: 1000,
                actual: input.confidence_milli,
            });
        }
        let validate_endpoint = |endpoint: &RelationEndpoint| -> Result<(), DomainError> {
            match endpoint {
                RelationEndpoint::Artifact(id) => {
                    if !self.artifacts.contains_key(id) {
                        return Err(DomainError::MissingArtifact { id: *id });
                    }
                }
                RelationEndpoint::Claim(id) => {
                    if !self.claims.contains_key(id) {
                        return Err(DomainError::MissingClaim { id: *id });
                    }
                }
                RelationEndpoint::Task(id) => {
                    if !self.tasks.contains_key(id) {
                        return Err(DomainError::MissingTask { id: *id });
                    }
                }
                RelationEndpoint::Memory(id) => {
                    if !self.memories.contains_key(id) {
                        return Err(DomainError::MissingMemory { id: *id });
                    }
                }
                RelationEndpoint::Card(id) => {
                    if !self.cards.contains_key(id) {
                        return Err(DomainError::MissingCard { id: *id });
                    }
                }
            }
            Ok(())
        };
        validate_endpoint(&input.source)?;
        validate_endpoint(&input.target)?;
        let endpoint_security = |endpoint: &RelationEndpoint| match endpoint {
            RelationEndpoint::Artifact(id) => self.artifacts.get(id).map(|v| &v.security),
            RelationEndpoint::Claim(id) => self.claims.get(id).map(|v| &v.security),
            RelationEndpoint::Task(_) => None,
            RelationEndpoint::Memory(id) => self.memories.get(id).map(|v| &v.security),
            RelationEndpoint::Card(id) => self.cards.get(id).map(|v| &v.security),
        };
        if self.relations.contains_key(&input.relation_id) {
            return Err(DomainError::DuplicateId {
                kind: "relation",
                id: input.relation_id.value(),
            });
        }
        if let Some(evidence_id) = input.evidence_id
            && !self.evidences.contains_key(&evidence_id)
        {
            return Err(DomainError::MissingEvidence { id: evidence_id });
        }
        let mut security = SecurityMetadata::from_optional(input.security);
        if let Some(source_security) = endpoint_security(&input.source) {
            security = security.taint_from(source_security);
        }
        if let Some(target_security) = endpoint_security(&input.target) {
            security = security.taint_from(target_security);
        }
        if let Some(evidence_id) = input.evidence_id
            && let Some(evidence) = self.evidences.get(&evidence_id)
        {
            security = security.taint_from(&evidence.security);
        }
        let relation = Relation {
            id: input.relation_id,
            source: input.source,
            kind: input.kind,
            target: input.target,
            evidence_id: input.evidence_id,
            confidence_milli: input.confidence_milli,
            security: security.clone(),
        };
        self.relations.insert(input.relation_id, relation);
        Ok(self.emit_event(DomainEvent::RelationCreated {
            relation_id: input.relation_id,
            source: input.source,
            kind: input.kind,
            target: input.target,
            evidence_id: input.evidence_id,
            confidence_milli: input.confidence_milli,
            security,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────
    pub(crate) fn apply_relation_created(
        &mut self,
        args: ApplyRelationCreatedArgs<'_>,
    ) -> Result<(), DomainError> {
        let ApplyRelationCreatedArgs {
            relation_id,
            source,
            kind,
            target,
            evidence_id,
            confidence_milli,
            security,
        } = args;
        if confidence_milli > 1000 {
            return Err(DomainError::InvalidConfidence {
                max: 1000,
                actual: confidence_milli,
            });
        }
        let validate_endpoint = |endpoint: &RelationEndpoint| -> Result<(), DomainError> {
            match endpoint {
                RelationEndpoint::Artifact(id) => {
                    if !self.artifacts.contains_key(id) {
                        return Err(DomainError::MissingArtifact { id: *id });
                    }
                }
                RelationEndpoint::Claim(id) => {
                    if !self.claims.contains_key(id) {
                        return Err(DomainError::MissingClaim { id: *id });
                    }
                }
                RelationEndpoint::Task(id) => {
                    if !self.tasks.contains_key(id) {
                        return Err(DomainError::MissingTask { id: *id });
                    }
                }
                RelationEndpoint::Memory(id) => {
                    if !self.memories.contains_key(id) {
                        return Err(DomainError::MissingMemory { id: *id });
                    }
                }
                RelationEndpoint::Card(id) => {
                    if !self.cards.contains_key(id) {
                        return Err(DomainError::MissingCard { id: *id });
                    }
                }
            }
            Ok(())
        };
        validate_endpoint(&source)?;
        validate_endpoint(&target)?;
        if self.relations.contains_key(&relation_id) {
            return Err(DomainError::DuplicateId {
                kind: "relation",
                id: relation_id.value(),
            });
        }
        if let Some(ev_id) = evidence_id
            && !self.evidences.contains_key(&ev_id)
        {
            return Err(DomainError::MissingEvidence { id: ev_id });
        }
        self.relations.insert(
            relation_id,
            Relation {
                id: relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
                security: security.clone(),
            },
        );
        Ok(())
    }
}
