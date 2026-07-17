use super::event_payloads::StoredEventPayload;
use super::relation_payloads::{StoredRelationEndpoint, StoredRelationKind};
use maestria_domain::{DomainEvent, EvidenceId, LogicalTick};

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
                security,
            } => Some(Self::RelationCreated {
                relation_id: relation_id.value(),
                source: StoredRelationEndpoint::from_domain(source),
                kind: StoredRelationKind::from_domain(kind),
                target: StoredRelationEndpoint::from_domain(target),
                evidence_id: evidence_id.map(|id| id.value()),
                confidence_milli: *confidence_milli,
                security: security.clone(),
            }),
            DomainEvent::TickObserved { at } => Some(Self::TickObserved { at: at.value() }),
            DomainEvent::SearchExecuted {
                query,
                limit,
                evidence_ids,
                pack_metadata,
                at,
            } => Some(Self::SearchExecuted {
                query: query.clone(),
                limit: *limit as u64,
                evidence_ids: evidence_ids.iter().map(|id| id.value()).collect(),
                pack_metadata: pack_metadata.clone(),
                at: at.value(),
            }),
            DomainEvent::SearchKnowledgeCompleted {
                task_id,
                plan,
                outcome,
            } => Some(Self::SearchKnowledgeCompleted {
                task_id: task_id.map(|id| id.value()),
                plan: plan.clone(),
                outcome: outcome.clone(),
            }),
            DomainEvent::IndexGenerationStarted {
                id,
                name,
                corpus_snapshot,
                fingerprint,
            } => Some(Self::IndexGenerationStarted {
                id: id.value(),
                name: name.clone(),
                corpus_snapshot: corpus_snapshot.value(),
                fingerprint: fingerprint.clone(),
            }),
            DomainEvent::IndexGenerationTransitioned {
                id,
                from,
                to,
                replaced_active_id,
            } => Some(Self::IndexGenerationTransitioned {
                id: id.value(),
                from: *from,
                to: *to,
                replaced_active_id: replaced_active_id.map(|i| i.value()),
            }),
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
                security,
            } => Ok(DomainEvent::RelationCreated {
                relation_id: maestria_domain::RelationId::new(relation_id),
                source: source.into_domain(),
                kind: kind.into_domain(),
                target: target.into_domain(),
                evidence_id: evidence_id.map(maestria_domain::EvidenceId::new),
                confidence_milli,
                security,
            }),
            Self::TickObserved { at } => Ok(DomainEvent::TickObserved {
                at: LogicalTick::new(at),
            }),
            Self::SearchExecuted {
                query,
                limit,
                evidence_ids,
                pack_metadata,
                at,
            } => match usize::try_from(limit) {
                Ok(limit) => Ok(DomainEvent::SearchExecuted {
                    query,
                    limit,
                    evidence_ids: evidence_ids.into_iter().map(EvidenceId::new).collect(),
                    pack_metadata,
                    at: LogicalTick::new(at),
                }),
                Err(_) => Err(Box::new(Self::SearchExecuted {
                    query,
                    limit,
                    evidence_ids,
                    pack_metadata,
                    at,
                })),
            },
            Self::SearchKnowledgeCompleted {
                task_id,
                plan,
                outcome,
            } => Ok(DomainEvent::SearchKnowledgeCompleted {
                task_id: task_id.map(maestria_domain::TaskId::new),
                plan,
                outcome,
            }),
            Self::IndexGenerationStarted {
                id,
                name,
                corpus_snapshot,
                fingerprint,
            } => Ok(DomainEvent::IndexGenerationStarted {
                id: maestria_domain::IndexGenerationId::new(id),
                name,
                corpus_snapshot: maestria_domain::CorpusSnapshotId::new(corpus_snapshot),
                fingerprint,
            }),
            Self::IndexGenerationTransitioned {
                id,
                from,
                to,
                replaced_active_id,
            } => Ok(DomainEvent::IndexGenerationTransitioned {
                id: maestria_domain::IndexGenerationId::new(id),
                from,
                to,
                replaced_active_id: replaced_active_id.map(maestria_domain::IndexGenerationId::new),
            }),
            other => Err(Box::new(other)),
        }
    }

    pub(crate) fn try_kind_misc(&self) -> Option<&'static str> {
        match self {
            Self::SearchKnowledgeCompleted { .. } => Some("search_knowledge_completed"),
            Self::RelationCreated { .. } => Some("relation_created"),
            Self::TickObserved { .. } => Some("tick_observed"),
            Self::SearchExecuted { .. } => Some("search_executed"),
            Self::IndexGenerationStarted { .. } => Some("index_generation_started"),
            Self::IndexGenerationTransitioned { .. } => Some("index_generation_transitioned"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_misc(&self) -> Option<u64> {
        None
    }
}
