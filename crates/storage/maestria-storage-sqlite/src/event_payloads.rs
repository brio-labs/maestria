use crate::payloads::{
    StoredClaimStatus, StoredEvidenceKind, StoredTaskPriority, StoredTaskStatus,
};
use maestria_domain::{
    ArtifactId, DomainEvent, EvidenceId, LogicalTick, RelationEndpoint, TaskId, ValidationReportId,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LegacyStoredEventPayload {
    ArtifactRegistered {
        artifact_id: u64,
        title: String,
    },
    ChunkRegistered {
        chunk_id: u64,
        artifact_id: u64,
        order: u32,
    },
    CardCreated {
        card_id: u64,
        artifact_id: u64,
    },
    ClaimCreated {
        claim_id: u64,
        artifact_id: u64,
    },
    EvidenceRecorded {
        evidence_id: u64,
        evidence_kind: StoredEvidenceKind,
        artifact_id: u64,
        claim_id: Option<u64>,
    },
    TaskOpened {
        task_id: u64,
        title: String,
        priority: StoredTaskPriority,
    },
    TaskStatusChanged {
        task_id: u64,
        from: StoredTaskStatus,
        to: StoredTaskStatus,
    },
    TaskCompletionRecorded {
        task_id: u64,
        status: StoredTaskStatus,
        validation_report_id: u64,
    },
    ClaimValidationUpdated {
        claim_id: u64,
        status: StoredClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: u64,
        evidence_id: u64,
    },
    RelationCreated {
        relation_id: u64,
    },
    MemoryCandidateCreated {
        candidate_id: u64,
        claim_id: u64,
        evidence_ids: Vec<u64>,
        confidence_milli: u16,
    },
    MemoryPromoted {
        memory_id: u64,
        candidate_id: u64,
    },
    MemoryContradicted {
        memory_id: u64,
        contradicting_candidate_id: u64,
    },
    MemoryDeprecated {
        memory_id: u64,
    },
    MemorySuperseded {
        memory_id: u64,
        by_memory_id: u64,
    },
    ValidationReportCreated {
        report_id: u64,
        task_id: Option<u64>,
        passed: bool,
        warnings: Vec<String>,
    },
    UserIntentObserved {
        task_id: u64,
        title: String,
    },
    ArtifactParsed {
        artifact_id: u64,
        chunks_added: u32,
    },
    SearchCompleted {
        artifact_id: u64,
        cards_added: u32,
    },
    HarnessRunCompleted {
        task_id: Option<u64>,
        command: String,
        exit_code: i32,
    },
    ApprovalRecorded {
        task_id: u64,
        approved: bool,
    },
    TickObserved {
        at: u64,
    },
}
impl LegacyStoredEventPayload {
    pub(crate) fn into_v2(self) -> Result<StoredEventPayload, PortError> {
        let unsupported = |kind: &str, field: &str| PortError::InvalidInput {
            message: format!("V1 {kind} event is missing required field(s): {field}"),
        };
        match self {
            Self::ArtifactRegistered { artifact_id, title } => {
                Ok(StoredEventPayload::ArtifactRegistered { artifact_id, title })
            }
            Self::ChunkRegistered { .. } => Err(unsupported("ChunkRegistered", "text")),
            Self::CardCreated { .. } => Err(unsupported("CardCreated", "title and body")),
            Self::ClaimCreated { .. } => Err(unsupported("ClaimCreated", "text and evidence_ids")),
            Self::EvidenceRecorded { .. } => {
                Err(unsupported("EvidenceRecorded", "excerpt and observed_at"))
            }
            Self::TaskOpened { .. } => Err(unsupported("TaskOpened", "artifact_id")),
            Self::TaskStatusChanged { task_id, from, to } => {
                Ok(StoredEventPayload::TaskStatusChanged { task_id, from, to })
            }
            Self::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => Ok(StoredEventPayload::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            }),
            Self::ClaimValidationUpdated { claim_id, status } => {
                Ok(StoredEventPayload::ClaimValidationUpdated { claim_id, status })
            }
            Self::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => Ok(StoredEventPayload::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            }),
            Self::RelationCreated { .. } => Err(unsupported(
                "RelationCreated",
                "source, kind, target, evidence_id, confidence_milli",
            )),
            Self::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => Ok(StoredEventPayload::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            }),
            Self::MemoryPromoted {
                memory_id,
                candidate_id,
            } => Ok(StoredEventPayload::MemoryPromoted {
                memory_id,
                candidate_id,
            }),
            Self::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => Ok(StoredEventPayload::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            }),
            Self::MemoryDeprecated {
                memory_id,
            } => Ok(StoredEventPayload::MemoryDeprecated { memory_id }),
            Self::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => Ok(StoredEventPayload::MemorySuperseded {
                memory_id,
                by_memory_id,
            }),
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Ok(StoredEventPayload::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            }),
            Self::UserIntentObserved { task_id, title } => {
                Ok(StoredEventPayload::UserIntentObserved { task_id, title })
            }
            Self::ArtifactParsed {
                artifact_id,
                chunks_added,
            } => Ok(StoredEventPayload::ArtifactParsed {
                artifact_id,
                chunks_added,
            }),
            Self::SearchCompleted {
                artifact_id,
                cards_added,
            } => Ok(StoredEventPayload::SearchCompleted {
                artifact_id,
                cards_added,
            }),
            Self::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Ok(StoredEventPayload::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            }),
            Self::ApprovalRecorded { task_id, approved } => {
                Ok(StoredEventPayload::ApprovalRecorded { task_id, approved })
            }
            Self::TickObserved { at } => Ok(StoredEventPayload::TickObserved { at }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event_kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredEventPayload {
    ArtifactRegistered {
        artifact_id: u64,
        title: String,
    },
    ChunkRegistered {
        chunk_id: u64,
        artifact_id: u64,
        order: u32,
        text: String,
    },
    CardCreated {
        card_id: u64,
        artifact_id: u64,
        title: String,
        body: String,
    },
    ClaimCreated {
        claim_id: u64,
        artifact_id: u64,
        text: String,
        evidence_ids: Vec<u64>,
    },
    EvidenceRecorded {
        evidence_id: u64,
        artifact_id: u64,
        claim_id: Option<u64>,
        evidence_kind: StoredEvidenceKind,
        excerpt: String,
        observed_at: u64,
    },
    TaskOpened {
        task_id: u64,
        title: String,
        priority: StoredTaskPriority,
        artifact_id: Option<u64>,
    },
    TaskStatusChanged {
        task_id: u64,
        from: StoredTaskStatus,
        to: StoredTaskStatus,
    },
    TaskCompletionRecorded {
        task_id: u64,
        status: StoredTaskStatus,
        validation_report_id: u64,
    },
    ClaimValidationUpdated {
        claim_id: u64,
        status: StoredClaimStatus,
    },
    ClaimEvidenceLinked {
        claim_id: u64,
        evidence_id: u64,
    },
    RelationCreated {
        relation_id: u64,
        source: StoredRelationEndpoint,
        kind: StoredRelationKind,
        target: StoredRelationEndpoint,
        evidence_id: Option<u64>,
        confidence_milli: u16,
    },
    MemoryCandidateCreated {
        candidate_id: u64,
        claim_id: u64,
        evidence_ids: Vec<u64>,
        confidence_milli: u16,
    },
    MemoryPromoted {
        memory_id: u64,
        candidate_id: u64,
    },
    MemoryContradicted {
        memory_id: u64,
        contradicting_candidate_id: u64,
    },
    MemoryDeprecated {
        memory_id: u64,
    },
    MemorySuperseded {
        memory_id: u64,
        by_memory_id: u64,
    },
    ValidationReportCreated {
        report_id: u64,
        task_id: Option<u64>,
        passed: bool,
        warnings: Vec<String>,
    },
    UserIntentObserved {
        task_id: u64,
        title: String,
    },
    ArtifactParsed {
        artifact_id: u64,
        chunks_added: u32,
    },
    SearchCompleted {
        artifact_id: u64,
        cards_added: u32,
    },
    HarnessRunCompleted {
        task_id: Option<u64>,
        command: String,
        exit_code: i32,
    },
    ApprovalRecorded {
        task_id: u64,
        approved: bool,
    },
    TickObserved {
        at: u64,
    },
}

impl StoredEventPayload {
    pub(crate) fn from_domain(event: &DomainEvent) -> Self {
        match event {
            DomainEvent::ArtifactRegistered { artifact_id, title } => Self::ArtifactRegistered {
                artifact_id: artifact_id.value(),
                title: title.clone(),
            },
            DomainEvent::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
                text,
            } => Self::ChunkRegistered {
                chunk_id: chunk_id.value(),
                artifact_id: artifact_id.value(),
                order: *order,
                text: text.clone(),
            },
            DomainEvent::CardCreated {
                card_id,
                artifact_id,
                title,
                body,
            } => Self::CardCreated {
                card_id: card_id.value(),
                artifact_id: artifact_id.value(),
                title: title.clone(),
                body: body.clone(),
            },
            DomainEvent::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
            } => Self::ClaimCreated {
                claim_id: claim_id.value(),
                artifact_id: artifact_id.value(),
                text: text.clone(),
                evidence_ids: evidence_ids.iter().map(|id| id.value()).collect(),
            },
            DomainEvent::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                kind,
                excerpt,
                observed_at,
            } => Self::EvidenceRecorded {
                evidence_id: evidence_id.value(),
                artifact_id: artifact_id.value(),
                claim_id: claim_id.map(|id| id.value()),
                evidence_kind: StoredEvidenceKind::from_domain(kind),
                excerpt: excerpt.clone(),
                observed_at: observed_at.value(),
            },
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => Self::TaskOpened {
                task_id: task_id.value(),
                title: title.clone(),
                priority: StoredTaskPriority::from_domain(*priority),
                artifact_id: artifact_id.map(|id| id.value()),
            },
            DomainEvent::TaskStatusChanged { task_id, from, to } => Self::TaskStatusChanged {
                task_id: task_id.value(),
                from: StoredTaskStatus::from_domain(*from),
                to: StoredTaskStatus::from_domain(*to),
            },
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => Self::TaskCompletionRecorded {
                task_id: task_id.value(),
                status: StoredTaskStatus::from_domain(*status),
                validation_report_id: validation_report_id.value(),
            },
            DomainEvent::ClaimValidationUpdated { claim_id, status } => Self::ClaimValidationUpdated {
                claim_id: claim_id.value(),
                status: StoredClaimStatus::from_domain(status),
            },
            DomainEvent::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => Self::ClaimEvidenceLinked {
                claim_id: claim_id.value(),
                evidence_id: evidence_id.value(),
            },
            DomainEvent::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => Self::RelationCreated {
                relation_id: relation_id.value(),
                source: StoredRelationEndpoint::from_domain(source),
                kind: StoredRelationKind::from_domain(kind),
                target: StoredRelationEndpoint::from_domain(target),
                evidence_id: evidence_id.map(|id| id.value()),
                confidence_milli: *confidence_milli,
            },
            DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => Self::MemoryCandidateCreated {
                candidate_id: candidate_id.value(),
                claim_id: claim_id.value(),
                evidence_ids: evidence_ids.iter().map(|evidence_id| evidence_id.value()).collect(),
                confidence_milli: *confidence_milli,
            },
            DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
            } => Self::MemoryPromoted {
                memory_id: memory_id.value(),
                candidate_id: candidate_id.value(),
            },
            DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => Self::MemoryContradicted {
                memory_id: memory_id.value(),
                contradicting_candidate_id: contradicting_candidate_id.value(),
            },
            DomainEvent::MemoryDeprecated { memory_id } => Self::MemoryDeprecated {
                memory_id: memory_id.value(),
            },
            DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => Self::MemorySuperseded {
                memory_id: memory_id.value(),
                by_memory_id: by_memory_id.value(),
            },
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Self::ValidationReportCreated {
                report_id: report_id.value(),
                task_id: task_id.map(|id| id.value()),
                passed: *passed,
                warnings: warnings.clone(),
            },
            DomainEvent::UserIntentObserved { task_id, title } => Self::UserIntentObserved {
                task_id: task_id.value(),
                title: title.clone(),
            },
            DomainEvent::ArtifactParsed {
                artifact_id,
                chunks_added,
            } => Self::ArtifactParsed {
                artifact_id: artifact_id.value(),
                chunks_added: *chunks_added,
            },
            DomainEvent::SearchCompleted {
                artifact_id,
                cards_added,
            } => Self::SearchCompleted {
                artifact_id: artifact_id.value(),
                cards_added: *cards_added,
            },
            DomainEvent::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Self::HarnessRunCompleted {
                task_id: task_id.map(|id| id.value()),
                command: command.clone(),
                exit_code: *exit_code,
            },
            DomainEvent::ApprovalRecorded { task_id, approved } => Self::ApprovalRecorded {
                task_id: task_id.value(),
                approved: *approved,
            },
            DomainEvent::TickObserved { at } => Self::TickObserved { at: at.value() },
        }
    }

    pub(crate) fn into_domain(self) -> DomainEvent {
        match self {
            Self::ArtifactRegistered { artifact_id, title } => DomainEvent::ArtifactRegistered {
                artifact_id: ArtifactId::new(artifact_id),
                title,
            },
            Self::ChunkRegistered {
                chunk_id,
                artifact_id,
                order,
                text,
            } => DomainEvent::ChunkRegistered {
                chunk_id: maestria_domain::ChunkId::new(chunk_id),
                artifact_id: ArtifactId::new(artifact_id),
                order,
                text,
            },
            Self::CardCreated {
                card_id,
                artifact_id,
                title,
                body,
            } => DomainEvent::CardCreated {
                card_id: maestria_domain::CardId::new(card_id),
                artifact_id: ArtifactId::new(artifact_id),
                title,
                body,
            },
            Self::ClaimCreated {
                claim_id,
                artifact_id,
                text,
                evidence_ids,
            } => DomainEvent::ClaimCreated {
                claim_id: maestria_domain::ClaimId::new(claim_id),
                artifact_id: ArtifactId::new(artifact_id),
                text,
                evidence_ids: evidence_ids.into_iter().map(EvidenceId::new).collect(),
            },
            Self::EvidenceRecorded {
                evidence_id,
                artifact_id,
                claim_id,
                evidence_kind,
                excerpt,
                observed_at,
            } => DomainEvent::EvidenceRecorded {
                evidence_id: maestria_domain::EvidenceId::new(evidence_id),
                artifact_id: ArtifactId::new(artifact_id),
                claim_id: claim_id.map(maestria_domain::ClaimId::new),
                kind: evidence_kind.into_domain(),
                excerpt,
                observed_at: maestria_domain::LogicalTick::new(observed_at),
            },
            Self::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => DomainEvent::TaskOpened {
                task_id: maestria_domain::TaskId::new(task_id),
                title,
                priority: priority.into_domain(),
                artifact_id: artifact_id.map(maestria_domain::ArtifactId::new),
            },
            Self::TaskStatusChanged { task_id, from, to } => DomainEvent::TaskStatusChanged {
                task_id: maestria_domain::TaskId::new(task_id),
                from: from.into_domain(),
                to: to.into_domain(),
            },
            Self::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => DomainEvent::TaskCompletionRecorded {
                task_id: TaskId::new(task_id),
                status: status.into_domain(),
                validation_report_id: ValidationReportId::new(validation_report_id),
            },
            Self::ClaimValidationUpdated { claim_id, status } => {
                DomainEvent::ClaimValidationUpdated {
                    claim_id: maestria_domain::ClaimId::new(claim_id),
                    status: status.into_domain(),
                }
            }
            Self::ClaimEvidenceLinked {
                claim_id,
                evidence_id,
            } => DomainEvent::ClaimEvidenceLinked {
                claim_id: maestria_domain::ClaimId::new(claim_id),
                evidence_id: maestria_domain::EvidenceId::new(evidence_id),
            },
            Self::RelationCreated {
                relation_id,
                source,
                kind,
                target,
                evidence_id,
                confidence_milli,
            } => DomainEvent::RelationCreated {
                relation_id: maestria_domain::RelationId::new(relation_id),
                source: source.into_domain(),
                kind: kind.into_domain(),
                target: target.into_domain(),
                evidence_id: evidence_id.map(maestria_domain::EvidenceId::new),
                confidence_milli,
            },
            Self::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                evidence_ids,
                confidence_milli,
            } => DomainEvent::MemoryCandidateCreated {
                candidate_id: maestria_domain::MemoryCandidateId::new(candidate_id),
                claim_id: maestria_domain::ClaimId::new(claim_id),
                evidence_ids: evidence_ids.into_iter().map(maestria_domain::EvidenceId::new).collect(),
                confidence_milli,
            },
            Self::MemoryPromoted {
                memory_id,
                candidate_id,
            } => DomainEvent::MemoryPromoted {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                candidate_id: maestria_domain::MemoryCandidateId::new(candidate_id),
            },
            Self::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            } => DomainEvent::MemoryContradicted {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                contradicting_candidate_id: maestria_domain::MemoryCandidateId::new(
                    contradicting_candidate_id,
                ),
            },
            Self::MemoryDeprecated { memory_id } => DomainEvent::MemoryDeprecated {
                memory_id: maestria_domain::MemoryId::new(memory_id),
            },
            Self::MemorySuperseded {
                memory_id,
                by_memory_id,
            } => DomainEvent::MemorySuperseded {
                memory_id: maestria_domain::MemoryId::new(memory_id),
                by_memory_id: maestria_domain::MemoryId::new(by_memory_id),
            },
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => DomainEvent::ValidationReportCreated {
                report_id: ValidationReportId::new(report_id),
                task_id: task_id.map(TaskId::new),
                passed,
                warnings,
            },
            Self::UserIntentObserved { task_id, title } => DomainEvent::UserIntentObserved {
                task_id: maestria_domain::TaskId::new(task_id),
                title,
            },
            Self::ArtifactParsed {
                artifact_id,
                chunks_added,
            } => DomainEvent::ArtifactParsed {
                artifact_id: ArtifactId::new(artifact_id),
                chunks_added,
            },
            Self::SearchCompleted {
                artifact_id,
                cards_added,
            } => DomainEvent::SearchCompleted {
                artifact_id: ArtifactId::new(artifact_id),
                cards_added,
            },
            Self::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => DomainEvent::HarnessRunCompleted {
                task_id: task_id.map(TaskId::new),
                command,
                exit_code,
            },
            Self::ApprovalRecorded { task_id, approved } => DomainEvent::ApprovalRecorded {
                task_id: maestria_domain::TaskId::new(task_id),
                approved,
            },
            Self::TickObserved { at } => DomainEvent::TickObserved {
                at: LogicalTick::new(at),
            },
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::ArtifactRegistered { .. } => "artifact_registered",
            Self::ChunkRegistered { .. } => "chunk_registered",
            Self::CardCreated { .. } => "card_created",
            Self::ClaimCreated { .. } => "claim_created",
            Self::EvidenceRecorded { .. } => "evidence_recorded",
            Self::TaskOpened { .. } => "task_opened",
            Self::TaskStatusChanged { .. } => "task_status_changed",
            Self::TaskCompletionRecorded { .. } => "task_completion_recorded",
            Self::ClaimValidationUpdated { .. } => "claim_validation_updated",
            Self::ClaimEvidenceLinked { .. } => "claim_evidence_linked",
            Self::RelationCreated { .. } => "relation_created",
            Self::MemoryCandidateCreated { .. } => "memory_candidate_created",
            Self::MemoryPromoted { .. } => "memory_promoted",
            Self::MemoryContradicted { .. } => "memory_contradicted",
            Self::MemoryDeprecated { .. } => "memory_deprecated",
            Self::MemorySuperseded { .. } => "memory_superseded",
            Self::ValidationReportCreated { .. } => "validation_report_created",
            Self::UserIntentObserved { .. } => "user_intent_observed",
            Self::ArtifactParsed { .. } => "artifact_parsed",
            Self::SearchCompleted { .. } => "search_completed",
            Self::HarnessRunCompleted { .. } => "harness_run_completed",
            Self::ApprovalRecorded { .. } => "approval_recorded",
            Self::TickObserved { .. } => "tick_observed",
        }
    }

    pub(crate) fn filter_artifact_id(&self) -> Option<u64> {
        match self {
            Self::ArtifactRegistered { artifact_id, .. }
            | Self::ChunkRegistered { artifact_id, .. }
            | Self::CardCreated { artifact_id, .. }
            | Self::ClaimCreated { artifact_id, .. }
            | Self::EvidenceRecorded { artifact_id, .. }
            | Self::ArtifactParsed { artifact_id, .. }
            | Self::SearchCompleted { artifact_id, .. } => Some(*artifact_id),
            Self::TaskOpened {
                artifact_id: Some(artifact_id),
                ..
            } => Some(*artifact_id),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredRelationEndpoint {
    Artifact { artifact_id: u64 },
    Claim { claim_id: u64 },
    Task { task_id: u64 },
    Memory { memory_id: u64 },
    Card { card_id: u64 },
}

impl StoredRelationEndpoint {
    pub(crate) fn from_domain(endpoint: &RelationEndpoint) -> Self {
        match endpoint {
            RelationEndpoint::Artifact(id) => Self::Artifact {
                artifact_id: id.value(),
            },
            RelationEndpoint::Claim(id) => Self::Claim {
                claim_id: id.value(),
            },
            RelationEndpoint::Task(id) => Self::Task {
                task_id: id.value(),
            },
            RelationEndpoint::Memory(id) => Self::Memory {
                memory_id: id.value(),
            },
            RelationEndpoint::Card(id) => Self::Card {
                card_id: id.value(),
            },
        }
    }

    pub(crate) fn into_domain(self) -> RelationEndpoint {
        match self {
            Self::Artifact { artifact_id } => RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(artifact_id)),
            Self::Claim { claim_id } => RelationEndpoint::Claim(maestria_domain::ClaimId::new(claim_id)),
            Self::Task { task_id } => RelationEndpoint::Task(maestria_domain::TaskId::new(task_id)),
            Self::Memory { memory_id } => RelationEndpoint::Memory(maestria_domain::MemoryId::new(memory_id)),
            Self::Card { card_id } => RelationEndpoint::Card(maestria_domain::CardId::new(card_id)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRelationKind {
    Contains,
    Defines,
    Supports,
    Contradicts,
    UsedEvidence,
    BasedOn,
    DerivedFrom,
    AppliesTo,
    RelatedTo,
}

impl StoredRelationKind {
    pub(crate) fn from_domain(kind: &maestria_domain::RelationKind) -> Self {
        match kind {
            maestria_domain::RelationKind::Contains => Self::Contains,
            maestria_domain::RelationKind::Defines => Self::Defines,
            maestria_domain::RelationKind::Supports => Self::Supports,
            maestria_domain::RelationKind::Contradicts => Self::Contradicts,
            maestria_domain::RelationKind::UsedEvidence => Self::UsedEvidence,
            maestria_domain::RelationKind::BasedOn => Self::BasedOn,
            maestria_domain::RelationKind::DerivedFrom => Self::DerivedFrom,
            maestria_domain::RelationKind::AppliesTo => Self::AppliesTo,
            maestria_domain::RelationKind::RelatedTo => Self::RelatedTo,
        }
    }

    pub(crate) fn into_domain(self) -> maestria_domain::RelationKind {
        match self {
            Self::Contains => maestria_domain::RelationKind::Contains,
            Self::Defines => maestria_domain::RelationKind::Defines,
            Self::Supports => maestria_domain::RelationKind::Supports,
            Self::Contradicts => maestria_domain::RelationKind::Contradicts,
            Self::UsedEvidence => maestria_domain::RelationKind::UsedEvidence,
            Self::BasedOn => maestria_domain::RelationKind::BasedOn,
            Self::DerivedFrom => maestria_domain::RelationKind::DerivedFrom,
            Self::AppliesTo => maestria_domain::RelationKind::AppliesTo,
            Self::RelatedTo => maestria_domain::RelationKind::RelatedTo,
        }
    }
}
