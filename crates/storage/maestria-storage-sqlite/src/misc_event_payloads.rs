use super::event_payloads::StoredEventPayload;
use super::relation_payloads::{StoredRelationEndpoint, StoredRelationKind};
use maestria_domain::{DomainEvent, LogicalTick};

impl StoredEventPayload {
    pub(crate) fn try_from_domain_misc(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => Some(Self::RelationCreated {
                relation_id: relation_id.value(),
                source: StoredRelationEndpoint::from_domain(source),
                kind: StoredRelationKind::from_domain(kind),
                target: StoredRelationEndpoint::from_domain(target),
                evidence_id: evidence_id.map(|id| id.value()),
                confidence_milli: *confidence_milli,
            }),
            DomainEvent::TickObserved { at } => Some(Self::TickObserved { at: at.value() }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_misc(self) -> Result<DomainEvent, Box<Self>> {
        match self {
            Self::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => Ok(DomainEvent::RelationCreated {
                relation_id: maestria_domain::RelationId::new(relation_id),
                source: source.into_domain(),
                kind: kind.into_domain(),
                target: target.into_domain(),
                evidence_id: evidence_id.map(maestria_domain::EvidenceId::new),
                confidence_milli,
            }),
            Self::TickObserved { at } => Ok(DomainEvent::TickObserved {
                at: LogicalTick::new(at),
            }),
            other => Err(Box::new(other)),
        }
    }

    pub(crate) fn try_kind_misc(&self) -> Option<&'static str> {
        match self {
            Self::RelationCreated { .. } => Some("relation_created"),
            Self::TickObserved { .. } => Some("tick_observed"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_misc(&self) -> Option<u64> {
        None
    }
}
