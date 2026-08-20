use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use super::relation_payloads::{StoredRelationEndpoint, StoredRelationKind};
use super::stored_evidence_pack::StoredEvidencePackMetadataRecord;
use super::stored_generations::{
    StoredIndexFingerprint, StoredIndexLifecycle, StoredRepresentationName,
};
use super::stored_model_agent::{StoredModelAgentProposalRequest, StoredModelAgentProposalResult};
use super::stored_search::{StoredSearchOutcome, StoredSearchPlan};
use super::stored_security::StoredSecurityMetadata;
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
                security: StoredSecurityMetadata::from_domain(security),
            }),
            DomainEvent::TickObserved { at } => Some(Self::TickObserved { at: at.value() }),
            other => Self::try_from_domain_misc_search(other),
        }
    }

    fn try_from_domain_misc_search(event: &DomainEvent) -> Option<Self> {
        match event {
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
                pack_metadata: pack_metadata
                    .as_ref()
                    .map(|record| Box::new(StoredEvidencePackMetadataRecord::from_domain(record))),
                at: at.value(),
            }),
            DomainEvent::SearchKnowledgeCompleted {
                task_id,
                plan,
                outcome,
            } => Some(Self::SearchKnowledgeCompleted {
                task_id: task_id.map(|id| id.value()),
                plan: plan
                    .as_ref()
                    .map(|plan| Box::new(StoredSearchPlan::from_domain(plan))),
                outcome: StoredSearchOutcome::from_domain(outcome),
            }),
            DomainEvent::ModelAgentProposalRequested { request } => {
                Some(Self::ModelAgentProposalRequested {
                    request: StoredModelAgentProposalRequest::from_domain(request),
                })
            }
            DomainEvent::ModelAgentProposalCompleted { result } => {
                Some(Self::ModelAgentProposalCompleted {
                    result: StoredModelAgentProposalResult::from_domain(result),
                })
            }
            DomainEvent::IndexGenerationStarted {
                id,
                name,
                corpus_snapshot,
                fingerprint,
                sparse_namespace,
            } => Some(Self::IndexGenerationStarted {
                id: id.value(),
                name: StoredRepresentationName::from_domain(name),
                corpus_snapshot: corpus_snapshot.value(),
                fingerprint: StoredIndexFingerprint::from_domain(fingerprint),
                sparse_namespace: sparse_namespace.clone(),
            }),
            DomainEvent::IndexGenerationTransitioned {
                id,
                from,
                to,
                replaced_active_id,
            } => Some(Self::IndexGenerationTransitioned {
                id: id.value(),
                from: StoredIndexLifecycle::from_domain(*from),
                to: StoredIndexLifecycle::from_domain(*to),
                replaced_active_id: replaced_active_id.map(|i| i.value()),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_misc(self) -> Result<DomainEvent, FamilyDecodeError> {
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
                kind: kind.try_into_domain().map_err(FamilyDecodeError::Invalid)?,
                target: target.into_domain(),
                evidence_id: evidence_id.map(maestria_domain::EvidenceId::new),
                confidence_milli,
                security: security
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            Self::TickObserved { at } => Ok(DomainEvent::TickObserved {
                at: LogicalTick::new(at),
            }),
            other => Self::try_into_domain_misc_search(other),
        }
    }

    fn try_into_domain_misc_search(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
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
                    pack_metadata: pack_metadata
                        .map(|record| record.try_into_domain())
                        .transpose()
                        .map_err(FamilyDecodeError::Invalid)?
                        .map(Box::new),
                    at: LogicalTick::new(at),
                }),
                Err(_) => Err(FamilyDecodeError::Foreign(Box::new(Self::SearchExecuted {
                    query,
                    limit,
                    evidence_ids,
                    pack_metadata,
                    at,
                }))),
            },
            Self::SearchKnowledgeCompleted {
                task_id,
                plan,
                outcome,
            } => Ok(DomainEvent::SearchKnowledgeCompleted {
                task_id: task_id.map(maestria_domain::TaskId::new),
                plan: plan
                    .map(|plan| plan.try_into_domain())
                    .transpose()
                    .map_err(FamilyDecodeError::Invalid)?
                    .map(Box::new),
                outcome: outcome
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            Self::ModelAgentProposalRequested { request } => {
                Ok(DomainEvent::ModelAgentProposalRequested {
                    request: request
                        .try_into_domain()
                        .map_err(FamilyDecodeError::Invalid)?,
                })
            }
            Self::ModelAgentProposalCompleted { result } => {
                Ok(DomainEvent::ModelAgentProposalCompleted {
                    result: result
                        .try_into_domain()
                        .map_err(FamilyDecodeError::Invalid)?,
                })
            }
            Self::IndexGenerationStarted {
                id,
                name,
                corpus_snapshot,
                fingerprint,
                sparse_namespace,
            } => Ok(DomainEvent::IndexGenerationStarted {
                id: maestria_domain::IndexGenerationId::new(id),
                name: name.try_into_domain().map_err(FamilyDecodeError::Invalid)?,
                corpus_snapshot: maestria_domain::CorpusSnapshotId::new(corpus_snapshot),
                fingerprint: fingerprint
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
                sparse_namespace,
            }),
            Self::IndexGenerationTransitioned {
                id,
                from,
                to,
                replaced_active_id,
            } => Ok(DomainEvent::IndexGenerationTransitioned {
                id: maestria_domain::IndexGenerationId::new(id),
                from: from.try_into_domain().map_err(FamilyDecodeError::Invalid)?,
                to: to.try_into_domain().map_err(FamilyDecodeError::Invalid)?,
                replaced_active_id: replaced_active_id.map(maestria_domain::IndexGenerationId::new),
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    pub(crate) fn try_kind_misc(&self) -> Option<&'static str> {
        match self {
            Self::SearchKnowledgeCompleted { .. } => Some("search_knowledge_completed"),
            Self::ModelAgentProposalRequested { .. } => Some("model_agent_proposal_requested"),
            Self::ModelAgentProposalCompleted { .. } => Some("model_agent_proposal_completed"),
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
