use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use super::evidence_payloads::{StoredClaimStatus, StoredEvidenceKind};
use super::stored_security::StoredSecurityMetadata;
use maestria_domain::{ArtifactId, DomainEvent, EvidenceId, LogicalTick};

impl StoredEventPayload {
    pub(crate) fn try_from_domain_claim(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
                security,
            } => Some(Self::ClaimCreated {
                claim_id: claim_id.value(),
                artifact_id: artifact_id.value(),
                text: text.clone(),
                evidence_ids: evidence_ids.iter().map(|id| id.value()).collect(),
                security: StoredSecurityMetadata::from_domain(security),
            }),
            DomainEvent::ClaimValidationUpdated { claim_id, status } => {
                Some(Self::ClaimValidationUpdated {
                    claim_id: claim_id.value(),
                    status: StoredClaimStatus::from_domain(status),
                })
            }
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => Some(Self::ClaimEvidenceLinked {
                claim_id: claim_id.value(),
                evidence_id: evidence_id.value(),
            }),
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
                excerpt,
                observed_at,
                security,
            } => Some(Self::EvidenceRecorded {
                evidence_id: evidence_id.value(),
                artifact_id: artifact_id.value(),
                claim_id: claim_id.map(|id| id.value()),
                evidence_kind: StoredEvidenceKind::from_domain(kind),
                excerpt: excerpt.clone(),
                security: StoredSecurityMetadata::from_domain(security),
                observed_at: observed_at.value(),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_claim(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            Self::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
                security,
            } => Ok(DomainEvent::ClaimCreated {
                claim_id: maestria_domain::ClaimId::new(claim_id),
                artifact_id: ArtifactId::new(artifact_id),
                text,
                evidence_ids: evidence_ids.into_iter().map(EvidenceId::new).collect(),
                security: security
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            Self::ClaimValidationUpdated { claim_id, status } => {
                Ok(DomainEvent::ClaimValidationUpdated {
                    claim_id: maestria_domain::ClaimId::new(claim_id),
                    status: status.into_domain(),
                })
            }
            Self::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => Ok(DomainEvent::ClaimEvidenceLinked {
                claim_id: maestria_domain::ClaimId::new(claim_id),
                evidence_id: EvidenceId::new(evidence_id),
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    pub(crate) fn try_into_domain_evidence(self) -> Result<DomainEvent, maestria_ports::PortError> {
        match self {
            Self::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                evidence_kind,
                excerpt,
                observed_at,
                security,
            } => {
                let kind = evidence_kind.try_into_domain().map_err(|source| {
                    maestria_ports::PortError::InternalContext {
                        context: "decode stored evidence kind",
                        source,
                    }
                })?;
                Ok(DomainEvent::EvidenceRecorded {
                    evidence_id: EvidenceId::new(evidence_id),
                    artifact_id: ArtifactId::new(artifact_id),
                    claim_id: claim_id.map(maestria_domain::ClaimId::new),
                    kind,
                    excerpt,
                    security: security.try_into_domain()?,
                    observed_at: LogicalTick::new(observed_at),
                })
            }
            _ => Err(maestria_ports::PortError::InternalContext {
                context: "decode stored evidence event",
                source: "conversion branch invariant was violated".to_string(),
            }),
        }
    }

    pub(crate) fn try_kind_claim(&self) -> Option<&'static str> {
        match self {
            Self::ClaimCreated { .. } => Some("claim_created"),
            Self::ClaimValidationUpdated { .. } => Some("claim_validation_updated"),
            Self::ClaimEvidenceLinked { .. } => Some("claim_evidence_linked"),
            Self::EvidenceRecorded { .. } => Some("evidence_recorded"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_claim(&self) -> Option<u64> {
        match self {
            Self::ClaimCreated { artifact_id, .. } | Self::EvidenceRecorded { artifact_id, .. } => {
                Some(*artifact_id)
            }
            _ => None,
        }
    }
}
