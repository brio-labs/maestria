use super::event_payloads::StoredEventPayload;
use maestria_domain::DomainEvent;

impl StoredEventPayload {
    pub(crate) fn try_from_domain_memory(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => Some(Self::MemoryCandidateCreated {
                candidate_id: candidate_id.value(),
                claim_id: claim_id.value(),
                evidence_ids: evidence_ids
                    .iter()
                    .map(|evidence_id| evidence_id.value())
                    .collect(),
                confidence_milli: *confidence_milli,
            }),
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
            } => Some(Self::MemoryPromoted {
                memory_id: memory_id.value(),
                candidate_id: candidate_id.value(),
            }),
            DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => Some(Self::MemoryContradicted {
                memory_id: memory_id.value(),
                contradicting_candidate_id: contradicting_candidate_id.value(),
            }),
            DomainEvent::MemoryDeprecated { memory_id } => Some(Self::MemoryDeprecated {
                memory_id: memory_id.value(),
            }),
            DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => Some(Self::MemorySuperseded {
                memory_id: memory_id.value(),
                by_memory_id: by_memory_id.value(),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_memory(self) -> Result<DomainEvent, Box<Self>> {
        match self {
            Self::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => Ok(DomainEvent::MemoryCandidateCreated {
                candidate_id: maestria_domain::MemoryCandidateId::new(candidate_id),
                claim_id: maestria_domain::ClaimId::new(claim_id),
                evidence_ids: evidence_ids
                    .into_iter()
                    .map(maestria_domain::EvidenceId::new)
                    .collect(),
                confidence_milli,
            }),
            Self::MemoryPromoted {
                memory_id,
                candidate_id,
            } => Ok(DomainEvent::MemoryPromoted {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                candidate_id: maestria_domain::MemoryCandidateId::new(candidate_id),
            }),
            Self::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => Ok(DomainEvent::MemoryContradicted {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                contradicting_candidate_id: maestria_domain::MemoryCandidateId::new(
                    contradicting_candidate_id,
                ),
            }),
            Self::MemoryDeprecated { memory_id } => Ok(DomainEvent::MemoryDeprecated {
                memory_id: maestria_domain::MemoryId::new(memory_id),
            }),
            Self::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => Ok(DomainEvent::MemorySuperseded {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                by_memory_id: maestria_domain::MemoryId::new(by_memory_id),
            }),
            other => Err(Box::new(other)),
        }
    }

    pub(crate) fn try_kind_memory(&self) -> Option<&'static str> {
        match self {
            Self::MemoryCandidateCreated { .. } => Some("memory_candidate_created"),
            Self::MemoryPromoted { .. } => Some("memory_promoted"),
            Self::MemoryContradicted { .. } => Some("memory_contradicted"),
            Self::MemoryDeprecated { .. } => Some("memory_deprecated"),
            Self::MemorySuperseded { .. } => Some("memory_superseded"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_memory(&self) -> Option<u64> {
        None
    }
}
